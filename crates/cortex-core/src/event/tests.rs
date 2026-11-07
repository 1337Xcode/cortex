//! The event's wire form, written out by hand, and the values it refuses.

use std::path::PathBuf;

use super::*;

/// Result type for these tests: any failure fails the test.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Every field of an event in its wire form, in declaration order, as written by hand.
const FULL_JSON: &str = concat!(
    r#"{"event_id":"01HF7YAT00NW9MKEFNZ6GTD820","ts_ms":1700000000000,"agent":"claude","#,
    r#""agent_version":"2.1.312","session_id":"session-1","parent_session_id":"parent-1","#,
    r#""cwd":"/repo/src","repo_root":"/repo","hook":"post_tool","source":"startup","#,
    r#""tool":"edit","tool_target":{"path":"calc/ops.py","ranges":[{"start_line":10,"#,
    r#""end_line":13}],"cut_after":2000,"pattern":"def divide","command":"pytest -q","#,
    r#""hits":["calc/ops.py","#,
    r#""tests/test_ops.py"],"more_paths":["calc/__init__.py"]},"tool_result":{"#,
    r#""exit_code":3,"is_success":false,"is_interrupted":false,"output_bytes":42},"#,
    r#""prompt_text":"fix divide","transcript_path":"/home/dev/t.jsonl","model":"sonnet","#,
    r#""subagent_type":"Explore","compact_trigger":"manual","end_reason":"other"}"#
);

/// The event [`FULL_JSON`] describes.
fn full_event() -> Result<Event, Box<dyn std::error::Error>> {
    Ok(Event {
        event_id: Ulid::parse("01HF7YAT00NW9MKEFNZ6GTD820")?,
        ts_ms: 1_700_000_000_000,
        agent: Agent::Claude,
        agent_version: Some(Identifier::new("2.1.312")?),
        session_id: Identifier::new("session-1")?,
        parent_session_id: Some(Identifier::new("parent-1")?),
        cwd: PathBuf::from("/repo/src"),
        repo_root: PathBuf::from("/repo"),
        hook: HookKind::PostTool,
        source: Some(StartSource::Startup),
        tool: Some(ToolKind::Edit),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new("calc/ops.py")?),
            ranges: vec![LineRange::new(10, 13)?],
            cut_after: Some(2000),
            pattern: Some("def divide".to_owned()),
            command: Some(ShellCommand::cut("pytest -q")),
            hits: vec![
                RepoPath::new("calc/ops.py")?,
                RepoPath::new("tests/test_ops.py")?,
            ],
            more_paths: vec![RepoPath::new("calc/__init__.py")?],
        }),
        tool_result: Some(ToolResult {
            exit_code: Some(3),
            is_success: false,
            is_interrupted: false,
            output_bytes: Some(42),
        }),
        prompt_text: Some("fix divide".to_owned()),
        transcript_path: Some(PathBuf::from("/home/dev/t.jsonl")),
        model: Some(Identifier::new("sonnet")?),
        subagent_type: Some(Identifier::new("Explore")?),
        compact_trigger: Some(CompactTrigger::Manual),
        end_reason: Some(Identifier::new("other")?),
        is_probe: false,
    })
}

/// An event with every field writes the wire form written by hand, and reads back from it.
#[test]
fn every_field_has_its_wire_form() -> TestResult {
    let event = full_event()?;
    assert_eq!(serde_json::to_string(&event)?, FULL_JSON);
    assert_eq!(serde_json::from_str::<Event>(FULL_JSON)?, event);
    Ok(())
}

