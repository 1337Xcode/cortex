//! The adapters built from an agent's documentation, not yet run live: Qwen Code, Droid and Devin.
//! Each payload typed from the documentation is normalized against an event written out by hand;
//! each adapter's hooks are the ones its documentation supports; and payloads of any shape never
//! panic.

mod common;

use std::path::PathBuf;

use common::TestResult;
use common::fixture::{
    assert_shape, event_name, fixture_names, fixtures_of, id, json_value, locate, native,
    read_fixture, root, stamp,
};
use cortex_core::{
    Agent, Event, HookKind, RepoPath, ShellCommand, StartSource, ToolKind, ToolResult, ToolTarget,
};
use cortex_hook::adapter;
use proptest::prelude::*;

/// The folder the documented payloads of every agent here are in.
const DOCUMENTED: &str = "docs-2026-11-06";

/// Every agent whose adapter is built from its documentation, by its name on the command line.
const DOCUMENTED_AGENTS: &[&str] = &[
    "qwen",
    "droid",
    "devin",
    "trae",
    "junie",
    "augment",
    "openhands",
    "gemini",
    "cursor",
    "copilot",
    "antigravity",
    "kiro",
    "cline",
    "crush",
    "hermes",
];

/// The session of every documented payload.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The event of `hook` from `agent`'s session, its transcript at `transcript` (in Windows form).
fn base(agent: Agent, hook: HookKind, transcript: Option<&str>) -> TestResult<Event> {
    Ok(Event {
        event_id: stamp().event_id,
        ts_ms: stamp().ts_ms,
        agent,
        agent_version: None,
        session_id: id(S1)?,
        parent_session_id: None,
        cwd: root(),
        repo_root: root(),
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: transcript.map(|path| PathBuf::from(native(path))),
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
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

/// A target at `path`.
fn file(path: &str) -> TestResult<Option<ToolTarget>> {
    Ok(Some(ToolTarget {
        path: Some(RepoPath::new(path)?),
        ..ToolTarget::default()
    }))
}

/// A call that ended as `is_success` says, its exit code unknown.
fn ended(is_success: bool) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code: None,
        is_success,
        is_interrupted: false,
        output_bytes: None,
    })
}

/// The prompt every documented prompt payload carries as the user's.
const PROMPT: &str = "Fix divide in calc/ops.py";

/// Qwen Code's payloads and the events they make, read off the fixtures by hand: the prompt from
/// `submitted_prompt`, and a prompt event without it (a tool's result going back) no event; a
/// read with an offset and a limit, whose lines Qwen Code's documentation does not count, no
/// target; a call's result a success after `PostToolUse` and a failure after
/// `PostToolUseFailure`, its exit code unknown.
fn qwen() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let transcript = format!(r"C:\Users\dev\.qwen\projects\repo\chats\{S1}.jsonl");
    let at = |hook| base(Agent::Qwen, hook, Some(&transcript));
    Ok(vec![
        (
            "post-tool-use-edit.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "post-tool-use-failure-shell.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (
                    ToolKind::Shell,
                    Some(ToolTarget {
                        command: Some(ShellCommand::cut("python -m pytest -q")),
                        ..ToolTarget::default()
                    }),
                    ended(false),
                ),
            )),
        ),
        (
            "post-tool-use-grep.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (
                    ToolKind::Grep,
                    Some(ToolTarget {
                        path: Some(RepoPath::new("calc")?),
                        pattern: Some("def divide".to_owned()),
                        ..ToolTarget::default()
                    }),
                    ended(true),
                ),
            )),
        ),
        (
            "pre-tool-use-read-range.json",
            Some(tool(at(HookKind::PreTool)?, (ToolKind::Read, None, None))),
        ),
        (
            "pre-tool-use-read.json",
            Some(tool(
                at(HookKind::PreTool)?,
                (ToolKind::Read, file("calc/ops.py")?, None),
            )),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("other")?),
                ..at(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                model: Some(id("qwen3-coder")?),
                ..at(HookKind::SessionStart)?
            }),
        ),
        ("stop.json", Some(at(HookKind::Stop)?)),
        ("user-prompt-submit-continuation.json", None),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Droid's payloads and the events they make, read off the fixtures by hand: `Create` a write,
