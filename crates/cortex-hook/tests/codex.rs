//! Every captured Codex payload normalized, against events written out by hand from the fixtures;
//! the hooks Cortex registers with Codex; payloads that make no event; and inputs that must never
//! panic.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::{Path, PathBuf};

use common::TestResult;
use common::fixture::{
    assert_shape, event_name, fixture_names, fixtures_of, id, json_value, locate, native,
    read_fixture, root, stamp,
};
use cortex_core::{
    Agent, CompactTrigger, Event, HookKind, LineRange, RepoPath, ShellCommand, StartSource,
    ToolKind, ToolResult, ToolTarget,
};
use cortex_hook::{NormalizeError, adapter};
use proptest::prelude::*;

/// The Codex version whose payloads the fixtures hold.
const CAPTURED: &str = "0.165.0";

/// The main session.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The subagent the main session started, which names its own rollout file.
const AGENT: &str = "00000000-0000-4000-8000-000000000011";

/// The session whose context was compacted automatically.
const S13: &str = "00000000-0000-4000-8000-000000000013";

/// The model every payload but the session's end names.
const MODEL: &str = "gpt-6-luna";

/// The fixture `name`, its paths in this platform's form.
fn payload(name: &str) -> TestResult<Vec<u8>> {
    read_fixture("codex", CAPTURED, name)
}

/// The rollout file of the session or subagent `id`, which Codex started at `time`.
fn rollout(time: &str, id: &str) -> PathBuf {
    PathBuf::from(native(&format!(
        r"C:\Users\dev\.codex\sessions\2026\10\29\rollout-2026-10-29T{time}-{id}.jsonl"
    )))
}

