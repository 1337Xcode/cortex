//! Claude Code's payloads turned into normalized events.
//!
//! Only paths, line ranges, sizes, exit codes and the prompt travel on: the prompt is cut so that
//! any event fits a frame, a search pattern and a command are cut to 1,000 characters, and an
//! optional value that breaks its rules (a model name over 256 bytes, say) is left out rather than
//! losing the event. A field is carried only on the hooks it belongs to.

use std::path::{Path, PathBuf};

use cortex_core::{Agent, CompactTrigger, Event, HookKind, Identifier, StartSource};

use super::payload::Payload;
use super::tool::{self, Aim};
use crate::adapter::{HookEntry, Located, NormalizeError, Stamp};

/// Most bytes of a prompt an event carries. JSON writes each byte in at most six, so an event with
/// a prompt this long, its other fields included, stays under a frame's mebibyte; a prompt longer
/// than a hundred thousand words is cut.
pub(crate) const MAX_PROMPT_BYTES: usize = 131_072;

/// The event `payload` makes for the hook `entry`, or `None` when its working folder is in no
/// repository Cortex serves or its tool aimed at a file or folder outside the repository.
pub(super) fn event(
    entry: &HookEntry,
    payload: &[u8],
    locate: &dyn Fn(&Path) -> Option<Located>,
    stamp: Stamp,
) -> Result<Option<Event>, NormalizeError> {
    let payload: Payload = serde_json::from_slice(strip_bom(payload))?;
    let Some(located) = payload
        .cwd
        .as_deref()
        .and_then(|cwd| locate(Path::new(cwd)))
    else {
        return Ok(None);
    };
    let session = Session::of(&payload)?;
    let is_tool_event = matches!(entry.hook, HookKind::PreTool | HookKind::PostTool);
    let tool = payload
        .tool_name
        .as_deref()
        .map(tool::kind)
        .filter(|_| is_tool_event);
    let tool_target = match tool.map(|tool| tool::aim(tool, entry.hook, &payload, &located)) {
        Some(Aim::Outside) => return Ok(None),
        Some(Aim::Target(target)) => Some(target),
        Some(Aim::Nothing) | None => None,
    };
    Ok(Some(Event {
        event_id: stamp.event_id,
        ts_ms: stamp.ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: session.id,
        parent_session_id: session.parent,
        cwd: located.cwd,
        repo_root: located.root,
        hook: entry.hook,
        source: payload
            .source
            .as_deref()
            .filter(|_| entry.hook == HookKind::SessionStart)
            .and_then(StartSource::from_name),
        tool,
        tool_target,
        tool_result: tool
            .filter(|_| entry.hook == HookKind::PostTool)
            .map(|tool| tool::result(tool, entry.event_name, &payload)),
        prompt_text: payload
            .prompt
            .as_deref()
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
        end_reason: payload
            .reason
            .as_deref()
            .filter(|_| entry.hook == HookKind::SessionEnd)
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
    fn of(payload: &'a Payload) -> Result<Self, NormalizeError> {
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