/// `Execute` a shell; a call's result unknown, Droid's `PostToolUse` following every call.
fn droid() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let transcript = format!(r"C:\Users\dev\.factory\projects\repo\{S1}.jsonl");
    let at = |hook| base(Agent::Droid, hook, Some(&transcript));
    Ok(vec![
        (
            "post-tool-use-create.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Write, file("NOTES.md")?, None),
            )),
        ),
        (
            "post-tool-use-execute.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (
                    ToolKind::Shell,
                    Some(ToolTarget {
                        command: Some(ShellCommand::cut("python -m pytest -q")),
                        ..ToolTarget::default()
                    }),
                    None,
                ),
            )),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("other")?),
                ..at(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
        ("stop.json", Some(at(HookKind::Stop)?)),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// A shell command.
fn command(text: &str) -> Option<ToolTarget> {
    Some(ToolTarget {
        command: Some(ShellCommand::cut(text)),
        ..ToolTarget::default()
    })
}

/// The test command every documented shell payload runs.
const TESTS: &str = "python -m pytest -q";

/// TRAE's payloads and the events they make, read off the fixtures by hand: a read whose input
/// names no file (TRAE's documentation names no field for it) aims at none; `RunCommand` a shell;
/// a call's result unknown.
fn trae() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Trae, hook, None);
    Ok(vec![
        (
            "post-tool-use-run-command.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), None),
            )),
        ),
        (
            "pre-tool-use-read.json",
            Some(tool(at(HookKind::PreTool)?, (ToolKind::Read, None, None))),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
        ("stop.json", Some(at(HookKind::Stop)?)),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Junie's payloads and the events they make: its start and its prompt, which name the session
/// and the folder; its tool and stop events, which name neither, are not registered.
fn junie() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Junie, hook, None);
    Ok(vec![
        ("pre-tool-use.json", None),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
        ("stop.json", None),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Augment's payloads and the events they make: the session is `conversation_id`, the prompt
/// `user_prompt`; an edit's file is its input's `path`, relative to the working folder; a call's
/// result is the opposite of `tool_is_error`.
fn augment() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Augment, hook, None);
    Ok(vec![
        (
            "post-tool-use-bash.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(false)),
            )),
        ),
        (
            "post-tool-use-edit.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
        ("session-end.json", Some(at(HookKind::SessionEnd)?)),
        ("session-start.json", Some(at(HookKind::SessionStart)?)),
        ("stop.json", Some(at(HookKind::Stop)?)),
    ])
}

/// OpenHands' payloads and the events they make: the folder is `working_dir`, the prompt
/// `message`; the file editor reads at `view` (not one that names a range, whose counting is not
/// documented) and edits at `str_replace`; a terminal command's result is its exit code.
fn openhands() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::OpenHands, hook, None);
    Ok(vec![
        (
            "post-tool-use-str-replace.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, file("calc/ops.py")?, None),
            )),
        ),
        (
            "post-tool-use-terminal.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (
                    ToolKind::Shell,
                    command(TESTS),
                    Some(ToolResult {
                        exit_code: Some(1),
                        is_success: false,
                        is_interrupted: false,
                        output_bytes: None,
                    }),
                ),
            )),
        ),
        (
            "post-tool-use-view-range.json",
            Some(tool(at(HookKind::PostTool)?, (ToolKind::Read, None, None))),
        ),
        (
            "post-tool-use-view.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, None),
            )),
        ),
        ("session-end.json", Some(at(HookKind::SessionEnd)?)),
        ("session-start.json", Some(at(HookKind::SessionStart)?)),
        ("stop.json", Some(at(HookKind::Stop)?)),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Gemini CLI's payloads and the events they make, read off the fixtures by hand: its own event
/// names (`BeforeAgent` the prompt, `AfterTool` a tool's result, `PreCompress` a compaction,
/// `AfterAgent` the end of a turn); a read naming its lines, whose counting is not documented, no
/// target; a call's result a failure when its response carries an `error`.
fn gemini() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let transcript = format!(r"C:\Users\dev\.gemini\tmp\repo\chats\session-{S1}.json");
    let at = |hook| base(Agent::Gemini, hook, Some(&transcript));
    Ok(vec![
        ("after-agent.json", Some(at(HookKind::Stop)?)),
        (
            "after-tool-read-lines.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, None, ended(true)),
            )),
        ),
        (
            "after-tool-read.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "after-tool-shell-error.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(false)),
            )),
        ),
        (
            "before-agent.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
        (
            "pre-compress.json",
            Some(Event {
                compact_trigger: Some(cortex_core::CompactTrigger::Auto),
                ..at(HookKind::Compact)?
            }),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("exit")?),
                ..at(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
    ])
}

