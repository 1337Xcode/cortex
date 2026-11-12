//! Every captured Claude Code payload normalized, against events written out by hand from the
//! fixtures; the newest version's payloads against the same events' shapes; the payloads that make
//! no event or no usable one; and inputs that must never panic.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::error::Error;
use std::path::{Path, PathBuf};

use cortex_core::ipc::{LENGTH_PREFIX_BYTES, MAX_FRAME_BYTES, Request, RequestBody, encode};
use cortex_core::{
    Agent, CompactTrigger, ContentHash, Event, FailedOutput, HookKind, LineRange,
    MAX_FAILED_OUTPUT_BYTES, RepoPath, ShellCommand, StartSource, ToolKind, ToolResult, ToolTarget,
};
use cortex_hook::{Located, NormalizeError, adapter};
use proptest::prelude::*;

use common::TestResult;
use common::fixture::{
    assert_shape, event_name, fixture_names, id, json_value, locate, native, read_fixture, root,
    stamp,
};

/// `calc/ops.py` as the captured edit found it, as its `originalFile` holds it: `divide`, on
/// lines 12 and 13, divides by floor.
const OPS_BEFORE_EDIT: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    \
                               return a + b\n\n\ndef mul(a, b):\n    return a * b\n\n\n\
                               def divide(a, b):\n    return a // b\n";

/// The main session of most fixtures.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The session `/clear` opened.
const S6: &str = "00000000-0000-4000-8000-000000000006";

/// The session of the range read, the passing test and the PowerShell fixtures.
const S7: &str = "00000000-0000-4000-8000-000000000007";

/// The Claude Code version whose payloads the table of events written by hand covers.
const TABLED: &str = "2.1.312";

/// The fixtures only the newest version has: Grep runs given a folder, one file, context lines and
/// count mode, captured after the table's version, whose hits are read off them by hand; and a
/// turn that ended on an API error.
const NEWEST_ONLY: [&str; 7] = [
    "post-tool-use-grep-context.json",
    "post-tool-use-grep-count.json",
    "post-tool-use-grep-folder-content.json",
    "post-tool-use-grep-folder.json",
    "post-tool-use-grep-one-file-count.json",
    "post-tool-use-grep-one-file.json",
    "stop-failure-model-not-found.json",
];

/// The folder of the fixtures of Claude Code `version`.
fn fixtures_of(version: &str) -> PathBuf {
    common::fixture::fixtures_of("claude", version)
}

/// The folder of the fixtures the table covers.
fn fixtures() -> PathBuf {
    fixtures_of(TABLED)
}

/// The fixture `name` the table covers, its paths in this platform's form.
fn payload(name: &str) -> TestResult<Vec<u8>> {
    payload_of(TABLED, name)
}

/// The fixture `name` of Claude Code `version`, its paths in this platform's form.
fn payload_of(version: &str, name: &str) -> TestResult<Vec<u8>> {
    read_fixture("claude", version, name)
}

/// The event of `hook` from session `session`, with only what every event has.
fn base(hook: HookKind, session: &str) -> TestResult<Event> {
    Ok(Event {
        event_id: stamp().event_id,
        ts_ms: stamp().ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: id(session)?,
        parent_session_id: None,
        cwd: root(),
        repo_root: root(),
        worktree_root: None,
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: Some(PathBuf::from(native(&format!(
            r"C:\Users\dev\.claude\projects\C--repo\{session}.jsonl"
        )))),
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// A tool event of `hook` for `tool` from `session`.
fn tool(
    hook: HookKind,
    session: &str,
    (kind, target, result): (ToolKind, Option<ToolTarget>, Option<ToolResult>),
) -> TestResult<Event> {
    Ok(Event {
        tool: Some(kind),
        tool_target: target,
        tool_result: result,
        ..base(hook, session)?
    })
}

/// The event from the subagent `agent` of session S1, whose own transcript the payload does not
/// give.
fn subagent(event: Event, agent: &str) -> TestResult<Event> {
    Ok(Event {
        session_id: id(&format!("{S1}/{agent}"))?,
        parent_session_id: Some(id(S1)?),
        transcript_path: None,
        ..event
    })
}

/// A target at `path` covering `ranges`.
fn file(path: &str, ranges: &[(u32, u32)]) -> TestResult<Option<ToolTarget>> {
    Ok(Some(ToolTarget {
        path: Some(RepoPath::new(path)?),
        ranges: ranges
            .iter()
            .map(|(start, end)| LineRange::new(*start, *end))
            .collect::<Result<_, _>>()?,
        ..ToolTarget::default()
    }))
}

/// A search for `pattern` in `path`, or where the agent was when there is no path.
fn search(pattern: &str, path: Option<RepoPath>) -> Option<ToolTarget> {
    Some(ToolTarget {
        path,
        pattern: Some(pattern.to_owned()),
        ..ToolTarget::default()
    })
}

/// `search` with the files it found, in the order listed.
fn found(search: Option<ToolTarget>, hits: &[&str]) -> TestResult<Option<ToolTarget>> {
    let mut target = search.ok_or("a search")?;
    target.hits = hits
        .iter()
        .map(|hit| RepoPath::new(hit))
        .collect::<Result<_, _>>()?;
    Ok(Some(target))
}

/// A shell command.
fn command(text: &str) -> Option<ToolTarget> {
    Some(ToolTarget {
        command: Some(ShellCommand::cut(text)),
        ..ToolTarget::default()
    })
}

/// A shell command that failed, with what it printed: the fixture `name`'s message after its
/// first line, `Exit code N`, which is Claude Code's own and not the command's.
fn command_failed(text: &str, name: &str) -> TestResult<Option<ToolTarget>> {
    let payload: serde_json::Value = serde_json::from_slice(&payload(name)?)?;
    let message = payload["error"].as_str().ok_or("no message")?;
    let (first, printed) = message.split_once('\n').ok_or("one line")?;
    assert!(first.starts_with("Exit code "), "{first}");
    Ok(Some(ToolTarget {
        command: Some(ShellCommand::cut(text)),
        failed_output: FailedOutput::tail_of(printed),
        ..ToolTarget::default()
    }))
}

/// A tool that succeeded, with the exit code and printed size reported.
fn ok(exit_code: Option<i32>, output_bytes: Option<u64>) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: true,
        is_interrupted: false,
        output_bytes,
        counts: None,
        failing: None,
    })
}

/// A tool that failed, with the exit code and message size reported.
fn failed(exit_code: Option<i32>, output_bytes: u64) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: false,
        is_interrupted: false,
        output_bytes: Some(output_bytes),
        counts: None,
        failing: None,
    })
}