/// The event of `hook` from session `session`, whose rollout file is `transcript`, with only what
/// every event has: the model, which every payload names but the session's end.
fn base(hook: HookKind, session: &str, transcript: Option<PathBuf>) -> TestResult<Event> {
    Ok(Event {
        event_id: stamp().event_id,
        ts_ms: stamp().ts_ms,
        agent: Agent::Codex,
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
        transcript_path: transcript,
        model: Some(id(MODEL)?),
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// The event of `hook` from the main session.
fn main_session(hook: HookKind) -> TestResult<Event> {
    base(hook, S1, Some(rollout("06-03-56", S1)))
}

/// The event of `hook` from the subagent, a session of its own under the main one; its rollout
/// file is known only when the payload gives it as the subagent's own (`agent_transcript_path`).
fn subagent(hook: HookKind, transcript: Option<PathBuf>) -> TestResult<Event> {
    Ok(Event {
        session_id: id(&format!("{S1}/{AGENT}"))?,
        parent_session_id: Some(id(S1)?),
        subagent_type: Some(id("default")?),
        ..base(hook, S1, transcript)?
    })
}

/// `event` for a call of `tool` aimed at `target` that ended as `result`.
fn tool(
    event: Event,
    (kind, target, result): (ToolKind, Option<ToolTarget>, Option<ToolResult>),
) -> Event {
    Event {
        tool: Some(kind),
        tool_target: target,
        tool_result: result,
        ..event
    }
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

/// A search for `pattern` in the repository's top folder that found `hits`.
fn search(pattern: &str, hits: &[&str]) -> TestResult<Option<ToolTarget>> {
    Ok(Some(ToolTarget {
        path: Some(RepoPath::root()),
        pattern: Some(pattern.to_owned()),
        hits: hits
            .iter()
            .map(|hit| RepoPath::new(hit))
            .collect::<Result<_, _>>()?,
        ..ToolTarget::default()
    }))
}

/// A shell command.
fn command(text: &str) -> Option<ToolTarget> {
    Some(ToolTarget {
        command: Some(ShellCommand::cut(text)),
        ..ToolTarget::default()
    })
}

/// A patch that exited 0, as its response says.
fn patched() -> Option<ToolResult> {
    Some(ToolResult {
        exit_code: Some(0),
        is_success: true,
        is_interrupted: false,
        output_bytes: None,
    })
}

/// Every fixture with the event it makes, read off the fixture by hand: the paths, the commands
/// and patterns, the files a search's output names. A shell's result is not in its payload, so no
/// shell has one; a patch's is. Codex's tools other than its shell and its patches are tools of
/// no kind Cortex reads, with no target.
#[allow(clippy::too_many_lines, reason = "one entry per captured payload")]
fn expected() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    use HookKind::{PostTool, PreTool};
    let ops = || file("calc/ops.py", &[]);
    let tests = "python -m unittest -q";
    let exit_3 = "python -c \"import sys; sys.exit(3)\"";
    let calls_add = r"\badd\s*\(";
    let compacted = rollout("06-13-42", S13);
    Ok(vec![
        ("post-compact-auto.json", None),
        (
            "post-tool-use-apply-patch-add.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::ApplyPatch, file("NOTES.md", &[])?, patched()),
            )),
        ),
        (
            "post-tool-use-apply-patch.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::ApplyPatch, ops()?, patched()),
            )),
        ),
        (
            "post-tool-use-bash-grep.json",
            Some(tool(
                main_session(PostTool)?,
                (
                    ToolKind::Grep,
                    search("divide", &["calc/ops.py", "tests/test_ops.py"])?,
                    None,
                ),
            )),
        ),
        (
            "post-tool-use-bash-nonzero-exit.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::Shell, command(exit_3), None),
            )),
        ),
        (
            "post-tool-use-bash-read.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::Read, ops()?, None),
            )),
        ),
        (
            "post-tool-use-bash-subagent.json",
            Some(tool(
                subagent(PostTool, None)?,
                (
                    ToolKind::Grep,
                    search(calls_add, &["calc/ops.py", "tests/test_ops.py", "main.py"])?,
                    None,
                ),
            )),
        ),
        (
            "post-tool-use-bash-test-failing.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::Shell, command(tests), None),
            )),
        ),
        (
            "post-tool-use-bash-test-passing.json",
            Some(tool(
                main_session(PostTool)?,
                (ToolKind::Shell, command(tests), None),
            )),
        ),
        (
            "post-tool-use-spawn-agent.json",
            Some(tool(main_session(PostTool)?, (ToolKind::Other, None, None))),
        ),
        (
            "post-tool-use-wait-agent.json",
            Some(tool(main_session(PostTool)?, (ToolKind::Other, None, None))),
        ),
        (
            "pre-compact-auto.json",
            Some(Event {
                compact_trigger: Some(CompactTrigger::Auto),
                ..base(HookKind::Compact, S13, Some(compacted.clone()))?
            }),
        ),
        (
            "pre-tool-use-apply-patch.json",
            Some(tool(
                main_session(PreTool)?,
                (ToolKind::ApplyPatch, ops()?, None),
            )),
        ),
        (
            "pre-tool-use-bash-grep.json",
            Some(tool(
                main_session(PreTool)?,
                (ToolKind::Grep, search("divide", &[])?, None),
            )),
        ),
        (
            "pre-tool-use-bash-read.json",
            Some(tool(main_session(PreTool)?, (ToolKind::Read, ops()?, None))),
        ),
        (
            "pre-tool-use-bash-subagent.json",
            Some(tool(
                subagent(PreTool, None)?,
                (ToolKind::Grep, search(calls_add, &[])?, None),
            )),
        ),
        (
            "pre-tool-use-bash-test.json",
            Some(tool(
                main_session(PreTool)?,
                (ToolKind::Shell, command(tests), None),
            )),
        ),
        (
            "pre-tool-use-spawn-agent.json",
            Some(tool(main_session(PreTool)?, (ToolKind::Other, None, None))),
        ),
        (
            "pre-tool-use-wait-agent.json",
            Some(tool(main_session(PreTool)?, (ToolKind::Other, None, None))),
        ),
        (
            "session-end.json",
            Some(Event {
                model: None,
                end_reason: Some(id("other")?),
                ..main_session(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start-compact.json",
            Some(Event {
                source: Some(StartSource::Compact),
                ..base(HookKind::SessionStart, S13, Some(compacted))?
            }),
        ),
        (
            "session-start-resume.json",
            Some(Event {
                source: Some(StartSource::Resume),
                // A resumed session's rollout file is named with the Windows verbatim prefix.
                transcript_path: Some(PathBuf::from(native(&format!(
                    r"\\?\C:\Users\dev\.codex\sessions\2026\10\29\rollout-2026-10-29T06-03-56-{S1}.jsonl"
                )))),
                ..main_session(HookKind::SessionStart)?
            }),
        ),
        (
            "session-start-startup.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..main_session(HookKind::SessionStart)?
            }),
        ),
        ("stop.json", Some(main_session(HookKind::Stop)?)),
        (
            "subagent-start.json",
            Some(subagent(HookKind::SubagentStart, None)?),
        ),
        (
            "subagent-stop.json",
            Some(subagent(
                HookKind::SubagentStop,
                Some(rollout("06-04-21", AGENT)),
            )?),
        ),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(prompt_of("user-prompt-submit.json")?),
                ..main_session(HookKind::Prompt)?
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
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let table = expected()?;
    for (name, expected) in &table {
        let payload = payload(name)?;
        let found = codex.normalize(&event_name(&payload)?, &payload, &locate, stamp())?;
        assert_eq!(&found, expected, "{name}");
    }
    let mut named: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    named.sort_unstable();
    assert_eq!(named, fixture_names(&fixtures_of("codex", CAPTURED))?);
    Ok(())
}