/// Cursor's payloads and the events they make, read off the fixtures by hand: the session is
/// `conversation_id`; only the tool events name a working folder, so with `CURSOR_PROJECT_DIR`
/// unset, as here, the start, the prompt and the stop make none; a call's result follows
/// `postToolUse` (a success) and `postToolUseFailure`.
fn cursor() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| -> TestResult<Event> {
        Ok(Event {
            model: Some(id("cursor-model")?),
            ..base(Agent::Cursor, hook, None)?
        })
    };
    Ok(vec![
        ("before-submit-prompt.json", None),
        (
            "post-tool-use-failure-read.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, ended(false)),
            )),
        ),
        (
            "post-tool-use-shell.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(true)),
            )),
        ),
        ("session-start.json", None),
        ("stop.json", None),
    ])
}

/// Copilot's payloads and the events they make, read off the fixtures by hand: its tools' inputs
/// are not documented, so a shell names no command and a view no file; a call's result follows
/// `PostToolUse` and `PostToolUseFailure`.
fn copilot() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Copilot, hook, None);
    Ok(vec![
        (
            "post-tool-use-bash.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, Some(ToolTarget::default()), ended(true)),
            )),
        ),
        (
            "post-tool-use-failure-view.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, None, ended(false)),
            )),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("complete")?),
                ..at(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Antigravity's payloads and the events they make, read off the fixtures by hand: its own fields
/// moved to Claude Code's places (the session `conversationId`, the working folder the first
/// workspace, the model `modelName`, the tool call's name and Pascal-case arguments); the prompt
/// on the turn's first model call only (`invocationNum` 0), a later call no event; a view naming
/// its lines no target; a call's result a failure when its `error` is not empty; no transcript,
/// its path being written with `~`.
fn antigravity() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| -> TestResult<Event> {
        Ok(Event {
            model: Some(id("gemini-3.6-flash-medium")?),
            ..base(Agent::Antigravity, hook, None)?
        })
    };
    Ok(vec![
        (
            "post-tool-use-grep-search.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (
                    ToolKind::Grep,
                    Some(ToolTarget {
                        path: Some(RepoPath::new("calc")?),
                        pattern: Some("def divide".to_owned()),
                        ..ToolTarget::default()
                    }),
                    ended(true),
                ),
            )),
        ),
        (
            "post-tool-use-replace-file-content.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "post-tool-use-run-command-error.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(false)),
            )),
        ),
        (
            "post-tool-use-view-file-lines.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, None, ended(true)),
            )),
        ),
        (
            "post-tool-use-view-file.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "pre-invocation-first.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
        ("pre-invocation-later.json", None),
        ("session-start.json", Some(at(HookKind::SessionStart)?)),
        ("stop.json", Some(at(HookKind::Stop)?)),
    ])
}

/// Kiro's payloads and the events they make, read off the fixtures by hand: the older agent's
/// `read` of lines, its file the first operation's path; a shell whose input is not documented
/// no command; a call's result its response's `success`.
fn kiro() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Kiro, hook, None);
    Ok(vec![
        (
            "post-tool-use-execute-bash-failed.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, Some(ToolTarget::default()), ended(false)),
            )),
        ),
        (
            "post-tool-use-read.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, ended(true)),
            )),
        ),
        ("session-end.json", Some(at(HookKind::SessionEnd)?)),
        ("session-start.json", Some(at(HookKind::SessionStart)?)),
        ("stop.json", Some(at(HookKind::Stop)?)),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Cline's payloads and the events they make, read off the fixtures by hand: the session `taskId`,