/// Absent fields are left out of the wire form, and fields this version does not know are
/// ignored when reading, so a newer sender's event still reads.
#[test]
fn absent_fields_are_left_out_and_unknown_ones_ignored() -> TestResult {
    let minimal = concat!(
        r#"{"event_id":"01HF7YAT00NW9MKEFNZ6GTD820","ts_ms":5,"agent":"codex","#,
        r#""session_id":"s","cwd":"/r","repo_root":"/r","hook":"stop"}"#
    );
    let event: Event = serde_json::from_str(minimal)?;
    assert_eq!(event.hook, HookKind::Stop);
    assert_eq!(event.tool_target, None);
    assert_eq!(serde_json::to_string(&event)?, minimal);
    let newer = minimal.replace(r#""hook":"stop""#, r#""hook":"stop","added_later":[1,2]"#);
    assert_eq!(serde_json::from_str::<Event>(&newer)?, event);
    Ok(())
}

/// Values that break a field's rules are refused when read: a range from line 0 or ending before
/// it starts, a path outside the repository, a command over the cap, an unknown hook, a session
/// id spanning lines, and an id that is not a ULID.
#[test]
fn values_that_break_the_rules_are_refused() {
    let long_command = format!(r#""command":"{}""#, "x".repeat(MAX_COMMAND_CHARS + 1));
    let breakages = [
        (
            r#""start_line":10,"end_line":13"#,
            r#""start_line":0,"end_line":13"#,
        ),
        (
            r#""start_line":10,"end_line":13"#,
            r#""start_line":10,"end_line":9"#,
        ),
        (r#""path":"calc/ops.py""#, r#""path":"/etc/passwd""#),
        (r#""path":"calc/ops.py""#, r#""path":"../outside.py""#),
        (r#""command":"pytest -q""#, long_command.as_str()),
        (r#""hook":"post_tool""#, r#""hook":"post_tools""#),
        (
            r#""session_id":"session-1""#,
            r#""session_id":"two\nlines""#,
        ),
        (
            r#""event_id":"01HF7YAT00NW9MKEFNZ6GTD820""#,
            r#""event_id":"not-an-id""#,
        ),
    ];
    for (good, bad) in breakages {
        assert_eq!(FULL_JSON.matches(good).count(), 1, "{good}");
        let broken = FULL_JSON.replace(good, bad);
        assert!(serde_json::from_str::<Event>(&broken).is_err(), "{bad}");
    }
}

/// A command is cut after its first 1,000 characters, counted as characters, not bytes; a shorter
/// one is kept whole.
#[test]
fn commands_are_cut_by_characters() {
    let long = "é".repeat(MAX_COMMAND_CHARS + 5);
    let cut = ShellCommand::cut(&long);
    assert_eq!(cut.as_str().chars().count(), MAX_COMMAND_CHARS);
    assert_eq!(cut.as_str().len(), 2 * MAX_COMMAND_CHARS);
    assert_eq!(ShellCommand::cut("cargo test").as_str(), "cargo test");
}

/// Debug output, which may end up in a log, shows the lengths of the prompt, the search pattern
/// and the command, never their text.
#[test]
fn debug_output_withholds_prompts_patterns_and_commands() -> TestResult {
    let shown = format!("{:?}", full_event()?);
    for text in ["fix divide", "def divide", "pytest -q"] {
        assert!(!shown.contains(text), "{text} in {shown}");
    }
    for length in [
        "<10 characters withheld>",
        "<10 characters withheld>",
        "<9 characters withheld>",
    ] {
        assert!(shown.contains(length), "{length} not in {shown}");
    }
    assert!(shown.contains("calc/ops.py"), "{shown}");
    Ok(())
}

/// Two ranges overlap when they share a line, worked by hand: 3 to 5 meets 5 to 9, 1 to 9, 4 to
/// 4 and itself, either way round; it does not meet 6 to 9 or 1 to 2.
#[test]
fn ranges_overlap_when_they_share_a_line() -> Result<(), LineRangeError> {
    let lines = |start, end| LineRange::new(start, end);
    let middle = lines(3, 5)?;
    for other in [lines(5, 9)?, lines(1, 9)?, lines(4, 4)?, middle] {
        assert!(middle.overlaps(other), "{other:?}");
        assert!(other.overlaps(middle), "{other:?}");
    }
    for other in [lines(6, 9)?, lines(1, 2)?] {
        assert!(!middle.overlaps(other), "{other:?}");
        assert!(!other.overlaps(middle), "{other:?}");
    }
    Ok(())
}

/// The file an event says a tool changed: an edit's successful result names `calc/ops.py`; a failed
/// one, a read's, the moment before the edit runs, and one naming no file inside the repository
/// name none. Every editing tool edits; reading, searching and running a command do not.
#[test]
fn only_a_successful_edit_names_its_file() -> TestResult {
    let failed = full_event()?;
    assert_eq!(failed.edited_file(), None);
    let succeeded = |event: Event| Event {
        tool_result: Some(ToolResult {
            is_success: true,
            ..event.tool_result.unwrap_or(ToolResult {
                exit_code: None,
                is_success: true,
                is_interrupted: false,
                output_bytes: None,
            })
        }),
        ..event
    };
    let edit = succeeded(full_event()?);
    assert_eq!(
        edit.edited_file().map(RepoPath::as_str),
        Some("calc/ops.py")
    );
    let read = Event {
        tool: Some(ToolKind::Read),
        ..edit.clone()
    };
    assert_eq!(read.edited_file(), None);
    let before = Event {
        hook: HookKind::PreTool,
        ..edit.clone()
    };
    assert_eq!(before.edited_file(), None);
    let outside = Event {
        tool_target: Some(ToolTarget::default()),
        ..edit
    };
    assert_eq!(outside.edited_file(), None);
    let edits: Vec<&str> = ToolKind::ALL
        .iter()
        .filter(|tool| tool.is_edit())
        .map(|tool| tool.name())
        .collect();
    assert_eq!(edits, ["edit", "write", "multi_edit", "apply_patch"]);
    Ok(())
}

/// The file a read is about to read: the moment before a read names `calc/ops.py`; the read's
/// result, an edit about to run and a read naming no file inside the repository name none.
/// A prompt asks for code unless it is a slash command, a shell escape (each after any spaces) or
/// fewer than four characters other than spaces; a prompt that starts with an absolute path is
/// no slash command. Any other event asks for none.
#[test]
fn a_prompt_asks_for_code_unless_it_is_a_command_or_a_word() -> TestResult {
    let asking = |text: &str| -> Result<Option<String>, Box<dyn std::error::Error>> {
        let event = Event {
            hook: HookKind::Prompt,
            prompt_text: Some(text.to_owned()),
            ..full_event()?
        };
        Ok(event.prompt_for_work().map(str::to_owned))
    };
    for text in [
        "fix the login bug",
        "a.py",
        "  x y z w  ",
        "why?",
        "/usr/lib/app/main.py crashes on start",
    ] {
        assert_eq!(asking(text)?.as_deref(), Some(text), "{text:?}");
    }
    for text in [
        "/review src/a.py",
        "  /commit",
        "!ls -la",
        "ok",
        " y e s ",
        "go",
        "",
    ] {
        assert_eq!(asking(text)?, None, "{text:?}");
    }
    let stop = Event {
        hook: HookKind::Stop,
        prompt_text: Some("fix the login bug".to_owned()),
        ..full_event()?
    };
    assert_eq!(stop.prompt_for_work(), None);
    Ok(())
}

#[test]
fn only_the_moment_before_a_read_names_its_file() -> TestResult {
    let before = Event {
        hook: HookKind::PreTool,
        tool: Some(ToolKind::Read),
        ..full_event()?
    };
    assert_eq!(
        before.file_to_read().map(RepoPath::as_str),
        Some("calc/ops.py")
    );
    let after = Event {
        hook: HookKind::PostTool,
        ..before.clone()
    };
    assert_eq!(after.file_to_read(), None);
    let edit = Event {
        tool: Some(ToolKind::Edit),
        ..before.clone()
    };
    assert_eq!(edit.file_to_read(), None);
    let outside = Event {
        tool_target: Some(ToolTarget::default()),
        ..before
    };
    assert_eq!(outside.file_to_read(), None);
    Ok(())
}