/// Every fixture with the event it makes, read off the fixture by hand: the paths, the lines, the
/// commands and patterns, the sizes of what shells printed, the exit codes.
#[allow(clippy::too_many_lines, reason = "one entry per captured payload")]
fn expected() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    use HookKind::{PostTool, PreTool};
    let explore = |event: Event| -> TestResult<Event> {
        subagent(
            Event {
                subagent_type: Some(id("Explore")?),
                ..event
            },
            "fixtureagent0001",
        )
    };
    let agent_transcript = |agent: &str| {
        Some(PathBuf::from(native(&format!(
            r"C:\Users\dev\.claude\projects\C--repo\{S1}\subagents\agent-{agent}.jsonl"
        ))))
    };
    let ops = "calc/ops.py";
    // The Explore subagent searched with `path` set to the repository's root folder; the other
    // searches name no path, and searched the working folder, which is the root too.
    let everywhere = search(r"\badd\b", Some(RepoPath::root()));
    let here = || Some(RepoPath::root());
    Ok(vec![
        ("post-compact-manual.json", None),
        (
            "post-tool-use-agent.json",
            Some(tool(PostTool, S1, (ToolKind::Other, None, ok(None, None)))?),
        ),
        (
            "post-tool-use-bash-test-passing.json",
            Some(tool(
                PostTool,
                S7,
                (
                    ToolKind::Shell,
                    command("python -m unittest tests.test_ops -q"),
                    ok(Some(0), Some(99)),
                ),
            )?),
        ),
        (
            "post-tool-use-bash.json",
            Some(tool(
                PostTool,
                S1,
                (
                    ToolKind::Shell,
                    command("python -m unittest -q 2>&1 | tail -20"),
                    ok(Some(0), Some(523)),
                ),
            )?),
        ),
        (
            "post-tool-use-edit.json",
            Some(tool(
                PostTool,
                S1,
                (
                    ToolKind::Edit,
                    // The captured edit replaced line 13, and the file before it hashes as
                    // its `originalFile`.
                    file(ops, &[(13, 13)])?.map(|target| ToolTarget {
                        replaced: target.ranges.clone(),
                        before_hash: Some(ContentHash::of(OPS_BEFORE_EDIT.as_bytes())),
                        ..target
                    }),
                    ok(None, None),
                ),
            )?),
        ),
        (
            "post-tool-use-failure-bash-test.json",
            Some(tool(
                PostTool,
                S7,
                (
                    ToolKind::Shell,
                    command_failed(
                        "python -m unittest tests.test_fail -q",
                        "post-tool-use-failure-bash-test.json",
                    )?,
                    failed(Some(1), 537),
                ),
            )?),
        ),
        (
            "post-tool-use-failure-bash.json",
            Some(tool(
                PostTool,
                S1,
                (
                    ToolKind::Shell,
                    command(r#"python -c "import sys; sys.exit(3)""#),
                    failed(Some(3), 11),
                ),
            )?),
        ),
        (
            "post-tool-use-failure-powershell.json",
            Some(tool(
                PostTool,
                S7,
                (
                    ToolKind::Shell,
                    command_failed(
                        "python -m unittest tests.test_fail -q",
                        "post-tool-use-failure-powershell.json",
                    )?,
                    failed(Some(1), 537),
                ),
            )?),
        ),
        (
            "post-tool-use-glob.json",
            Some(tool(
                PostTool,
                S1,
                (
                    ToolKind::Glob,
                    found(
                        search("**/*.py", here()),
                        &[
                            "calc/ops.py",
                            "calc/__init__.py",
                            "tests/__init__.py",
                            "main.py",
                            "tests/test_ops.py",
                        ],
                    )?,
                    ok(None, None),
                ),
            )?),
        ),
        (
            "post-tool-use-grep-subagent.json",
            Some(explore(tool(
                PostTool,
                S1,
                (
                    ToolKind::Grep,
                    found(
                        everywhere.clone(),
                        &["main.py", "tests/test_ops.py", "calc/ops.py"],
                    )?,
                    ok(None, None),
                ),
            )?)?),
        ),
        (
            "post-tool-use-grep.json",
            Some(tool(
                PostTool,
                S1,
                (
                    ToolKind::Grep,
                    found(search("def divide", here()), &["calc/ops.py"])?,
                    ok(None, None),
                ),
            )?),
        ),
        (
            "post-tool-use-powershell.json",
            Some(tool(
                PostTool,
                S7,
                (
                    ToolKind::Shell,
                    command("Get-ChildItem calc -Name"),
                    ok(Some(0), Some(19)),
                ),
            )?),
        ),
        (
            "post-tool-use-read-range.json",
            Some(tool(
                PostTool,
                S7,
                (ToolKind::Read, file(ops, &[(2, 5)])?, ok(None, None)),
            )?),
        ),
        (
            "post-tool-use-read.json",
            Some(tool(
                PostTool,
                S1,
                (ToolKind::Read, file(ops, &[])?, ok(None, None)),
            )?),
        ),
        (
            "post-tool-use-write.json",
            Some(tool(
                PostTool,
                S1,
                (ToolKind::Write, file("NOTES.md", &[])?, ok(None, None)),
            )?),
        ),
        (
            "pre-compact-manual.json",
            Some(Event {
                compact_trigger: Some(CompactTrigger::Manual),
                ..base(HookKind::Compact, S1)?
            }),
        ),
        (
            "pre-tool-use-agent.json",
            Some(tool(PreTool, S1, (ToolKind::Other, None, None))?),
        ),
        (
            "pre-tool-use-bash.json",
            Some(tool(
                PreTool,
                S1,
                (
                    ToolKind::Shell,
                    command("python -m unittest -q 2>&1 | tail -20"),
                    None,
                ),
            )?),
        ),
        (
            "pre-tool-use-edit.json",
            Some(tool(PreTool, S1, (ToolKind::Edit, file(ops, &[])?, None))?),
        ),
        (
            "pre-tool-use-glob.json",
            Some(tool(
                PreTool,
                S1,
                (ToolKind::Glob, search("**/*.py", here()), None),
            )?),
        ),
        (
            "pre-tool-use-grep-subagent.json",
            Some(explore(tool(
                PreTool,
                S1,
                (ToolKind::Grep, everywhere, None),
            )?)?),
        ),
        (
            "pre-tool-use-grep.json",
            Some(tool(
                PreTool,
                S1,
                (ToolKind::Grep, search("def divide", here()), None),
            )?),
        ),
        (
            "pre-tool-use-powershell.json",
            Some(tool(
                PreTool,
                S7,
                (ToolKind::Shell, command("Get-ChildItem calc -Name"), None),
            )?),
        ),
        (
            "pre-tool-use-read-range.json",
            Some(tool(
                PreTool,
                S7,
                (ToolKind::Read, file(ops, &[(2, 5)])?, None),
            )?),
        ),
        (
            "pre-tool-use-read.json",
            Some(tool(PreTool, S1, (ToolKind::Read, file(ops, &[])?, None))?),
        ),
        (
            "pre-tool-use-write.json",
            Some(tool(
                PreTool,
                S1,
                (ToolKind::Write, file("NOTES.md", &[])?, None),
            )?),
        ),
        (
            "session-end-clear.json",
            Some(Event {
                end_reason: Some(id("clear")?),
                ..base(HookKind::SessionEnd, S1)?
            }),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("other")?),
                ..base(HookKind::SessionEnd, S1)?
            }),
        ),
        (
            "session-start-clear.json",
            Some(Event {
                source: Some(StartSource::Clear),
                ..base(HookKind::SessionStart, S6)?
            }),
        ),
        (
            "session-start-compact.json",
            Some(Event {
                source: Some(StartSource::Compact),
                model: Some(id("claude-sonnet-5-5")?),
                ..base(HookKind::SessionStart, S1)?
            }),
        ),
        (
            "session-start-resume.json",
            Some(Event {
                source: Some(StartSource::Resume),
                ..base(HookKind::SessionStart, S1)?
            }),
        ),
        (
            "session-start-startup.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..base(HookKind::SessionStart, S1)?
            }),
        ),
        ("stop.json", Some(base(HookKind::Stop, S1)?)),
        (
            "subagent-start.json",
            Some(explore(base(HookKind::SubagentStart, S1)?)?),
        ),
        (
            "subagent-stop-internal.json",
            Some(Event {
                transcript_path: agent_transcript("fixtureagent0002"),
                ..subagent(base(HookKind::SubagentStop, S1)?, "fixtureagent0002")?
            }),
        ),
        (
            "subagent-stop.json",
            Some(Event {
                transcript_path: agent_transcript("fixtureagent0001"),
                ..explore(base(HookKind::SubagentStop, S1)?)?
            }),
        ),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(prompt_of("user-prompt-submit.json")?),
                ..base(HookKind::Prompt, S1)?
            }),
        ),
    ])
}