/// the working folder the first workspace root, the model `model.slug`; each event's fields under
/// its name; a tool's path relative to the working folder; a call's result `postToolUse.success`;
/// a task's start and its resumption both a session's start.
fn cline() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| -> TestResult<Event> {
        Ok(Event {
            model: Some(id("anthropic/claude-sonnet-4.5")?),
            ..base(Agent::Cline, hook, None)?
        })
    };
    Ok(vec![
        (
            "post-tool-use-execute-command.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(true)),
            )),
        ),
        (
            "post-tool-use-read-file.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, ended(true)),
            )),
        ),
        (
            "post-tool-use-replace-in-file-failed.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, file("calc/ops.py")?, ended(false)),
            )),
        ),
        ("pre-compact.json", Some(at(HookKind::Compact)?)),
        ("task-complete.json", Some(at(HookKind::Stop)?)),
        ("task-resume.json", Some(at(HookKind::SessionStart)?)),
        ("task-start.json", Some(at(HookKind::SessionStart)?)),
        (
            "user-prompt-submit.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// Crush's payloads and the events they make, read off the fixtures by hand: a turn's start a
/// prompt event with no text, which Crush does not give; a file tool whose input fields are not
/// documented no target; a call's result follows `PostToolUse` and `PostToolUseFailure`.
fn crush() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Crush, hook, None);
    Ok(vec![
        (
            "post-tool-use-bash.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), ended(true)),
            )),
        ),
        (
            "post-tool-use-failure-edit.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Edit, None, ended(false)),
            )),
        ),
        (
            "pre-compact.json",
            Some(Event {
                compact_trigger: Some(cortex_core::CompactTrigger::Auto),
                ..at(HookKind::Compact)?
            }),
        ),
        (
            "pre-tool-use-view.json",
            Some(tool(at(HookKind::PreTool)?, (ToolKind::Read, None, None))),
        ),
        (
            "session-end.json",
            Some(Event {
                end_reason: Some(id("exit")?),
                ..at(HookKind::SessionEnd)?
            }),
        ),
        (
            "session-start.json",
            Some(Event {
                source: Some(StartSource::Startup),
                ..at(HookKind::SessionStart)?
            }),
        ),
        ("turn-end.json", Some(at(HookKind::Stop)?)),
        ("turn-start.json", Some(at(HookKind::Prompt)?)),
    ])
}

/// Hermes' payloads and the events they make, read off the fixtures by hand: the prompt
/// `extra.user_message`; a call's result unknown, a shell hook not being told it.
fn hermes() -> TestResult<Vec<(&'static str, Option<Event>)>> {
    let at = |hook| base(Agent::Hermes, hook, None);
    Ok(vec![
        ("on-session-end.json", Some(at(HookKind::SessionEnd)?)),
        ("on-session-start.json", Some(at(HookKind::SessionStart)?)),
        (
            "post-tool-call-read-file.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Read, file("calc/ops.py")?, None),
            )),
        ),
        (
            "post-tool-call-terminal.json",
            Some(tool(
                at(HookKind::PostTool)?,
                (ToolKind::Shell, command(TESTS), None),
            )),
        ),
        (
            "pre-llm-call.json",
            Some(Event {
                prompt_text: Some(PROMPT.to_owned()),
                ..at(HookKind::Prompt)?
            }),
        ),
    ])
}

/// The event a documented payload was written for: the one the payload names, or for
/// Antigravity's, which name none, the one the fixture's file name begins with.
fn event_of(agent: &str, fixture: &str, payload: &[u8]) -> TestResult<String> {
    if agent != "antigravity" {
        return event_name(payload);
    }
    let named = [
        ("session-start", "SessionStart"),
        ("pre-invocation", "PreInvocation"),
        ("post-tool-use", "PostToolUse"),
        ("stop", "Stop"),
    ];
    let (_, event) = named
        .iter()
        .find(|(prefix, _)| fixture.starts_with(prefix))
        .ok_or("no event")?;
    Ok((*event).to_owned())
}

/// Kiro's older agent's `read` is a read of a file only when its first operation reads lines: a
/// read of a folder (mode `Directory`) is a tool of no kind Cortex reads. The payload is
/// `post-tool-use-read.json` with its mode changed.
#[test]
fn kiro_reads_lines_only() -> TestResult {
    let mut payload: serde_json::Value = serde_json::from_slice(&read_fixture(
        "kiro",
        DOCUMENTED,
        "post-tool-use-read.json",
    )?)?;
    payload["tool_input"]["operations"][0]["mode"] = "Directory".into();
    let kiro = adapter("kiro").ok_or("kiro")?;
    let found = kiro.normalize(
        "PostToolUse",
        &serde_json::to_vec(&payload)?,
        &locate,
        stamp(),
    )?;
    let expected = tool(
        base(Agent::Kiro, HookKind::PostTool, None)?,
        (ToolKind::Other, None, ended(true)),
    );
    assert_eq!(found, Some(expected));
    Ok(())
}

