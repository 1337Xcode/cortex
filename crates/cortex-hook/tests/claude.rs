//! Every captured Claude Code payload normalized, against events written out by hand from the
//! fixtures; the payloads that make no event or no usable one; and inputs that must never panic.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

use std::error::Error;
use std::path::{Path, PathBuf};

use cortex_core::ipc::{LENGTH_PREFIX_BYTES, MAX_FRAME_BYTES, Request, RequestBody, encode};
use cortex_core::{
    Agent, CompactTrigger, Event, HookKind, Identifier, LineRange, RepoPath, ShellCommand,
    StartSource, ToolKind, ToolResult, ToolTarget, Ulid,
};
use cortex_hook::{Located, NormalizeError, Stamp, adapter};
use proptest::prelude::*;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The main session of most fixtures.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The session `/clear` opened.
const S6: &str = "00000000-0000-4000-8000-000000000006";

/// The session of the range read, the passing test and the PowerShell fixtures.
const S7: &str = "00000000-0000-4000-8000-000000000007";

/// The folder of the fixtures.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude/2.1.312")
}

/// A path the fixtures write in Windows form, in this platform's form: as captured on Windows;
/// elsewhere the home folder `C:\Users\dev` is `/home/dev`, and any other `C:\x` is `/x`.
fn native(windows: &str) -> String {
    if cfg!(windows) {
        return windows.to_owned();
    }
    let unix = windows
        .replace(r"C:\Users\dev", "/home/dev")
        .replace('\\', "/");
    unix.strip_prefix("C:")
        .map_or_else(|| unix.clone(), str::to_owned)
}

/// The fixture `name`, its paths in this platform's form.
fn payload(name: &str) -> TestResult<Vec<u8>> {
    let text = std::fs::read(fixtures().join(name))?;
    if cfg!(windows) {
        return Ok(text);
    }
    let mut value: serde_json::Value = serde_json::from_slice(&text)?;
    let rewrite = |field: Option<&mut serde_json::Value>| {
        if let Some(serde_json::Value::String(path)) = field {
            *path = native(path);
        }
    };
    for key in ["cwd", "transcript_path", "agent_transcript_path"] {
        rewrite(value.get_mut(key));
    }
    if let Some(input) = value.get_mut("tool_input") {
        rewrite(input.get_mut("file_path"));
        rewrite(input.get_mut("path"));
    }
    Ok(serde_json::to_vec(&value)?)
}

/// The fixture's event name, as the installer passes it to `cortex hook --event`.
fn event_name(payload: &[u8]) -> TestResult<String> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;
    Ok(value
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        .ok_or("no event name")?
        .to_owned())
}

/// The repository folder, already canonical, which every fixture runs in.
fn root() -> PathBuf {
    PathBuf::from(native(r"C:\repo"))
}

/// Finds every fixture's working folder in the repository.
fn locate(_: &Path) -> Option<Located> {
    Some(Located {
        cwd: root(),
        root: root(),
        agent_root: root(),
    })
}

/// The id and time every event in these tests gets.
fn stamp() -> Stamp {
    Stamp {
        event_id: Ulid::from_seed(1_700_000_000_000, b"fixture"),
        ts_ms: 1_700_000_000_000,
    }
}

/// A name.
fn id(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
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

/// A shell command.
fn command(text: &str) -> Option<ToolTarget> {
    Some(ToolTarget {
        command: Some(ShellCommand::cut(text)),
        ..ToolTarget::default()
    })
}

/// A tool that succeeded, with the exit code and printed size reported.
fn ok(exit_code: Option<i32>, output_bytes: Option<u64>) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: true,
        is_interrupted: false,
        output_bytes,
    })
}