/// The prompt a fixture carries, as written in it.
fn prompt_of(name: &str) -> TestResult<String> {
    let value: serde_json::Value = serde_json::from_slice(&payload(name)?)?;
    Ok(value
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .ok_or("no prompt")?
        .to_owned())
}

/// Each fixture makes the event written for it, and the table names every fixture there is.
#[test]
fn every_fixture_normalizes_as_written() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let table = expected()?;
    for (name, expected) in &table {
        let payload = payload(name)?;
        let found = claude.normalize(&event_name(&payload)?, &payload, &locate, stamp())?;
        assert_eq!(&found, expected, "{name}");
    }
    let mut named: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    named.sort_unstable();
    assert_eq!(named, fixture_names(&fixtures())?);
    Ok(())
}

/// A turn that ended on an API error (Claude Code 2.1.318's `StopFailure`, captured with an unknown
/// model) is a stop that names the error, so that its task ends unfinished; nothing is asked of the
/// daemon for it, since Claude Code reads no output.
#[test]
fn a_turn_ended_on_an_error_is_a_stop_naming_it() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let payload = payload_of("2.1.318", "stop-failure-model-not-found.json")?;
    let event = claude
        .normalize("StopFailure", &payload, &locate, stamp())?
        .ok_or("no event")?;
    assert_eq!(event.hook, HookKind::Stop);
    assert_eq!(
        event
            .end_reason
            .as_ref()
            .map(cortex_core::Identifier::as_str),
        Some("model_not_found")
    );
    let entry = claude
        .hooks()
        .iter()
        .find(|entry| entry.event_name == "StopFailure")
        .ok_or("no StopFailure hook")?;
    assert!(!entry.gives_context);
    Ok(())
}

/// A later Claude Code adds fields: 2.1.318 added `background_tasks` to a stop and a
/// `description` to a Bash call, and Cortex needed no change. Every newest payload, with a field
/// Cortex does not know added at its top and inside every object it holds, makes the same event as
/// the payload as captured, so an update that adds fields changes nothing Cortex does.
#[test]
fn fields_a_later_claude_code_adds_change_nothing() -> TestResult {
    /// Adds an unknown key to `value` and to every object inside it.
    fn widen(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                for inner in fields.values_mut() {
                    widen(inner);
                }
                fields.insert(
                    "cortex_test_unknown".to_owned(),
                    serde_json::json!({"added": [1, "two", null]}),
                );
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(widen),
            _ => {}
        }
    }
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let newest = claude.versions().ok_or("no versions")?.newest_tested;
    for name in fixture_names(&fixtures_of(newest))? {
        let payload = payload_of(newest, &name)?;
        let event = event_name(&payload)?;
        let as_captured = claude.normalize(&event, &payload, &locate, stamp())?;
        let mut value: serde_json::Value = serde_json::from_slice(&payload)?;
        widen(&mut value);
        let widened = claude.normalize(&event, &serde_json::to_vec(&value)?, &locate, stamp())?;
        assert_eq!(widened, as_captured, "{name}");
    }
    Ok(())
}

/// The newest version's payloads, captured from the same steps as the table's, make the same
/// events as the table's in all but their values: the same fields filled, the hook among them, and
/// no event where the table's make none. The lines an edit replaced are left out of the
/// comparison: the captured edits differ there, the newest only adding a line.
#[test]
fn the_newest_payloads_normalize_like_the_tabled_ones() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let newest = claude.versions().ok_or("no versions")?.newest_tested;
    assert_ne!(newest, TABLED);
    for name in fixture_names(&fixtures())? {
        let tabled = payload(&name)?;
        let current = payload_of(newest, &name)?;
        let older = claude.normalize(&event_name(&tabled)?, &tabled, &locate, stamp())?;
        let newer = claude.normalize(&event_name(&current)?, &current, &locate, stamp())?;
        let without_replaced = |shape: Vec<String>| -> Vec<String> {
            shape
                .into_iter()
                .filter(|path| !path.starts_with("/tool_target/replaced"))
                .collect()
        };
        assert_eq!(
            without_replaced(shape(newer.as_ref())?),
            without_replaced(shape(older.as_ref())?),
            "{name}"
        );
    }
    Ok(())
}

/// What `event` is made of, without its values: the path of each field it fills, sorted, a list
/// named with `[]` after it when it holds anything.
fn shape(event: Option<&Event>) -> TestResult<Vec<String>> {
    let mut filled = Vec::new();
    if let Some(event) = event {
        fill(&serde_json::to_value(event)?, "", &mut filled);
    }
    filled.sort();
    Ok(filled)
}

/// Adds to `filled` the path of each field `value`, at `prefix`, fills, and those inside them.
fn fill(value: &serde_json::Value, prefix: &str, filled: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, inner) in fields.iter().filter(|(_, inner)| !inner.is_null()) {
                let path = format!("{prefix}/{key}");
                fill(inner, &path, filled);
                filled.push(path);
            }
        }
        serde_json::Value::Array(items) if !items.is_empty() => {
            filled.push(format!("{prefix}[]"));
        }
        _ => {}
    }
}

/// The registered hooks are the eleven the decisions name, with their matchers and timeouts:
/// `StopFailure` beside `Stop`, for a turn that ends on an API error.
#[test]
fn the_hooks_are_the_ones_cortex_registers() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let hooks: Vec<(&str, Option<&str>, u128)> = claude
        .hooks()
        .iter()
        .map(|entry| {
            (
                entry.event_name,
                entry.matcher,
                entry.hard_timeout.as_millis(),
            )
        })
        .collect();
    let results = Some("Read|Grep|Glob|Edit|Write|MultiEdit|Bash|PowerShell");
    assert_eq!(
        hooks,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            ("PreToolUse", Some("Read|Bash|PowerShell"), 300),
            ("PostToolUse", results, 500),
            ("PostToolUseFailure", results, 500),
            ("PreCompact", None, 300),
            ("Stop", None, 300),
            ("StopFailure", None, 300),
            ("SubagentStart", None, 300),
            ("SubagentStop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    Ok(())
}

/// A payload outside any repository Cortex serves, an event Cortex does not register, and a
/// payload with no working folder make no event; one with no session is refused, as is text that
/// is not JSON. A byte-order mark in front is read past.
#[test]
fn payloads_that_make_no_event() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let stop = payload("stop.json")?;
    let nowhere = |_: &Path| None;
    assert_eq!(claude.normalize("Stop", &stop, &nowhere, stamp())?, None);
    assert_eq!(
        claude.normalize("Notification", &stop, &locate, stamp())?,
        None
    );
    assert_eq!(
        claude.normalize("Stop", br#"{"session_id":"s"}"#, &locate, stamp())?,
        None
    );
    let no_session = claude.normalize("Stop", br#"{"cwd":"/repo"}"#, &locate, stamp());
    assert!(
        matches!(no_session, Err(NormalizeError::NoSession)),
        "{no_session:?}"
    );
    let not_json = claude.normalize("Stop", b"{{{", &locate, stamp());
    assert!(
        matches!(not_json, Err(NormalizeError::Json(_))),
        "{not_json:?}"
    );
    let mut with_mark = b"\xEF\xBB\xBF".to_vec();
    with_mark.extend_from_slice(&stop);
    assert_eq!(
        claude.normalize("Stop", &with_mark, &locate, stamp())?,
        Some(base(HookKind::Stop, S1)?)
    );
    Ok(())
}

/// A prompt past 128 KiB is cut at a character boundary, so the event still fits a frame; a model
/// name that is no name and an empty subagent id are left out, and the event kept; a field that
/// belongs to another hook is not carried.
#[test]
fn long_prompts_are_cut_and_bad_names_left_out() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    // 131,071 one-byte letters, then the two bytes of `é`: byte 131,072 falls inside `é`, so the
    // cut moves back to the letters alone.
    let prompt = format!("{}é tail", "a".repeat(131_071));
    let payload = serde_json::json!({
        "session_id": S1,
        "cwd": native(r"C:\repo"),
        "prompt": prompt,
        "model": "two\nlines",
        "agent_id": "",
        "agent_type": "",
        "source": "startup",
    });
    let bytes = serde_json::to_vec(&payload)?;
    let event = claude
        .normalize("UserPromptSubmit", &bytes, &locate, stamp())?
        .ok_or("no event")?;
    assert_eq!(event.prompt_text, Some("a".repeat(131_071)));
    assert_eq!(event.model, None);
    assert_eq!(event.source, None);
    assert_eq!(event.session_id, id(S1)?);
    assert_eq!(event.parent_session_id, None);
    assert_eq!(event.subagent_type, None);
    let start = claude
        .normalize("SessionStart", &bytes, &locate, stamp())?
        .ok_or("no event")?;
    assert_eq!(start.source, Some(StartSource::Startup));
    assert_eq!(start.prompt_text, None);
    Ok(())
}

/// The newest version Cortex knows has its fixtures, one for each the table covers and the Grep
/// runs only it has, and no other; and the oldest it works with is no newer.
#[test]
fn the_known_versions_hold() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let versions = claude.versions().ok_or("no versions")?;
    let mut expected = fixture_names(&fixtures())?;
    expected.extend(NEWEST_ONLY.iter().map(|name| (*name).to_owned()));
    expected.sort_unstable();
    assert_eq!(
        fixture_names(&fixtures_of(versions.newest_tested))?,
        expected
    );
    let oldest = cortex_core::ipc::CortexVersion::parse(versions.oldest)?;
    let newest = cortex_core::ipc::CortexVersion::parse(versions.newest_tested)?;
    assert!(oldest <= newest);
    Ok(())
}