/// Each agent's payloads make the events written for them, and its table names every fixture.
#[test]
fn every_documented_payload_normalizes_as_written() -> TestResult {
    let tables = [
        ("qwen", qwen()?),
        ("droid", droid()?),
        ("trae", trae()?),
        ("junie", junie()?),
        ("augment", augment()?),
        ("openhands", openhands()?),
        ("gemini", gemini()?),
        ("cursor", cursor()?),
        ("copilot", copilot()?),
        ("antigravity", antigravity()?),
        ("kiro", kiro()?),
        ("cline", cline()?),
        ("crush", crush()?),
        ("hermes", hermes()?),
    ];
    assert_eq!(std::env::var_os("CURSOR_PROJECT_DIR"), None);
    for (name, table) in tables {
        let adapter = adapter(name).ok_or(name)?;
        for (fixture, expected) in &table {
            let payload = read_fixture(name, DOCUMENTED, fixture)?;
            let event = event_of(name, fixture, &payload)?;
            let found = adapter.normalize(&event, &payload, &locate, stamp())?;
            assert_eq!(&found, expected, "{name} {fixture}");
        }
        let mut named: Vec<&str> = table.iter().map(|(fixture, _)| *fixture).collect();
        named.sort_unstable();
        assert_eq!(
            named,
            fixture_names(&fixtures_of(name, DOCUMENTED))?,
            "{name}"
        );
    }
    Ok(())
}

/// Devin's payloads name no working folder: with `DEVIN_PROJECT_DIR` unset, as here, none makes
/// an event. The binary's tests run Devin's payloads with the variable set.
#[test]
fn devin_payloads_need_the_project_folder_from_the_environment() -> TestResult {
    assert_eq!(std::env::var_os("DEVIN_PROJECT_DIR"), None);
    let devin = adapter("devin").ok_or("devin")?;
    for fixture in fixture_names(&fixtures_of("devin", DOCUMENTED))? {
        let payload = read_fixture("devin", DOCUMENTED, &fixture)?;
        let found = devin.normalize(&event_name(&payload)?, &payload, &locate, stamp())?;
        assert_eq!(found, None, "{fixture}");
    }
    Ok(())
}

