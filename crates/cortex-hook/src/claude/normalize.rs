//! Claude Code's payloads turned into normalized events.
//!
//! Only paths, line ranges, sizes, exit codes and the prompt travel on: the prompt is cut so that
//! any event fits a frame, a search pattern and a command are cut to 1,000 characters, and an
//! optional value that breaks its rules (a model name over 256 bytes, say) is left out rather than
//! losing the event. Paths are taken relative to the repository as the agent spells its root, then
//! as the root is spelled on disk; a path in neither is outside the repository and has no path.

use std::path::{Path, PathBuf};

use cortex_core::{
    Agent, CompactTrigger, Event, HookKind, Identifier, LineRange, MAX_COMMAND_CHARS, RepoPath,
    ShellCommand, StartSource, ToolKind, ToolResult, ToolTarget,
};

use super::payload::{Payload, ReadFile, ToolResponse};
use crate::adapter::{HookEntry, Located, NormalizeError, Stamp};

/// Most bytes of a prompt an event carries. JSON writes each byte in at most six, so an event with
/// a prompt this long, its other fields included, stays under a frame's mebibyte; a prompt longer
/// than a hundred thousand words is cut.
pub(crate) const MAX_PROMPT_BYTES: usize = 131_072;

/// The event `payload` makes for the hook `entry`, or `None` when its working folder is in no
/// repository Cortex serves.
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
    let session = payload
        .session_id
        .as_deref()
        .and_then(name)
        .ok_or(NormalizeError::NoSession)?;
    let subagent = payload.agent_id.as_deref().filter(|id| !id.is_empty());
    let (session_id, parent_session_id) = match subagent {
        Some(agent) => (
            Identifier::new(&format!("{}/{agent}", session.as_str()))
                .map_err(|_| NormalizeError::NoSession)?,
            Some(session),
        ),
        None => (session, None),
    };
    let transcript = match subagent {
        Some(_) => payload.agent_transcript_path.as_ref(),
        None => None,
    }
    .or(payload.transcript_path.as_ref());
    let tool = payload.tool_name.as_deref().map(tool_kind);
    let is_tool_event = matches!(entry.hook, HookKind::PreTool | HookKind::PostTool);
    Ok(Some(Event {
        event_id: stamp.event_id,
        ts_ms: stamp.ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id,
        parent_session_id,
        cwd: located.cwd.clone(),
        repo_root: located.root.clone(),
        hook: entry.hook,
        source: payload
            .source
            .as_deref()
            .filter(|_| entry.hook == HookKind::SessionStart)
            .and_then(StartSource::from_name),
        tool: tool.filter(|_| is_tool_event),
        tool_target: tool
            .filter(|_| is_tool_event)
            .and_then(|tool| target(tool, entry.hook, &payload, &located)),
        tool_result: tool
            .filter(|_| entry.hook == HookKind::PostTool)
            .map(|tool| result(tool, entry.event_name, &payload)),
        prompt_text: payload
            .prompt
            .as_deref()
            .filter(|_| entry.hook == HookKind::Prompt)
            .map(cut_prompt),
        transcript_path: transcript.map(PathBuf::from),
        model: payload.model.as_deref().and_then(name),
        subagent_type: payload.agent_type.as_deref().and_then(name),
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

/// `payload` without a UTF-8 byte-order mark in front.
fn strip_bom(payload: &[u8]) -> &[u8] {
    payload.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(payload)
}

/// `text` as a stored name, or `None` when it is not one.
fn name(text: &str) -> Option<Identifier> {
    Identifier::new(text).ok()
}

/// The kind of the tool Claude Code names `name`.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" => ToolKind::Read,
        "Grep" => ToolKind::Grep,
        "Glob" => ToolKind::Glob,
        "Edit" => ToolKind::Edit,
        "Write" => ToolKind::Write,
        "MultiEdit" => ToolKind::MultiEdit,
        "Bash" | "PowerShell" => ToolKind::Shell,
        _ => ToolKind::Other,
    }
}

/// What the tool aimed at: its file or folder inside the repository, the lines, the pattern, the
/// command. `None` for a tool with none of them.
fn target(
    tool: ToolKind,
    hook: HookKind,
    payload: &Payload,
    located: &Located,
) -> Option<ToolTarget> {
    let input = payload.tool_input.as_ref()?;
    let response = payload.tool_response.as_ref();
    let target = match tool {
        ToolKind::Read => ToolTarget {
            path: input
                .file_path
                .as_deref()
                .and_then(|path| relative(path, located)),
            ranges: match (hook, response.and_then(|response| response.file.as_ref())) {
                (HookKind::PostTool, Some(file)) => lines_read(file),
                _ => lines_asked(input.offset, input.limit),
            },
            ..ToolTarget::default()
        },
        ToolKind::Edit | ToolKind::Write | ToolKind::MultiEdit | ToolKind::ApplyPatch => {
            ToolTarget {
                path: input
                    .file_path
                    .as_deref()
                    .and_then(|path| relative(path, located)),
                ranges: response.map(lines_changed).unwrap_or_default(),
                ..ToolTarget::default()
            }
        }
        ToolKind::Grep | ToolKind::Glob => ToolTarget {
            path: input
                .path
                .as_deref()
                .and_then(|path| relative(path, located)),
            pattern: input
                .pattern
                .as_deref()
                .map(|pattern| pattern.chars().take(MAX_COMMAND_CHARS).collect()),
            ..ToolTarget::default()
        },
        ToolKind::Shell => ToolTarget {
            command: input.command.as_deref().map(ShellCommand::cut),
            ..ToolTarget::default()
        },
        ToolKind::Other => return None,
    };
    Some(target)
}

/// The repository path of `path`, under the root as the agent spells it or as it is on disk.
fn relative(path: &str, located: &Located) -> Option<RepoPath> {
    let path = Path::new(path);
    RepoPath::from_native(&located.agent_root, path)
        .or_else(|_| RepoPath::from_native(&located.root, path))
        .ok()
}

/// The lines a read returned: none when it returned the whole file or nothing.
fn lines_read(file: &ReadFile) -> Vec<LineRange> {
    let (Some(start), Some(count)) = (file.start_line, file.num_lines) else {
        return Vec::new();
    };
    let is_whole = start == 1 && file.total_lines.is_some_and(|total| count >= total);
    if is_whole {
        return Vec::new();
    }
    range(start, count).into_iter().collect()
}

/// The lines a read asked for: from `offset` (the first line, 1 when absent) for `limit` lines;
/// none when it asked for no limit.
fn lines_asked(offset: Option<i64>, limit: Option<i64>) -> Vec<LineRange> {
    match limit {
        Some(limit) => range(offset.unwrap_or(1), limit).into_iter().collect(),
        None => Vec::new(),
    }
}

/// The stretches of lines an edit or a write changed, in the file after it. A stretch that only
/// removed lines is the line where they were.
fn lines_changed(response: &ToolResponse) -> Vec<LineRange> {
    response
        .structured_patch
        .iter()
        .flatten()
        .filter_map(|hunk| range(hunk.new_start?.max(1), hunk.new_lines?.max(1)))
        .collect()
}

/// Lines `start` to `start + count - 1`, when they make a range.
fn range(start: i64, count: i64) -> Option<LineRange> {
    let first = u32::try_from(start).ok()?;
    let last = u32::try_from(start.checked_add(count)?.checked_sub(1)?).ok()?;
    LineRange::new(first, last).ok()
}

/// How the tool ended. A shell's result means exit code 0 (Claude Code reports a non-zero exit as
/// a failure); a failed shell's message begins `Exit code N`.
fn result(tool: ToolKind, event_name: &str, payload: &Payload) -> ToolResult {
    let response = payload.tool_response.as_ref();
    if event_name == "PostToolUseFailure" {
        let message = payload.error.as_deref().unwrap_or_default();
        return ToolResult {
            exit_code: exit_code(message).filter(|_| tool == ToolKind::Shell),
            is_success: false,
            is_interrupted: payload.is_interrupt.unwrap_or(false),
            output_bytes: payload
                .error
                .as_ref()
                .map(|error| u64::try_from(error.len()).unwrap_or(u64::MAX)),
        };
    }
    let printed = response.and_then(|response| {
        let stdout = response.stdout.map(|length| length.0);
        let stderr = response.stderr.map(|length| length.0);
        match (stdout, stderr) {
            (None, None) => None,
            (out, err) => Some(out.unwrap_or(0).saturating_add(err.unwrap_or(0))),
        }
    });
    ToolResult {
        exit_code: (tool == ToolKind::Shell).then_some(0),
        is_success: true,
        is_interrupted: response
            .and_then(|response| response.interrupted)
            .unwrap_or(false),
        output_bytes: printed,
    }
}

/// The exit code a failed shell's message starts with: `Exit code N` on its first line.
fn exit_code(message: &str) -> Option<i32> {
    message
        .lines()
        .next()?
        .strip_prefix("Exit code ")?
        .trim()
        .parse()
        .ok()
}

/// `prompt`, cut after its first [`MAX_PROMPT_BYTES`] bytes at a character boundary.
fn cut_prompt(prompt: &str) -> String {
    let end = prompt.floor_char_boundary(MAX_PROMPT_BYTES);
    prompt.get(..end).unwrap_or_default().to_owned()
}