/// A tool that failed, with the exit code and message size reported.
fn failed(exit_code: Option<i32>, output_bytes: u64) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: false,
        is_interrupted: false,
        output_bytes: Some(output_bytes),
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
                (ToolKind::Edit, file(ops, &[(13, 13)])?, ok(None, None)),
            )?),
        ),
        (
            "post-tool-use-failure-bash-test.json",
            Some(tool(
                PostTool,
                S7,
                (
                    ToolKind::Shell,
                    command("python -m unittest tests.test_fail -q"),
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
                    command("python -m unittest tests.test_fail -q"),
                    failed(Some(1), 537),
                ),
            )?),
        ),
        (
            "post-tool-use-glob.json",
            Some(tool(
                PostTool,
                S1,
                (ToolKind::Glob, search("**/*.py", here()), ok(None, None)),
            )?),
        ),
        (
            "post-tool-use-grep-subagent.json",
            Some(explore(tool(
                PostTool,
                S1,
                (ToolKind::Grep, everywhere.clone(), ok(None, None)),
            )?)?),
        ),
        (
            "post-tool-use-grep.json",
            Some(tool(
                PostTool,
                S1,
                (ToolKind::Grep, search("def divide", here()), ok(None, None)),
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
    assert_eq!(named, fixture_names()?);
    Ok(())
}

/// The names of the captured payloads, sorted.
fn fixture_names() -> TestResult<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.retain(|name| name.ends_with(".json"));
    names.sort_unstable();
    Ok(names)
}

/// The registered hooks are the ten the decisions name, with their matchers and timeouts.
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
            ("PreToolUse", Some("Read"), 300),
            ("PostToolUse", results, 500),
            ("PostToolUseFailure", results, 500),
            ("PreCompact", None, 300),
            ("Stop", None, 300),
            ("SubagentStart", None, 300),
            ("SubagentStop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert!(adapter("codex").is_none());
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

/// After it ran, a read records the lines it returned: the first 5 of a 14-line file are 1..5.
/// Before it runs, an offset without a limit asks for no lines Cortex can know. A read that failed
/// returned no lines.
#[test]
fn reads_record_the_lines_they_returned() -> TestResult {
    let first_five = edited("post-tool-use-read.json", "PostToolUse", &locate, |value| {
        value["tool_response"]["file"]["numLines"] = 5.into();
    })?;
    assert_eq!(first_five.tool_target, file("calc/ops.py", &[(1, 5)])?);
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

/// Any JSON value a few levels deep, its object keys drawn from [`FIELD_NAMES`].
fn json_value() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i64>().prop_map(serde_json::Value::from),
        prop::num::f64::NORMAL.prop_map(serde_json::Value::from),
        prop::sample::select(STEERING_TEXTS).prop_map(serde_json::Value::from),
        ".{0,40}".prop_map(serde_json::Value::from),
    ];
    leaf.prop_recursive(3, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(serde_json::Value::from),
            prop::collection::btree_map(prop::sample::select(FIELD_NAMES), inner, 0..8).prop_map(
                |fields| {
                    serde_json::Value::Object(
                        fields
                            .into_iter()
                            .map(|(key, value)| (key.to_owned(), value))
                            .collect(),
                    )
                }
            ),
        ]
    })
}

/// What every event of `hook` keeps to its own fields: a tool's only on tool events, a prompt only
/// on a prompt, every cut within its cap.
fn assert_shape(hook: HookKind, event: &Event) {
    let is_tool_event = matches!(hook, HookKind::PreTool | HookKind::PostTool);
    assert_eq!(event.hook, hook);
    assert!(is_tool_event || event.tool.is_none() && event.tool_target.is_none());
    assert!(hook == HookKind::PostTool || event.tool_result.is_none());
    assert!(hook == HookKind::Prompt || event.prompt_text.is_none());
    assert!(hook == HookKind::SessionStart || event.source.is_none());
    assert!(
        event
            .prompt_text
            .as_ref()
            .is_none_or(|prompt| prompt.len() <= 131_072)
    );
    if let Some(target) = &event.tool_target {
        assert!(
            target
                .pattern
                .as_ref()
                .is_none_or(|pattern| pattern.chars().count() <= 1_000)
        );
        assert!(
            target
                .ranges
                .iter()
                .all(|range| range.start_line() >= 1 && range.start_line() <= range.end_line())
        );
    }
}

proptest! {
    /// Payloads of any shape, with a session in the repository or without one, normalize to an
    /// event that keeps to its hook's fields, to none, or to an error: never a panic.
    #[test]
    fn any_payload_is_safe(body in json_value(), has_session in any::<bool>()) {
        let mut payload = match body {
            serde_json::Value::Object(fields) => fields,
            other => std::iter::once(("tool_input".to_owned(), other)).collect(),
        };
        if has_session {
            payload.insert("session_id".to_owned(), S1.into());
            payload.insert("cwd".to_owned(), native(r"C:\repo").into());
        }
        let bytes = serde_json::to_vec(&payload).map_err(|error| TestCaseError::fail(error.to_string()))?;
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
        let names = fixture_names().map_err(fail)?;
        let original = payload(which.get(&names)).map_err(fail)?;
        let name = event_name(&original).map_err(fail)?;
        let mut bytes = original;
        let position = at.index(bytes.len());
        bytes[position] = byte;
        let claude = adapter("claude").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        let _ = claude.normalize(&name, &bytes, &locate, stamp());
    }
}