/// The hooks each adapter registers: the events its agent's documentation gives, with the tools
/// it matches and the hard timeouts; text only where the documentation says the agent adds it to
/// the model's context; none of them names versions, since none was run live.
#[test]
fn the_hooks_are_the_ones_each_documentation_supports() -> TestResult {
    let giving = |name: &str| -> TestResult<Vec<&'static str>> {
        Ok(adapter(name)
            .ok_or("adapter")?
            .hooks()
            .iter()
            .filter(|entry| entry.gives_context)
            .map(|entry| entry.event_name)
            .collect())
    };
    let registered = |name: &str| -> TestResult<Vec<(&'static str, Option<&'static str>, u128)>> {
        Ok(adapter(name)
            .ok_or("adapter")?
            .hooks()
            .iter()
            .map(|entry| {
                (
                    entry.event_name,
                    entry.matcher,
                    entry.hard_timeout.as_millis(),
                )
            })
            .collect())
    };
    let qwen_tools = Some("^(read_file|write_file|edit|glob|grep_search|run_shell_command)$");
    assert_eq!(
        registered("qwen")?,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            ("PreToolUse", Some("^read_file$"), 300),
            ("PostToolUse", qwen_tools, 500),
            ("PostToolUseFailure", qwen_tools, 500),
            ("PreCompact", None, 300),
            ("Stop", None, 300),
            ("SubagentStart", None, 300),
            ("SubagentStop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert_eq!(
        giving("qwen")?,
        [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse"
        ]
    );
    assert_eq!(
        registered("droid")?,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            (
                "PostToolUse",
                Some("^(Read|Edit|Create|ApplyPatch|Glob|Grep|Execute)$"),
                500
            ),
            ("PreCompact", None, 300),
            ("Stop", None, 300),
            ("SubagentStop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert_eq!(
        giving("droid")?,
        ["SessionStart", "UserPromptSubmit", "PostToolUse"]
    );
    assert_eq!(
        registered("devin")?,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            (
                "PostToolUse",
                Some("^(read|edit|write|apply_patch|grep|glob|exec)$"),
                500
            ),
            ("Stop", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert_eq!(
        giving("devin")?,
        ["SessionStart", "UserPromptSubmit", "PostToolUse"]
    );
    assert_eq!(
        registered("trae")?,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000),
            ("PreToolUse", Some("^Read$"), 300),
            (
                "PostToolUse",
                Some("^(Read|Write|Edit|Glob|Grep|RunCommand)$"),
                500
            ),
            ("Stop", None, 300),
        ]
    );
    assert_eq!(
        giving("trae")?,
        [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse"
        ]
    );
    assert_eq!(
        registered("junie")?,
        [
            ("SessionStart", None, 1_500),
            ("UserPromptSubmit", None, 1_000)
        ]
    );
    assert_eq!(giving("junie")?, ["UserPromptSubmit"]);
    assert_eq!(giving("augment")?, ["SessionStart", "PostToolUse"]);
    assert_eq!(giving("openhands")?, ["SessionStart", "UserPromptSubmit"]);
    assert_eq!(
        giving("gemini")?,
        ["SessionStart", "BeforeAgent", "AfterTool"]
    );
    assert_eq!(giving("cursor")?, ["sessionStart", "postToolUse"]);
    assert_eq!(giving("copilot")?, ["SessionStart", "PostToolUse"]);
    assert_eq!(
        registered("antigravity")?,
        [
            ("SessionStart", None, 1_500),
            ("PreInvocation", None, 1_000),
            (
                "PostToolUse",
                Some(
                    "^(view_file|write_to_file|replace_file_content|multi_replace_file_content|grep_search|find_by_name|run_command)$"
                ),
                500
            ),
            ("Stop", None, 300),
        ]
    );
    assert_eq!(giving("antigravity")?, ["SessionStart", "PreInvocation"]);
    assert_eq!(giving("kiro")?, ["SessionStart", "UserPromptSubmit"]);
    assert_eq!(
        giving("cline")?,
        ["TaskStart", "TaskResume", "UserPromptSubmit", "PostToolUse"]
    );
    let crush_tools = Some("^(view|write|edit|multiedit|glob|grep|bash)$");
    assert_eq!(
        registered("crush")?,
        [
            ("SessionStart", None, 1_500),
            ("TurnStart", None, 1_000),
            ("PreToolUse", Some("^view$"), 300),
            ("PostToolUse", crush_tools, 500),
            ("PostToolUseFailure", crush_tools, 500),
            ("PreCompact", None, 300),
            ("TurnEnd", None, 300),
            ("SessionEnd", None, 300),
        ]
    );
    assert_eq!(
        giving("crush")?,
        ["PreToolUse", "PostToolUse", "PostToolUseFailure"]
    );
    assert_eq!(giving("hermes")?, ["pre_llm_call"]);
    for name in DOCUMENTED_AGENTS {
        let adapter = adapter(name).ok_or(*name)?;
        assert!(adapter.versions().is_none(), "{name}");
        assert_eq!(adapter.agent().name(), *name);
    }
    Ok(())
}

/// The keys of the payloads the proptest builds: every field the adapters read, and one they do
/// not.
const FIELD_NAMES: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "source",
    "model",
    "prompt",
    "submitted_prompt",
    "tool_name",
    "tool_input",
    "tool_response",
    "file_path",
    "path",
    "pattern",
    "offset",
    "limit",
    "command",
    "success",
    "working_dir",
    "message",
    "conversation_id",
    "user_prompt",
    "tool_is_error",
    "view_range",
    "start_line",
    "end_line",
    "error",
    "exit_code",
    "conversationId",
    "workspacePaths",
    "invocationNum",
    "lastUserInput",
    "toolCall",
    "name",
    "args",
    "AbsolutePath",
    "CommandLine",
    "operations",
    "mode",
    "taskId",
    "workspaceRoots",
    "userPromptSubmit",
    "postToolUse",
    "toolName",
    "parameters",
    "extra",
    "user_message",
    "reason",
    "unknown",
];

/// Strings that steer a payload down the adapters' branches: their tool names, prompts, paths
/// inside and outside the repository.
const STEERING_TEXTS: &[&str] = &[
    "read_file",
    "edit",
    "grep_search",
    "run_shell_command",
    "Create",
    "Execute",
    "exec",
    "terminal",
    "file_editor",
    "view",
    "bash",
    "RunCommand",
    "AfterTool",
    "Shell",
    "Read",
    "view",
    "read",
    "view_file",
    "run_command",
    "Line",
    "execute_command",
    "startup",
    "/repo/a.rs",
    r"C:\repo\a.rs",
    "/elsewhere/b.rs",
    "",
];

proptest! {
    /// Payloads of any shape, with a session in the repository or without one, normalize to an
    /// event that keeps to its hook's fields, to none, or to an error, for every documented
    /// adapter: never a panic.
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
        for name in DOCUMENTED_AGENTS {
            let adapter = adapter(name).ok_or_else(|| TestCaseError::fail("no adapter"))?;
            for entry in adapter.hooks() {
                if let Ok(Some(event)) = adapter.normalize(entry.event_name, &bytes, &locate, stamp()) {
                    assert_shape(entry.hook, &event);
                }
            }
        }
    }
}