/// The registered hooks are the nine Codex has events for that Cortex uses, with their matchers
/// (regular expressions on the tool's name) and timeouts; no hook is registered for after a
/// compaction, whose text Codex drops.
#[test]
fn the_hooks_are_the_ones_cortex_registers() -> TestResult {
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let hooks: Vec<(&str, Option<&str>, u128)> = codex
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
    assert_eq!(
        hooks,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            ("PreToolUse", Some("^Bash$"), 300),
            ("PostToolUse", Some("^(Bash|apply_patch)$"), 500),
            ("PreCompact", None, 300),
            ("Stop", None, 300),
            ("SubagentStart", None, 300),
            ("SubagentStop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert_eq!(codex.agent(), Agent::Codex);
    let versions = codex.versions().ok_or("no versions")?;
    assert_eq!(
        (versions.oldest, versions.newest_tested),
        (CAPTURED, CAPTURED)
    );
    Ok(())
}

/// A payload outside any repository Cortex serves, an event Cortex does not register, and a
/// payload with no working folder make no event; one with no session is refused, as is text that
/// is not JSON. A byte-order mark in front is read past.
#[test]
fn payloads_that_make_no_event() -> TestResult {
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let stop = payload("stop.json")?;
    let nowhere = |_: &Path| None;
    assert_eq!(codex.normalize("Stop", &stop, &nowhere, stamp())?, None);
    assert_eq!(
        codex.normalize("PostCompact", &stop, &locate, stamp())?,
        None
    );
    assert_eq!(
        codex.normalize("Stop", br#"{"session_id":"s"}"#, &locate, stamp())?,
        None
    );
    let no_session = codex.normalize("Stop", br#"{"cwd":"/repo"}"#, &locate, stamp());
    assert!(
        matches!(no_session, Err(NormalizeError::NoSession)),
        "{no_session:?}"
    );
    let not_json = codex.normalize("Stop", b"{{{", &locate, stamp());
    assert!(
        matches!(not_json, Err(NormalizeError::Json(_))),
        "{not_json:?}"
    );
    let mut with_mark = b"\xEF\xBB\xBF".to_vec();
    with_mark.extend_from_slice(&stop);
    assert_eq!(
        codex.normalize("Stop", &with_mark, &locate, stamp())?,
        Some(main_session(HookKind::Stop)?)
    );
    Ok(())
}

/// The fixture `name` with its command replaced by `command`, normalized as `event_name`.
fn with_command(name: &str, event_name: &str, command: &str) -> TestResult<Option<Event>> {
    let mut value: serde_json::Value = serde_json::from_slice(&payload(name)?)?;
    value["tool_input"]["command"] = command.into();
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    Ok(codex.normalize(event_name, &serde_json::to_vec(&value)?, &locate, stamp())?)
}

/// A read of some lines records them, before and after it ran; a read of a file outside the
/// repository makes no event, nor does a patch of files all outside it; a patch that names no
/// file has no target; a patch of several files names the first as its path and the others after
/// it, each once, leaving out those outside the repository.
#[test]
fn reads_and_patches_name_their_files() -> TestResult {
    let read = with_command(
        "post-tool-use-bash-read.json",
        "PostToolUse",
        "sed -n '3,9p' calc/ops.py",
    )?
    .ok_or("no event")?;
    assert_eq!(read.tool, Some(ToolKind::Read));
    assert_eq!(read.tool_target, file("calc/ops.py", &[(3, 9)])?);
    let before = with_command(
        "pre-tool-use-bash-read.json",
        "PreToolUse",
        "Get-Content calc/ops.py -TotalCount 12",
    )?
    .ok_or("no event")?;
    assert_eq!(before.tool_target, file("calc/ops.py", &[(1, 12)])?);
    let outside = native(r"C:\elsewhere\a.py");
    for (name, event_name, command) in [
        (
            "post-tool-use-bash-read.json",
            "PostToolUse",
            format!("cat {outside}"),
        ),
        (
            "post-tool-use-apply-patch.json",
            "PostToolUse",
            format!("*** Begin Patch\n*** Update File: {outside}\n*** End Patch"),
        ),
        (
            "post-tool-use-bash-read.json",
            "PostToolUse",
            "cat ../elsewhere/a.py".to_owned(),
        ),
    ] {
        assert_eq!(with_command(name, event_name, &command)?, None, "{command}");
    }
    let empty = with_command(
        "post-tool-use-apply-patch.json",
        "PostToolUse",
        "*** Begin Patch\n*** End Patch",
    )?
    .ok_or("no event")?;
    assert_eq!(empty.tool, Some(ToolKind::ApplyPatch));
    assert_eq!(empty.tool_target, None);
    let several = with_command(
        "post-tool-use-apply-patch.json",
        "PostToolUse",
        "*** Begin Patch\n*** Add File: main2.py\n+x\n*** Update File: calc/ops.py\n*** End Patch",
    )?
    .ok_or("no event")?;
    assert_eq!(
        several.tool_target,
        Some(ToolTarget {
            more_paths: vec![RepoPath::new("calc/ops.py")?],
            ..file("main2.py", &[])?.ok_or("a target")?
        })
    );
    let mixed = with_command(
        "post-tool-use-apply-patch.json",
        "PostToolUse",
        &format!(
            "*** Begin Patch\n*** Update File: {outside}\n*** Update File: calc/ops.py\n\
             *** Update File: main.py\n*** Update File: calc/ops.py\n*** End Patch"
        ),
    )?
    .ok_or("no event")?;
    assert_eq!(
        mixed.tool_target,
        Some(ToolTarget {
            more_paths: vec![RepoPath::new("main.py")?],
            ..file("calc/ops.py", &[])?.ok_or("a target")?
        })
    );
    Ok(())
}

/// The finished command `id`, exited with `code`, as a rollout line records it.
fn finished(id: &str, code: i32) -> String {
    format!(
        r#"{{"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"CommandExecution","id":"{id}","status":"completed","exit_code":{code}}}}}}}"#
    )
}

/// A shell command's result, and a read's, is the exit code its line in the session's rollout
/// gives, the rollout in a Codex sessions folder: the failing command's 3 is no success, the
/// read's 0 is one. A search's result stays unknown, as does a command the rollout does not
/// hold, and one whose rollout is not where Codex keeps them.
#[test]
fn a_command_s_result_is_read_from_its_rollout() -> TestResult {
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let folder = tempfile::tempdir()?;
    let day = folder.path().join(".codex/sessions/2026/10/29");
    std::fs::create_dir_all(&day)?;
    let rollout = day.join(format!("rollout-2026-10-29T06-03-56-{S1}.jsonl"));
    let lines = [
        finished("exec-00000000-0000-4000-8000-000000000003", 0),
        finished("exec-00000000-0000-4000-8000-000000000004", 0),
        finished("exec-00000000-0000-4000-8000-000000000009", 3),
    ];
    std::fs::write(&rollout, lines.join("\n") + "\n")?;
    let outside = folder.path().join(format!("rollout-{S1}.jsonl"));
    std::fs::write(&outside, lines.join("\n") + "\n")?;
    let result = |name: &str, transcript: &Path| -> TestResult<Option<ToolResult>> {
        let mut value: serde_json::Value = serde_json::from_slice(&payload(name)?)?;
        value["transcript_path"] = transcript.to_string_lossy().into_owned().into();
        let event = codex
            .normalize(
                "PostToolUse",
                &serde_json::to_vec(&value)?,
                &locate,
                stamp(),
            )?
            .ok_or("no event")?;
        Ok(event.tool_result)
    };
    let exited = |code: i32| {
        Some(ToolResult {
            exit_code: Some(code),
            is_success: code == 0,
            is_interrupted: false,
            output_bytes: None,
        })
    };
    assert_eq!(
        result("post-tool-use-bash-nonzero-exit.json", &rollout)?,
        exited(3)
    );
    assert_eq!(result("post-tool-use-bash-read.json", &rollout)?, exited(0));
    assert_eq!(result("post-tool-use-bash-grep.json", &rollout)?, None);
    assert_eq!(
        result("post-tool-use-bash-test-failing.json", &rollout)?,
        None
    );
    assert_eq!(
        result("post-tool-use-bash-nonzero-exit.json", &outside)?,
        None
    );
    Ok(())
}

/// A patch's result is the exit code its response opens with: a patch that failed is no success;
/// a response with no code, or no response, gives no result.
#[test]
fn a_patch_s_result_is_its_exit_code() -> TestResult {
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let result = |response: serde_json::Value| -> TestResult<Option<ToolResult>> {
        let mut value: serde_json::Value =
            serde_json::from_slice(&payload("post-tool-use-apply-patch.json")?)?;
        value["tool_response"] = response;
        let event = codex
            .normalize(
                "PostToolUse",
                &serde_json::to_vec(&value)?,
                &locate,
                stamp(),
            )?
            .ok_or("no event")?;
        Ok(event.tool_result)
    };
    assert_eq!(
        result("Exit code: 1\nWall time: 0 seconds\nOutput:\nerror\n".into())?,
        Some(ToolResult {
            exit_code: Some(1),
            is_success: false,
            is_interrupted: false,
            output_bytes: None,
        })
    );
    assert_eq!(result("Success.".into())?, None);
    assert_eq!(result(serde_json::Value::Null)?, None);
    assert_eq!(result(serde_json::json!({"exit_code": 0}))?, None);
    Ok(())
}

/// A search's output names its files relative to the working folder, each once, those outside the
/// repository left out; before the search ran it found nothing.
#[test]
fn a_search_s_output_names_its_hits() -> TestResult {
    let codex = adapter("codex").ok_or("no Codex adapter")?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&payload("post-tool-use-bash-grep.json")?)?;
    let outside = native(r"C:\elsewhere\b.py");
    value["tool_response"] = format!(
        "main.py:3:print(add(1, 2))\nmain.py:4:x\n{outside}:1:y\ncalc/ops.py:12:def divide(a, b):\n"
    )
    .into();
    let event = codex
        .normalize(
            "PostToolUse",
            &serde_json::to_vec(&value)?,
            &locate,
            stamp(),
        )?
        .ok_or("no event")?;
    assert_eq!(
        event.tool_target,
        search("divide", &["main.py", "calc/ops.py"])?
    );
    let before = codex
        .normalize("PreToolUse", &serde_json::to_vec(&value)?, &locate, stamp())?
        .ok_or("no event")?;
    assert_eq!(before.tool_target, search("divide", &[])?);
    Ok(())
}

/// The keys of the payloads the proptests build: every field the adapter reads, and one it does
/// not.
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
    "trigger",
    "agent_id",
    "agent_type",
    "agent_transcript_path",
    "reason",
    "command",
    "unknown",
];

/// Strings that steer a payload down the adapter's branches: tool names, commands that read,
/// search and patch, start sources, paths inside and outside the repository.
const STEERING_TEXTS: &[&str] = &[
    "Bash",
    "apply_patch",
    "cat calc/ops.py",
    "head -n 0 calc/ops.py",
    "sed -n '9,2p' calc/ops.py",
    "rg -n x . | head",
    "Get-Content -Path",
    "*** Begin Patch\n*** Update File: calc/ops.py\n*** End Patch",
    "*** Update File: ../x.py",
    "Exit code: 2",
    "a.py:1:x",
    "startup",
    "compact",
    "auto",
    "/repo/a.rs",
    r"C:\repo\a.rs",
    "/elsewhere/b.rs",
    "'\"|;&&",
    "",
];

proptest! {
    /// Payloads of any shape, with a session in the repository or without one, normalize to an
    /// event that keeps to its hook's fields, to none, or to an error: never a panic.
    #[test]
    fn any_payload_is_safe(
        body in json_value(FIELD_NAMES, STEERING_TEXTS),
        has_session in any::<bool>(),
    ) {
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
        let codex = adapter("codex").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        for entry in codex.hooks() {
            if let Ok(Some(event)) = codex.normalize(entry.event_name, &bytes, &locate, stamp()) {
                assert_shape(entry.hook, &event);
            }
        }
    }

    /// A shell command of any text, read or searched or neither, never panics, and a read's lines
    /// are a range.
    #[test]
    fn any_command_is_safe(command in "[ -~\n]{0,80}") {
        let fail = |error: Box<dyn std::error::Error>| TestCaseError::fail(error.to_string());
        for (name, event_name) in [
            ("post-tool-use-bash-read.json", "PostToolUse"),
            ("pre-tool-use-bash-read.json", "PreToolUse"),
            ("post-tool-use-apply-patch.json", "PostToolUse"),
        ] {
            if let Some(event) = with_command(name, event_name, &command).map_err(fail)? {
                let hook = if event_name == "PreToolUse" { HookKind::PreTool } else { HookKind::PostTool };
                assert_shape(hook, &event);
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
        let fail = |error: Box<dyn std::error::Error>| TestCaseError::fail(error.to_string());
        let names = fixture_names(&fixtures_of("codex", CAPTURED)).map_err(fail)?;
        let original = payload(which.get(&names)).map_err(fail)?;
        let name = event_name(&original).map_err(fail)?;
        let mut bytes = original;
        let position = at.index(bytes.len());
        bytes[position] = byte;
        let codex = adapter("codex").ok_or_else(|| TestCaseError::fail("no adapter"))?;
        let _ = codex.normalize(&name, &bytes, &locate, stamp());
    }
}
