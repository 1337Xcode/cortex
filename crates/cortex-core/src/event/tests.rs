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
    r#""end_line":13}],"pattern":"def divide","command":"pytest -q"},"tool_result":{"#,
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
            pattern: Some("def divide".to_owned()),
            command: Some(ShellCommand::cut("pytest -q")),
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