/// A payload marked `cortex_probe: true`, as `cortex doctor` sends, makes the same event with the
/// probe flag set; a marker of another type reads as absent, and no marker as the agent's own.
#[test]
fn a_probe_is_marked() -> TestResult {
    let marked = |marker: serde_json::Value| {
        normalized("stop.json", "Stop", &locate, |payload| {
            payload["cortex_probe"] = marker;
        })
    };
    let probe = Event {
        is_probe: true,
        ..base(HookKind::Stop, S1)?
    };
    assert_eq!(marked(serde_json::json!(true))?, Some(probe));
    assert_eq!(
        marked(serde_json::json!("yes"))?,
        Some(base(HookKind::Stop, S1)?)
    );
    assert_eq!(
        marked(serde_json::json!(false))?,
        Some(base(HookKind::Stop, S1)?)
    );
    Ok(())
}

/// The fixture `name` with `edit` applied to its JSON, normalized as event `event_name` with the
/// repository found by `locate`: the event, or `None` when the payload makes none.
fn normalized(
    name: &str,
    event_name: &str,
    locate: &dyn Fn(&Path) -> Option<Located>,
    edit: impl FnOnce(&mut serde_json::Value),
) -> TestResult<Option<Event>> {
    let mut value: serde_json::Value = serde_json::from_slice(&payload(name)?)?;
    edit(&mut value);
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    Ok(claude.normalize(event_name, &serde_json::to_vec(&value)?, locate, stamp())?)
}

/// As [`normalized`], for a payload that makes an event.
fn edited(
    name: &str,
    event_name: &str,
    locate: &dyn Fn(&Path) -> Option<Located>,
    edit: impl FnOnce(&mut serde_json::Value),
) -> TestResult<Event> {
    Ok(normalized(name, event_name, locate, edit)?.ok_or("no event")?)
}

/// A tool's path is in the repository under the root as the agent spells it (here through a link,
/// `C:\repo-link`) and under the root as it is on disk (`C:\repo`), a relative path under the
/// agent's working folder; a tool aimed at a path under neither makes no event at all, nor does one
/// whose path climbs with `..`, which is never followed. A search that names no path searched the
/// working folder, here `calc`.
#[test]
fn tool_paths_under_either_spelling_of_the_root() -> TestResult {
    let linked = |_: &Path| {
        Some(Located {
            cwd: root().join("calc"),
            root: root(),
            agent_root: PathBuf::from(native(r"C:\repo-link")),
            main_root: None,
        })
    };
    let cwd = native(r"C:\repo-link\calc");
    let read = |file: &str| -> TestResult<Option<Event>> {
        let (cwd, file) = (cwd.clone(), file.to_owned());
        normalized(
            "pre-tool-use-read.json",
            "PreToolUse",
            &linked,
            move |value| {
                value["cwd"] = cwd.into();
                value["tool_input"]["file_path"] = file.into();
            },
        )
    };
    let target = |event: Option<Event>| event.and_then(|event| event.tool_target);
    let ops = file("calc/ops.py", &[])?;
    assert_eq!(target(read(&native(r"C:\repo-link\calc\ops.py"))?), ops);
    assert_eq!(target(read(&native(r"C:\repo\calc\ops.py"))?), ops);
    assert_eq!(target(read("ops.py")?), ops);
    assert_eq!(read(&native(r"C:\elsewhere\ops.py"))?, None);
    assert_eq!(read(&native(r"C:\repo-link\calc\..\ops.py"))?, None);
    if cfg!(windows) {
        assert_eq!(target(read(r"\\?\C:\repo\calc\ops.py")?), ops);
    }
    let search = |path: Option<&str>| -> TestResult<Option<Event>> {
        let (cwd, path) = (cwd.clone(), path.map(str::to_owned));
        normalized(
            "pre-tool-use-grep.json",
            "PreToolUse",
            &linked,
            move |value| {
                value["cwd"] = cwd.into();
                if let Some(path) = path {
                    value["tool_input"]["path"] = path.into();
                }
            },
        )
    };
    assert_eq!(
        target(search(None)?).and_then(|target| target.path),
        Some(RepoPath::new("calc")?)
    );
    assert_eq!(search(Some(".."))?, None);
    assert_eq!(search(Some(&native(r"C:\elsewhere")))?, None);
    Ok(())
}

/// The ranges an edit records for the hunks `patch`, as Claude Code writes them after an edit.
fn edit_ranges(patch: serde_json::Value) -> TestResult<Vec<(u32, u32)>> {
    let event = edited("post-tool-use-edit.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["structuredPatch"] = patch;
    })?;
    Ok(event
        .tool_target
        .map(|target| target.ranges)
        .unwrap_or_default()
        .into_iter()
        .map(|range| (range.start_line(), range.end_line()))
        .collect())
}

/// The lines an edit replaced, as first and last line numbered before it, and the hash of the file
/// before it.
type Replaced = (Vec<(u32, u32)>, Option<ContentHash>);

/// The lines an edit records as replaced for the hunks `patch`, numbered before the edit, and
/// the hash it records of the file before it.
fn edit_replaced(patch: serde_json::Value) -> TestResult<Replaced> {
    let event = edited("post-tool-use-edit.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["structuredPatch"] = patch;
    })?;
    let target = event.tool_target.unwrap_or_default();
    let replaced = target
        .replaced
        .into_iter()
        .map(|range| (range.start_line(), range.end_line()))
        .collect();
    Ok((replaced, target.before_hash))
}

