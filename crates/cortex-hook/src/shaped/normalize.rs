//! A shaped agent's payloads turned into normalized events.
//!
//! Only paths, line ranges, sizes, exit codes, the prompt and the end of a failed shell command's
//! output travel on: the prompt is cut so that any event fits a frame, the output to its last
//! 64 KiB, a search pattern and a command are cut to 1,000 characters, and an
//! optional value that breaks its rules (a model name over 256 bytes, say) is left out rather than
//! losing the event. A field is carried only on the hooks it belongs to.

use std::path::{Path, PathBuf};

use cortex_core::{
    Agent, CompactTrigger, Event, FailedOutput, HookKind, Identifier, StartSource, ToolKind,
};

use super::payload::Payload;
use super::{Aim, Prompted, Tools};
use crate::adapter::{HookEntry, Located, NormalizeError, Stamp};

/// Most bytes of a prompt an event carries. JSON writes each byte in at most six, so an event with
/// a prompt this long, its other fields included, stays under a frame's mebibyte; a prompt longer
/// than a hundred thousand words is cut.
pub(crate) const MAX_PROMPT_BYTES: usize = 131_072;

/// The event `payload` makes for the hook `entry` of `agent`, whose tools read as `tools` says, or
/// `None` when its working folder is in no repository Cortex serves or its tool aimed at a file or
/// folder outside the repository.
pub(crate) fn event(
    (agent, tools, entry): (Agent, &dyn Tools, &HookEntry),
    payload: &[u8],
    locate: &dyn Fn(&Path) -> Option<Located>,
    stamp: Stamp,
) -> Result<Option<Event>, NormalizeError> {
    let renamed = tools.renamed(strip_bom(payload));
    let payload: Payload<'_> = serde_json::from_slice(&renamed)?;
    let Some(located) = tools.working_folder(&payload).and_then(|cwd| locate(&cwd)) else {
        return Ok(None);
    };
    let prompt = match tools.prompt(&payload) {
        Prompted::NotUser if entry.hook == HookKind::Prompt => return Ok(None),
        Prompted::User(prompt) => prompt,
        Prompted::NotUser => None,
    };
    let session = Session::of(&payload)?;
    let is_tool_event = matches!(entry.hook, HookKind::PreTool | HookKind::PostTool);
    let tool = payload
        .tool_name
        .as_deref()
        .map(|name| tools.kind(name, &payload))
        .filter(|_| is_tool_event);
    let mut tool_target = match tool.map(|tool| tools.aim(tool, entry.hook, &payload, &located)) {
        Some(Aim::Outside) => return Ok(None),
        Some(Aim::Target(target)) => Some(*target),
        Some(Aim::Nothing) | None => None,
    };
    let tool_result = tool
        .filter(|_| entry.hook == HookKind::PostTool)
        .and_then(|tool| tools.result(tool, entry.event_name, &payload));
    if let Some(target) = tool_target.as_mut()
        && tool == Some(ToolKind::Shell)
        && tool_result.is_some_and(|result| !result.is_success && !result.is_interrupted)
    {
        target.failed_output = tools
            .failed_output(entry.event_name, &payload)
            .and_then(|text| FailedOutput::tail_of(&text));
    }
    // A linked working tree's event goes to its repository's main tree, with the paths it holds
    // relative to its own top.
    let (repo_root, worktree_root) = match located.main_root {
        Some(main_root) => (main_root, Some(located.root)),
        None => (located.root, None),
    };
    Ok(Some(Event {
        event_id: stamp.event_id,
        ts_ms: stamp.ts_ms,
        agent,
        agent_version: None,
        session_id: session.id,
        parent_session_id: session.parent,
        cwd: located.cwd,
        repo_root,
        worktree_root,
        hook: entry.hook,
        source: payload
            .source
            .as_deref()
            .filter(|_| entry.hook == HookKind::SessionStart)
            .and_then(StartSource::from_name),
        tool,
        tool_target,
        tool_result,
        prompt_text: prompt
            .filter(|_| entry.hook == HookKind::Prompt)
            .map(cut_prompt),
        transcript_path: session.transcript.map(PathBuf::from),
        model: payload.model.as_deref().and_then(name),
        subagent_type: session.subagent_type,
        compact_trigger: payload
            .trigger
            .as_deref()
            .filter(|_| entry.hook == HookKind::Compact)
            .and_then(CompactTrigger::from_name),
        is_probe: payload.cortex_probe == Some(true),
        end_reason: match entry.hook {
            HookKind::SessionEnd => payload.reason.as_deref(),
            // A turn that ended on an error names it (Claude Code's `StopFailure`).
            HookKind::Stop => payload.error.as_ref().and_then(serde_json::Value::as_str),
            _ => None,
        }
        .and_then(name),
    }))
}

/// The session an event belongs to. A subagent's events belong to a session of its own,
/// `<session>/<agent>`, whose parent is the session that started it; its transcript is the
/// subagent's own, unknown when the payload does not give it, never the parent's.
struct Session<'a> {
    /// The session's id.
    id: Identifier,
    /// The session that started it, for a subagent.
    parent: Option<Identifier>,
    /// Its transcript.
    transcript: Option<&'a str>,
    /// The subagent's type, for a subagent.
    subagent_type: Option<Identifier>,
}

impl<'a> Session<'a> {
    /// The session of `payload`.
    ///
    /// # Errors
    ///
    /// [`NormalizeError::NoSession`] when the payload names no session, or its id and the
    /// subagent's together are no name.
    fn of(payload: &'a Payload<'_>) -> Result<Self, NormalizeError> {
        let session = payload
            .session_id
            .as_deref()
            .and_then(name)
            .ok_or(NormalizeError::NoSession)?;
        let Some(agent) = payload.agent_id.as_deref().filter(|id| !id.is_empty()) else {
            return Ok(Self {
                id: session,
                parent: None,
                transcript: payload.transcript_path.as_deref(),
                subagent_type: None,
            });
        };
        Ok(Self {
            id: Identifier::new(&format!("{}/{agent}", session.as_str()))
                .map_err(|_| NormalizeError::NoSession)?,
            parent: Some(session),
            transcript: payload.agent_transcript_path.as_deref(),
            subagent_type: payload.agent_type.as_deref().and_then(name),
        })
    }
}

/// `payload` without a UTF-8 byte-order mark in front.
fn strip_bom(payload: &[u8]) -> &[u8] {
    payload.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(payload)
}

/// `text` as a stored name, or `None` when it is not one.
fn name(text: &str) -> Option<Identifier> {
    Identifier::new(text).ok()
}

/// `prompt`, cut after its first [`MAX_PROMPT_BYTES`] bytes at a character boundary.
fn cut_prompt(prompt: &str) -> String {
    let end = prompt.floor_char_boundary(MAX_PROMPT_BYTES);
    prompt.get(..end).unwrap_or_default().to_owned()
}