/// An edit records the lines it replaced, numbered in the file before it: its hunk's first line
/// before the edit (`oldStart`), counted on over the unchanged and removed lines, never over the
/// added ones. Worked by hand:
/// - from 10, `   -+`: 10, 11, 12 unchanged, 13 removed: 13..13 (the captured edit);
/// - from 5, `   --   `: 8 and 9 removed: 8..9, where the lines changed after the edit name the
///   line before the gap, 7;
/// - from 1, `+  -+`: 1 added, which is not in the file before; 1 and 2 unchanged, 3 removed:
///   3..3;
/// - from 4, `-+-`: 4 removed, a line added, 5 removed: 4..5;
/// - from 1, `++   `: only added lines replace nothing: none;
/// - two hunks, one only adding, give the other's lines: 41..41;
/// - a hunk without its lines, or without its first line before the edit, gives none;
/// - 17 hunks are more than an event holds: none.
///
/// The file before the captured edit hashes as its `originalFile`; one with none (a file the
/// write created) has no hash.
#[test]
fn edits_record_the_lines_they_replaced_and_the_file_before() -> TestResult {
    let one = |start, marks| -> TestResult<Vec<(u32, u32)>> {
        Ok(edit_replaced(serde_json::json!([hunk(start, marks)]))?.0)
    };
    assert_eq!(one(10, "   -+")?, [(13, 13)]);
    assert_eq!(one(5, "   --   ")?, [(8, 9)]);
    assert_eq!(one(1, "+  -+")?, [(3, 3)]);
    assert_eq!(one(4, "-+-")?, [(4, 5)]);
    assert_eq!(one(1, "++   ")?, []);
    let two = serde_json::json!([hunk(3, " + "), hunk(40, " -+ ")]);
    assert_eq!(edit_replaced(two)?.0, [(41, 41)]);
    let bare =
        serde_json::json!([{ "oldStart": 20, "oldLines": 3, "newStart": 20, "newLines": 3 }]);
    assert_eq!(edit_replaced(bare)?.0, []);
    let unplaced = serde_json::json!([{ "newStart": 20, "newLines": 1, "lines": ["-a", "+b"] }]);
    assert_eq!(edit_replaced(unplaced)?.0, []);
    let many: Vec<serde_json::Value> = (0..17).map(|at| hunk(at * 10 + 1, "-+")).collect();
    assert_eq!(edit_replaced(many.into())?.0, []);
    let (_, before) = edit_replaced(serde_json::json!([hunk(10, "   -+")]))?;
    assert_eq!(before, Some(ContentHash::of(OPS_BEFORE_EDIT.as_bytes())));
    let created = edited("post-tool-use-edit.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["originalFile"] = serde_json::Value::Null;
    })?;
    assert_eq!(
        created.tool_target.and_then(|target| target.before_hash),
        None
    );
    Ok(())
}

/// One hunk from line `new_start`, its diff lines marked by `marks` (one character each).
fn hunk(new_start: u32, marks: &str) -> serde_json::Value {
    let lines: Vec<String> = marks.chars().map(|mark| format!("{mark}code")).collect();
    let new_lines = marks.chars().filter(|mark| *mark != '-').count();
    serde_json::json!({
        "oldStart": new_start, "oldLines": 0, "newStart": new_start, "newLines": new_lines,
        "lines": lines,
    })
}

/// An edit records the lines it changed, without the unchanged lines around them. Worked by hand,
/// counting lines in the file after the edit:
/// - from 10, `   -+`: 10, 11, 12 unchanged, the removed line replaced at 13: 13..13 (the
///   captured edit);
/// - from 5, `   --   `: 5, 6, 7 unchanged, two lines removed before 8: the line before the gap,
///   7..7;
/// - from 8, `   -`: 8, 9, 10 unchanged, then the end of the file removed: 10..10;
/// - from 1, `++   `: lines 1 and 2 added: 1..2;
/// - from 1, `+  -+`: 1 added, 2 and 3 unchanged, 4 replaced: 1..4;
/// - two hunks, as an edit of two places makes, give two ranges;
/// - a hunk without its lines counts whole: from 20 for 3 lines, 20..22;
/// - 17 hunks are more than an event holds: the whole file, no ranges; 16 are kept.
#[test]
fn edits_record_the_lines_they_changed() -> TestResult {
    let one = |start, marks| edit_ranges(serde_json::json!([hunk(start, marks)]));
    assert_eq!(one(10, "   -+")?, [(13, 13)]);
    assert_eq!(one(5, "   --   ")?, [(7, 7)]);
    assert_eq!(one(8, "   -")?, [(10, 10)]);
    assert_eq!(one(1, "++   ")?, [(1, 2)]);
    assert_eq!(one(1, "+  -+")?, [(1, 4)]);
    assert_eq!(
        edit_ranges(serde_json::json!([hunk(3, " + "), hunk(40, " -+ ")]))?,
        [(4, 4), (41, 41)]
    );
    assert_eq!(
        edit_ranges(serde_json::json!([{ "newStart": 20, "newLines": 3 }]))?,
        [(20, 22)]
    );
    let hunks = |count: u32| {
        (0..count)
            .map(|at| hunk(at * 10 + 1, "+"))
            .collect::<Vec<_>>()
    };
    assert_eq!(edit_ranges(hunks(17).into())?, []);
    assert_eq!(edit_ranges(hunks(16).into())?.len(), 16);
    Ok(())
}

/// After it ran, a read that asked for some lines records the lines it returned: from line 2 for 4
/// lines of the 14 is 2..5, and from line 2 with no limit, where Claude Code returned 5 lines,
/// 2..6. A read that asked for neither a start nor a limit read the whole file, however much of it
/// Claude Code returned: the first 5 lines of the 14 are no lines, and the read keeps that it was
/// cut after line 5; all 14 of 14 are no lines and no cut. Before it runs, an offset
/// without a limit asks for no lines Cortex can know. A read that failed returned no lines.
#[test]
fn reads_record_the_lines_they_returned() -> TestResult {
    let asked = edited(
        "post-tool-use-read-range.json",
        "PostToolUse",
        &locate,
        |_| {},
    )?;
    assert_eq!(asked.tool_target, file("calc/ops.py", &[(2, 5)])?);
    let from_two = edited(
        "post-tool-use-read-range.json",
        "PostToolUse",
        &locate,
        |value| {
            value["tool_input"]
                .as_object_mut()
                .map(|input| input.remove("limit"));
            value["tool_response"]["file"]["numLines"] = 5.into();
        },
    )?;
    assert_eq!(from_two.tool_target, file("calc/ops.py", &[(2, 6)])?);
    let first_five = edited("post-tool-use-read.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["file"]["numLines"] = 5.into();
    })?;
    let cut = file("calc/ops.py", &[])?.map(|target| ToolTarget {
        cut_after: Some(5),
        ..target
    });
    assert_eq!(first_five.tool_target, cut);
    let whole = edited("post-tool-use-read.json", "PostToolUse", &locate, |_| {})?;
    assert_eq!(whole.tool_target, file("calc/ops.py", &[])?);
    let open_ended = edited(
        "pre-tool-use-read-range.json",
        "PreToolUse",
        &locate,
        |value| {
            value["tool_input"]
                .as_object_mut()
                .map(|input| input.remove("limit"));
        },
    )?;
    assert_eq!(open_ended.tool_target, file("calc/ops.py", &[])?);
    let missing = edited(
        "post-tool-use-read-range.json",
        "PostToolUseFailure",
        &locate,
        |value| {
            value
                .as_object_mut()
                .map(|payload| payload.remove("tool_response"));
            value["error"] = "File does not exist.".into();
        },
    )?;
    assert_eq!(missing.tool_target, file("calc/ops.py", &[])?);
    assert_eq!(missing.tool_result, failed(None, 20));
    Ok(())
}

/// A failed shell's exit code is the `N` of `Exit code N`, a code past `i32::MAX` read as its 32
/// bits (4294967295 is -1); another message gives none. A shell that runs on in the background, or
/// was interrupted, has no exit code yet however its result came.
#[test]
fn exit_codes_and_shells_that_have_not_finished() -> TestResult {
    let failure = |message: &str, is_interrupt: bool| -> TestResult<Option<ToolResult>> {
        let message = message.to_owned();
        Ok(edited(
            "post-tool-use-failure-bash.json",
            "PostToolUseFailure",
            &locate,
            move |value| {
                value["error"] = message.into();
                value["is_interrupt"] = is_interrupt.into();
            },
        )?
        .tool_result)
    };
    assert_eq!(
        failure("Exit code 127\nnot found", false)?,
        failed(Some(127), 23)
    );
    assert_eq!(
        failure("Exit code 4294967295", false)?,
        failed(Some(-1), 20)
    );
    assert_eq!(failure("Exit code -2", false)?, failed(Some(-2), 12));
    assert_eq!(failure("Command timed out", false)?, failed(None, 17));
    let interrupted = failure("Exit code 130", true)?.ok_or("no result")?;
    assert_eq!(
        (interrupted.exit_code, interrupted.is_interrupted),
        (Some(130), true)
    );

    let background = edited("post-tool-use-bash.json", "PostToolUse", &locate, |value| {
        value["tool_input"]["run_in_background"] = true.into();
    })?;
    assert_eq!(background.tool_result, ok(None, Some(523)));
    let stopped = edited("post-tool-use-bash.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["interrupted"] = true.into();
    })?;
    assert_eq!(
        stopped.tool_result,
        Some(ToolResult {
            exit_code: None,
            is_success: true,
            is_interrupted: true,
            output_bytes: Some(523),
            counts: None,
            failing: None,
        })
    );
    Ok(())
}

/// A search pattern and a command keep their first 1,000 characters, counted as characters: 1,500
/// two-byte letters keep 1,000.
#[test]
fn patterns_and_commands_are_cut_to_a_thousand_characters() -> TestResult {
    let long = "é".repeat(1_500);
    let grep = edited("pre-tool-use-grep.json", "PreToolUse", &locate, |value| {
        value["tool_input"]["pattern"] = long.clone().into();
    })?;
    let pattern = grep
        .tool_target
        .and_then(|target| target.pattern)
        .ok_or("no pattern")?;
    assert_eq!(pattern, "é".repeat(1_000));
    let bash = edited("pre-tool-use-bash.json", "PreToolUse", &locate, |value| {
        value["tool_input"]["command"] = long.into();
    })?;
    let command = bash
        .tool_target
        .and_then(|target| target.command)
        .ok_or("no command")?;
    assert_eq!(command.as_str(), "é".repeat(1_000));
    Ok(())
}

/// A main session started as a named agent sends that agent's type with its events: it is no
/// subagent, so the event carries no subagent type.
#[test]
fn only_subagents_have_a_subagent_type() -> TestResult {
    let stop = edited("stop.json", "Stop", &locate, |value| {
        value["agent_type"] = "reviewer".into();
    })?;
    assert_eq!(stop, base(HookKind::Stop, S1)?);
    Ok(())
}

/// The largest event a payload can make still fits a frame: a prompt of 200,000 control
/// characters, each written as six bytes in JSON, cut to 131,072 of them, with a session id at the
/// 256-byte limit.
#[test]
fn the_largest_event_fits_a_frame() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let payload = serde_json::json!({
        "session_id": "s".repeat(256),
        "cwd": native(r"C:\repo"),
        "transcript_path": native(r"C:\Users\dev\.claude\projects\C--repo\session.jsonl"),
        "prompt": "\u{1}".repeat(200_000),
    });
    let event = claude
        .normalize(
            "UserPromptSubmit",
            &serde_json::to_vec(&payload)?,
            &locate,
            stamp(),
        )?
        .ok_or("no event")?;
    let request = Request {
        request_id: 1,
        body: RequestBody::Event {
            event: Box::new(event),
        },
    };
    let frame = encode(&request)?;
    assert!(
        frame.len() <= MAX_FRAME_BYTES + LENGTH_PREFIX_BYTES,
        "{}",
        frame.len()
    );
    assert!(frame.len() > 786_432, "{}", frame.len());
    Ok(())
}

/// A shell's printed size counts both of its streams: 2 bytes on standard output and 3 on
/// standard error make 5; a shell that reports neither has no size.
#[test]
fn shell_output_counts_both_streams() -> TestResult {
    let result = |stdout: serde_json::Value, stderr: serde_json::Value| {
        edited("post-tool-use-bash.json", "PostToolUse", &locate, |value| {
            value["tool_response"]["stdout"] = stdout;
            value["tool_response"]["stderr"] = stderr;
        })
        .map(|event| event.tool_result)
    };
    assert_eq!(result("ab".into(), "cde".into())?, ok(Some(0), Some(5)));
    assert_eq!(
        result(serde_json::Value::Null, serde_json::Value::Null)?,
        ok(Some(0), None)
    );
    assert_eq!(result(12.into(), "cde".into())?, ok(Some(0), Some(3)));
    Ok(())
}

/// A field of another type than Claude Code sends reads as absent, and the event is kept with what
/// the other fields say; only a session that is not text costs the event.
#[test]
fn odd_fields_read_as_absent() -> TestResult {
    let read = edited(
        "post-tool-use-read-range.json",
        "PostToolUse",
        &locate,
        |value| {
            value["tool_response"]["file"]["startLine"] = "2".into();
        },
    )?;
    assert_eq!(read.tool_target, file("calc/ops.py", &[])?);
    let asked = edited(
        "pre-tool-use-read-range.json",
        "PreToolUse",
        &locate,
        |value| {
            value["tool_input"]["limit"] = serde_json::json!([4]);
        },
    )?;
    assert_eq!(asked.tool_target, file("calc/ops.py", &[])?);
    let unread = edited("pre-tool-use-read.json", "PreToolUse", &locate, |value| {
        value["tool_input"] = "calc/ops.py".into();
    })?;
    assert_eq!(
        (unread.tool, unread.tool_target),
        (Some(ToolKind::Read), None)
    );
    let failed = edited(
        "post-tool-use-failure-bash.json",
        "PostToolUseFailure",
        &locate,
        |value| {
            value["is_interrupt"] = "yes".into();
            value["error"] = serde_json::json!({ "text": "Exit code 3" });
        },
    )?;
    assert_eq!(
        failed.tool_result,
        Some(ToolResult {
            exit_code: None,
            is_success: false,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
            failing: None,
        })
    );
    let start = edited(
        "session-start-compact.json",
        "SessionStart",
        &locate,
        |value| {
            value["model"] = 5.into();
            value["source"] = serde_json::Value::Null;
        },
    )?;
    assert_eq!((start.model, start.source), (None, None));
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let numbered = claude.normalize(
        "Stop",
        br#"{"session_id":5,"cwd":"/repo"}"#,
        &locate,
        stamp(),
    );
    assert!(
        matches!(numbered, Err(NormalizeError::NoSession)),
        "{numbered:?}"
    );
    Ok(())
}

/// The field names Claude Code's payloads use at any depth, and one they do not.
const FIELD_NAMES: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "source",
    "model",
    "prompt",
    "tool_name",
    "tool_input",
    "tool_response",
    "error",
    "is_interrupt",
    "trigger",
    "agent_id",
    "agent_type",
    "agent_transcript_path",
    "reason",
    "file_path",
    "path",
    "pattern",
    "offset",
    "limit",
    "command",
    "file",
    "structuredPatch",
    "stdout",
    "stderr",
    "interrupted",
    "startLine",
    "numLines",
    "totalLines",
    "newStart",
    "newLines",
    "unknown",
];

/// Strings that steer a payload down the adapter's branches: tool names, start sources, a failed
/// shell's message, paths inside and outside the repository.
const STEERING_TEXTS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "Edit",
    "Write",
    "MultiEdit",
    "Bash",
    "PowerShell",
    "Agent",
    "startup",
    "manual",
    "Exit code 2",
    "Exit code -1",
    "/repo/a.rs",
    r"C:\repo\a.rs",
    "/elsewhere/b.rs",
    "",
];

proptest! {
    /// Payloads of any shape, with a session in the repository or without one, normalize to an
    /// event that keeps to its hook's fields, to none, or to an error: never a panic.
    #[test]
    fn any_payload_is_safe(body in json_value(FIELD_NAMES, STEERING_TEXTS), has_session in any::<bool>()) {
        let mut payload = match body {
            serde_json::Value::Object(fields) => fields,
            other => std::iter::once(("tool_input".to_owned(), other)).collect(),
        };
        if has_session {
            payload.insert("session_id".to_owned(), S1.into());
            payload.insert("cwd".to_owned(), native(r"C:\repo").into());
        }
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let claude = adapter("claude").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        for entry in claude.hooks() {
            if let Ok(Some(event)) = claude.normalize(entry.event_name, &bytes, &locate, stamp()) {
                assert_shape(entry.hook, &event);
            }
        }
    }

    /// A captured payload with one byte changed normalizes or fails, never panics.
    #[test]
    fn a_damaged_fixture_is_safe(
        which in any::<prop::sample::Index>(),
        at in any::<prop::sample::Index>(),
        byte in any::<u8>(),
    ) {
        let fail = |error: Box<dyn Error>| TestCaseError::fail(error.to_string());
        let names = fixture_names(&fixtures()).map_err(fail)?;
        let original = payload(which.get(&names)).map_err(fail)?;
        let name = event_name(&original).map_err(fail)?;
        let mut bytes = original;
        let position = at.index(bytes.len());
        bytes[position] = byte;
        let claude = adapter("claude").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        let _ = claude.normalize(&name, &bytes, &locate, stamp());
    }
}

proptest! {
    // Each case builds and reads two payloads of up to 480 names and lines: fewer cases than the
    // default keep the suite quick.
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// A search's response of any length, its files short and long, inside the repository and out,
    /// named twice or not, and its output lines of any shape, keeps the hits within their caps,
    /// each once, before and after the search ran.
    #[test]
    fn any_search_response_keeps_hits_within_their_caps(
        files in prop::collection::vec(listed_name(), 0..240),
        lines in prop::collection::vec(output_line(), 0..240),
    ) {
        let fail = |error: Box<dyn Error>| TestCaseError::fail(error.to_string());
        let claude = adapter("claude").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        let hooks = [("PreToolUse", HookKind::PreTool), ("PostToolUse", HookKind::PostTool)];
        for (event_name, hook) in hooks {
            let fixture = payload("post-tool-use-grep.json").map_err(fail)?;
            let mut value: serde_json::Value = serde_json::from_slice(&fixture)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            value["tool_response"]["filenames"] = files.clone().into();
            value["tool_response"]["content"] = lines.join("\n").into();
            let bytes = serde_json::to_vec(&value)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            if let Ok(Some(event)) = claude.normalize(event_name, &bytes, &locate, stamp()) {
                assert_shape(hook, &event);
            }
        }
    }
}

/// A name a search may list: short or long, nested or not, outside the repository or its top.
fn listed_name() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-c]{1,2}(/[a-c]{1,2}){0,2}\\.py",
        "\\.\\./[a-c]{1,3}",
        Just(".".to_owned()),
        "[a-c]{300,600}",
    ]
}

/// A line Grep may print: a match, a count, context, a separator, or text of any shape.
fn output_line() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-c]{1,3}(/[a-c]{1,3})?\\.py:[0-9]{1,3}:[a-z 0-9:-]{0,12}",
        "[a-c]{1,3}\\.py:[0-9]{1,3}",
        "[a-c]{1,3}\\.py-[0-9]{1,3}-[a-z 0-9:-]{0,12}",
        Just("--".to_owned()),
        "[a-c0-9:\\- ./]{0,16}",
        "[a-c]{300,600}\\.py:1:x",
    ]
}

/// The newest capture's searches name the files they found, read off the fixtures by hand: Glob's
/// six files in the order listed; Grep's match line names `calc/ops.py`; the subagent's Grep names
/// `tests/test_ops.py` (two lines, once), `NOTES.md` and `calc/ops.py`. Grep given the folder
/// `calc` lists `calc\ops.py`, in files mode and in content mode, relative to the working folder
/// and not to `calc`, so it names `calc/ops.py`. Grep given the one file `calc/ops.py` prints lines
/// that start with their numbers and name none, a hit missed; in count mode it prints
/// `calc\ops.py:3`, which names it. With context, the match lines name `tests/test_ops.py`,
/// `NOTES.md` and `calc/ops.py` and the context lines name nothing more; counting names the three
/// files with a `def`, `tests/test_ops.py`, `tests/test_fail.py` and `calc/ops.py`.
#[test]
fn searches_name_the_files_they_found() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let cases: [(&str, &[&str]); 9] = [
        (
            "post-tool-use-glob.json",
            &[
                "calc/__init__.py",
                "tests/__init__.py",
                "main.py",
                "tests/test_ops.py",
                "calc/ops.py",
                "tests/test_fail.py",
            ],
        ),
        ("post-tool-use-grep.json", &["calc/ops.py"]),
        (
            "post-tool-use-grep-subagent.json",
            &["tests/test_ops.py", "NOTES.md", "calc/ops.py"],
        ),
        ("post-tool-use-grep-folder.json", &["calc/ops.py"]),
        ("post-tool-use-grep-folder-content.json", &["calc/ops.py"]),
        ("post-tool-use-grep-one-file.json", &[]),
        ("post-tool-use-grep-one-file-count.json", &["calc/ops.py"]),
        (
            "post-tool-use-grep-context.json",
            &["tests/test_ops.py", "NOTES.md", "calc/ops.py"],
        ),
        (
            "post-tool-use-grep-count.json",
            &["tests/test_ops.py", "tests/test_fail.py", "calc/ops.py"],
        ),
    ];
    for (name, expected) in cases {
        let payload = payload_of("2.1.316", name)?;
        let event = claude
            .normalize("PostToolUse", &payload, &locate, stamp())?
            .ok_or("no event")?;
        let hits = event.tool_target.ok_or("no target")?.hits;
        let expected = expected
            .iter()
            .map(|path| RepoPath::new(path))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(hits, expected, "{name}");
    }
    Ok(())
}

/// Grep's output names a file by a match line (`path:line:text`) or a count line (`path:count`),
/// whatever colons the text holds; not by a context line, even one whose text holds a time, a
/// separator, a line without its number, however its text reads, or a line of a one-file search,
/// which starts with its number. Each file is named once, in order, after the files listed,
/// however it is spelled; one outside the repository is left out, and so is its top folder; past
/// 50 files or 4,096 bytes of paths, from the output or the list, the rest are, and so is what
/// comes after the first 200 files named; output that is not text names none, and the event
/// stands. Before the search runs it has found nothing.
#[test]
fn grep_output_names_the_files_it_matched() -> TestResult {
    let hits_at = |event_name: &str, files: serde_json::Value, content: serde_json::Value| {
        let event = edited("post-tool-use-grep.json", event_name, &locate, |value| {
            value["tool_response"]["filenames"] = files;
            value["tool_response"]["content"] = content;
        })?;
        let hits: Vec<String> = event
            .tool_target
            .ok_or("no target")?
            .hits
            .iter()
            .map(ToString::to_string)
            .collect();
        TestResult::Ok(hits)
    };
    let hits = |files, content| hits_at("PostToolUse", files, content);
    let lines = [
        "calc/ops.py:12:def divide(a, b):",
        "context.py-11-# divides",
        "notes.md-5-meeting at 10:30",
        "--",
        "main.py:3",
        "no_numbers.py:def x():",
        "ops.py:  at 10:30:00",
        "calc/ops.py:40:    return a / b",
        "../outside.py:1:x",
        "tests/time.py:7:at 10:30:00",
        "9:start = 10:30:00",
    ];
    assert_eq!(
        hits(serde_json::json!(["NOTES.md"]), lines.join("\n").into())?,
        ["NOTES.md", "calc/ops.py", "main.py", "tests/time.py"]
    );
    assert_eq!(
        hits_at(
            "PreToolUse",
            serde_json::json!(["NOTES.md"]),
            lines.join("\n").into()
        )?,
        Vec::<String>::new()
    );
    let many: Vec<String> = (0..60).map(|number| format!("f{number}.py:1:x")).collect();
    let first_fifty: Vec<String> = (0..50).map(|number| format!("f{number}.py")).collect();
    assert_eq!(
        hits(serde_json::json!([]), many.join("\n").into())?,
        first_fifty
    );
    let not_text = [
        serde_json::json!(42),
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!({"a": "b.py:1:x"}),
    ];
    for content in not_text {
        assert_eq!(hits(serde_json::json!(["main.py"]), content)?, ["main.py"]);
    }
    let listed: Vec<String> = (0..60).map(|number| format!("f{number}.py")).collect();
    assert_eq!(hits(listed.into(), "".into())?, first_fifty);
    // Twenty names of 1,024 bytes each (1,019 letters, two digits, `.py`): four hold exactly
    // 4,096 bytes, the cap; a fifth would make 5,120.
    let long: Vec<String> = (0..20)
        .map(|number| format!("{}{number:02}.py", "a".repeat(1_019)))
        .collect();
    let first_four = long.get(..4).ok_or("fewer than 4 names")?.to_vec();
    assert_eq!(hits(long.into(), "".into())?, first_four);
    assert_eq!(
        hits(
            serde_json::json!(["main.py", "./main.py", "."]),
            "main.py:1:x".into()
        )?,
        ["main.py"]
    );
    // The 200th file named counts; a 201st is not looked at.
    for (outside, expected) in [(199, vec!["main.py"]), (200, Vec::new())] {
        let mut named: Vec<String> = (0..outside)
            .map(|number| format!("../o{number}.py"))
            .collect();
        named.push("main.py".to_owned());
        assert_eq!(hits(named.into(), "".into())?, expected, "{outside}");
    }
    Ok(())
}

/// A failed command's event carries what it printed, Claude Code's `Exit code N` line left out:
/// the unittest run's output, from its first `=====` line to `FAILED (failures=1)`. A failure that
/// printed nothing after the exit code carries none, nor does a command that succeeded. Output
/// past 64 KiB keeps its end, from a line's start, and the event still fits a frame: 100,000 lines
/// of one control character (six bytes in JSON) are cut to their last 64 KiB.
#[test]
fn a_failed_command_carries_what_it_printed() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let output = |name: &str| -> TestResult<Option<String>> {
        let payload = payload(name)?;
        let event = claude
            .normalize(&event_name(&payload)?, &payload, &locate, stamp())?
            .ok_or("no event")?;
        Ok(event
            .tool_target
            .and_then(|target| target.failed_output)
            .map(String::from))
    };
    let printed = output("post-tool-use-failure-bash-test.json")?.ok_or("nothing printed")?;
    assert!(printed.starts_with("====="), "{printed}");
    assert!(printed.ends_with("FAILED (failures=1)"), "{printed}");
    assert!(!printed.contains("Exit code"), "{printed}");
    assert_eq!(output("post-tool-use-failure-bash.json")?, None);
    assert_eq!(output("post-tool-use-bash.json")?, None);

    let payload = serde_json::json!({
        "session_id": "s".repeat(256),
        "cwd": native(r"C:\repo"),
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "pytest"},
        "error": format!(
            "Exit code 1\n{}FAILED tests/test_ops.py::test_divide",
            "\u{1}\n".repeat(100_000)
        ),
    });
    let event = claude
        .normalize(
            "PostToolUseFailure",
            &serde_json::to_vec(&payload)?,
            &locate,
            stamp(),
        )?
        .ok_or("no event")?;
    let kept = event
        .tool_target
        .as_ref()
        .and_then(|target| target.failed_output.as_ref())
        .ok_or("nothing kept")?;
    assert!(
        kept.as_str()
            .ends_with("\u{1}\nFAILED tests/test_ops.py::test_divide")
    );
    // The cut moves to a line's start: whole lines of two bytes, then the last line's 37.
    assert_eq!(kept.as_str().len() % 2, 1);
    assert!(kept.as_str().len() > MAX_FAILED_OUTPUT_BYTES - 2);
    assert!(kept.as_str().len() <= MAX_FAILED_OUTPUT_BYTES);
    let frame = encode(&Request {
        request_id: 1,
        body: RequestBody::Event {
            event: Box::new(event),
        },
    })?;
    assert!(
        frame.len() <= MAX_FRAME_BYTES + LENGTH_PREFIX_BYTES,
        "{}",
        frame.len()
    );
    Ok(())
}

/// The payloads of the agents that import Claude Code's settings and so may run Cortex's Claude
/// Code hooks beside their own (Cursor, GitHub Copilot, TRAE, from their documented fixtures) make
/// no Claude Code event, each given as the Claude Code event it stands for. A Claude Code payload
/// with any one of those agents' fields added makes none either; as captured, it makes its event.
#[test]
fn another_agent_s_payload_makes_no_claude_code_event() -> TestResult {
    let claude = adapter("claude").ok_or("no Claude Code adapter")?;
    let claude_event = |name: &str| match name {
        "sessionStart" => "SessionStart".to_owned(),
        "beforeSubmitPrompt" => "UserPromptSubmit".to_owned(),
        "postToolUse" => "PostToolUse".to_owned(),
        "postToolUseFailure" => "PostToolUseFailure".to_owned(),
        "stop" => "Stop".to_owned(),
        other => other.to_owned(),
    };
    for agent in ["cursor", "copilot", "trae"] {
        let folder = common::fixture::fixtures_of(agent, "docs-2026-11-06");
        for name in fixture_names(&folder)? {
            let payload = read_fixture(agent, "docs-2026-11-06", &name)?;
            let event = claude_event(&event_name(&payload)?);
            let made = claude.normalize(&event, &payload, &locate, stamp())?;
            assert!(made.is_none(), "{agent} {name}");
        }
    }
    for field in [
        "conversation_id",
        "generation_id",
        "cursor_version",
        "workspace_roots",
        "llm_tool_name",
        "loop_count",
        "timestamp",
    ] {
        let made = normalized(
            "session-start-startup.json",
            "SessionStart",
            &locate,
            |value| {
                value[field] = serde_json::json!("2026-11-06T10:00:00.000Z");
            },
        )?;
        assert!(made.is_none(), "{field}");
    }
    assert!(
        normalized(
            "session-start-startup.json",
            "SessionStart",
            &locate,
            |_| {}
        )?
        .is_some()
    );
    Ok(())
}
