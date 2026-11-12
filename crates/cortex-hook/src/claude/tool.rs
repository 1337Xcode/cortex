//! What a Claude Code tool call aimed at and how it ended: the file or folder in the repository,
//! the lines read or changed, the search pattern and the files it found, or the command, the exit
//! code and the size of what it printed, and for a command that failed, what it printed.

use std::borrow::Cow;

use cortex_core::{
    ContentHash, HookKind, LineRange, MAX_COMMAND_CHARS, MAX_RANGES, RepoPath, ShellCommand,
    ToolKind, ToolResult, ToolTarget,
};

use crate::adapter::Located;
use crate::shaped::payload::{Hunk, LineMark, Payload, ReadFile, ToolInput, ToolResponse};
use crate::shaped::place::{Place, hits, place};
use crate::shaped::{Aim, Tools};

/// How Claude Code's tools read.
pub(super) struct ClaudeTools;

impl Tools for ClaudeTools {
    fn kind(&self, name: &str, _: &Payload<'_>) -> ToolKind {
        kind(name)
    }

    fn aim(&self, tool: ToolKind, hook: HookKind, payload: &Payload<'_>, located: &Located) -> Aim {
        aim(tool, hook, payload, located)
    }

    fn result(
        &self,
        tool: ToolKind,
        event_name: &str,
        payload: &Payload<'_>,
    ) -> Option<ToolResult> {
        Some(result(tool, event_name, payload))
    }

    fn failed_output<'p>(
        &self,
        event_name: &str,
        payload: &'p Payload<'_>,
    ) -> Option<Cow<'p, str>> {
        if event_name != "PostToolUseFailure" {
            return None;
        }
        let message = payload.error.as_ref()?.as_str()?;
        // The message opens with the shell's `Exit code N` line, which is Claude Code's, not the
        // command's.
        let output = match message.split_once('\n') {
            Some((first, rest)) if exit_code(first).is_some() => rest,
            None if exit_code(message).is_some() => "",
            _ => message,
        };
        Some(Cow::Borrowed(output))
    }
}

/// The kind of the tool Claude Code names `name`.
fn kind(name: &str) -> ToolKind {
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

/// What a call of `tool` at `hook` aimed at. Before a read runs its lines are the ones asked for;
/// after, the ones it returned, when it asked for a start or a limit: one that asked for neither
/// asked for the whole file, however much of it Claude Code returned, and keeps the last line
/// returned when Claude Code cut it short. A search without a path searched the working folder.
fn aim(tool: ToolKind, hook: HookKind, payload: &Payload<'_>, located: &Located) -> Aim {
    let Some(input) = payload.tool_input.as_ref() else {
        return Aim::Nothing;
    };
    let cwd = payload.cwd.as_deref();
    let response = payload.tool_response.as_ref();
    let target = match tool {
        ToolKind::Read => match place(input.file_path.as_deref(), cwd, located) {
            Place::Outside => return Aim::Outside,
            place => ToolTarget {
                path: place.path(),
                ranges: match hook {
                    HookKind::PostTool if input.offset.is_none() && input.limit.is_none() => {
                        Vec::new()
                    }
                    HookKind::PostTool => response
                        .and_then(|response| response.file.as_ref())
                        .map(lines_read)
                        .unwrap_or_default(),
                    _ => lines_asked(input),
                },
                cut_after: match hook {
                    HookKind::PostTool if input.offset.is_none() && input.limit.is_none() => {
                        response
                            .and_then(|response| response.file.as_ref())
                            .and_then(|file| lines_read(file).first().map(|lines| lines.end_line()))
                    }
                    _ => None,
                },
                ..ToolTarget::default()
            },
        },
        ToolKind::Edit | ToolKind::Write | ToolKind::MultiEdit | ToolKind::ApplyPatch => {
            match place(input.file_path.as_deref(), cwd, located) {
                Place::Outside => return Aim::Outside,
                place => ToolTarget {
                    path: place.path(),
                    ranges: response.map(lines_changed).unwrap_or_default(),
                    replaced: response.map(lines_replaced).unwrap_or_default(),
                    // INVARIANT: no-content-stored (the file before the edit leaves the hook as
                    // its hash only)
                    before_hash: response
                        .and_then(|response| response.original_file.as_deref())
                        .map(|text| ContentHash::of(text.as_bytes())),
                    ..ToolTarget::default()
                },
            }
        }
        ToolKind::Grep | ToolKind::Glob => match place(input.path.as_deref(), cwd, located) {
            Place::Outside => return Aim::Outside,
            place => ToolTarget {
                path: place
                    .path()
                    .or_else(|| RepoPath::from_native(&located.root, &located.cwd).ok()),
                pattern: input
                    .pattern
                    .as_deref()
                    .map(|pattern| pattern.chars().take(MAX_COMMAND_CHARS).collect()),
                // Before a search runs it has found nothing.
                hits: match (hook, response) {
                    (HookKind::PostTool, Some(response)) => hits(response, cwd, located),
                    _ => Vec::new(),
                },
                ..ToolTarget::default()
            },
        },
        ToolKind::Shell => ToolTarget {
            command: input.command.as_deref().map(ShellCommand::cut),
            ..ToolTarget::default()
        },
        ToolKind::Other => return Aim::Nothing,
    };
    Aim::Target(Box::new(target))
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

/// The lines a read asks for: from `offset` (the first line, 1 when absent) for `limit` lines;
/// none when it asks for no limit, since how far it reads is then Claude Code's to decide.
fn lines_asked(input: &ToolInput) -> Vec<LineRange> {
    match input.limit {
        Some(limit) => range(input.offset.unwrap_or(1), limit)
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

/// The stretches of lines an edit or a write changed, one per hunk of its diff, in the file after
/// it; none for a new file written whole, and none, the whole file, past [`MAX_RANGES`] stretches.
fn lines_changed(response: &ToolResponse) -> Vec<LineRange> {
    let ranges: Vec<LineRange> = response
        .structured_patch
        .iter()
        .flatten()
        .filter_map(changed_in)
        .collect();
    if ranges.len() > MAX_RANGES {
        return Vec::new();
    }
    ranges
}

/// The stretches of lines an edit or a write replaced, one per hunk of its diff that removed lines,
/// numbered in the file before it; none past [`MAX_RANGES`] hunks, as for the lines it changed.
fn lines_replaced(response: &ToolResponse) -> Vec<LineRange> {
    let hunks = response.structured_patch.as_deref().unwrap_or_default();
    if hunks.len() > MAX_RANGES {
        return Vec::new();
    }
    hunks.iter().filter_map(replaced_in).collect()
}

/// The lines one hunk removed, numbered in the file before the edit: from its first removed line
/// to its last, the unchanged lines between them included. None for a hunk that removed nothing,
/// and for one without its first line before the edit or its lines, which cannot be told apart.
fn replaced_in(hunk: &Hunk) -> Option<LineRange> {
    let mut line = hunk.old_start?;
    let mut removed: Option<(i64, i64)> = None;
    for mark in hunk.lines.as_deref()? {
        match mark {
            LineMark::Unchanged => line = line.saturating_add(1),
            LineMark::Removed => {
                removed = Some(removed.map_or((line, line), |(first, _)| (first, line)));
                line = line.saturating_add(1);
            }
            // An added line is not in the file before the edit.
            LineMark::Added | LineMark::Other => {}
        }
    }
    let (first, last) = removed?;
    LineRange::new(u32::try_from(first).ok()?, u32::try_from(last).ok()?).ok()
}

/// The lines one hunk changed: from its first changed line to its last, the unchanged lines around
/// them left out. Lines added are where they now stand; lines removed with none added in their
/// place are the line before the gap they left. A hunk without its lines counts whole.
fn changed_in(hunk: &Hunk) -> Option<LineRange> {
    let start = hunk.new_start?.max(1);
    let Some(marks) = hunk.lines.as_deref() else {
        return range(start, hunk.new_lines?.max(1));
    };
    let mut line = start;
    let mut changed: Option<(i64, i64)> = None;
    let mut widen = |at: i64| {
        changed = Some(changed.map_or((at, at), |(first, last)| (first.min(at), last.max(at))));
    };
    // Whether the run of changes since the last unchanged line removed lines. Where it added some
    // too, the line before the gap is the last one added, already counted.
    let mut is_removing = false;
    // An unchanged line after the last mark closes a removal at the end of the hunk.
    for mark in marks.iter().chain([&LineMark::Unchanged]) {
        match mark {
            LineMark::Added => {
                widen(line);
                line = line.saturating_add(1);
            }
            LineMark::Removed => is_removing = true,
            LineMark::Unchanged => {
                if is_removing {
                    widen(line.saturating_sub(1).max(1));
                    is_removing = false;
                }
                line = line.saturating_add(1);
            }
            LineMark::Other => {}
        }
    }
    let (first, last) = changed?;
    LineRange::new(u32::try_from(first).ok()?, u32::try_from(last).ok()?).ok()
}

/// Lines `start` to `start + count - 1`, when they make a range.
fn range(start: i64, count: i64) -> Option<LineRange> {
    let first = u32::try_from(start).ok()?;
    let last = u32::try_from(start.checked_add(count)?.checked_sub(1)?).ok()?;
    LineRange::new(first, last).ok()
}

/// How a call of `tool`, reported as `event_name`, ended.
fn result(tool: ToolKind, event_name: &str, payload: &Payload<'_>) -> ToolResult {
    if event_name == "PostToolUseFailure" {
        let message = payload.error.as_ref().and_then(serde_json::Value::as_str);
        return ToolResult {
            exit_code: message
                .and_then(exit_code)
                .filter(|_| tool == ToolKind::Shell),
            is_success: false,
            is_interrupted: payload.is_interrupt.unwrap_or(false),
            output_bytes: message.map(|message| u64::try_from(message.len()).unwrap_or(u64::MAX)),
            counts: None,
            failing: None,
        };
    }
    let response = payload.tool_response.as_ref();
    let is_interrupted = response
        .and_then(|response| response.interrupted)
        .unwrap_or(false);
    let is_background = payload
        .tool_input
        .as_ref()
        .and_then(|input| input.run_in_background)
        .unwrap_or(false);
    ToolResult {
        // Claude Code reports a shell that exits non-zero as a failure, so a shell's result means
        // exit code 0, unless it was interrupted or runs on in the background: then no code is
        // known yet.
        exit_code: (tool == ToolKind::Shell && !is_interrupted && !is_background).then_some(0),
        is_success: true,
        is_interrupted,
        output_bytes: response.and_then(printed),
        counts: None,
        failing: None,
    }
}

/// The bytes a shell printed on both its streams, when it reports either.
fn printed(response: &ToolResponse) -> Option<u64> {
    let stdout = response.stdout.map(|length| length.0);
    let stderr = response.stderr.map(|length| length.0);
    match (stdout, stderr) {
        (None, None) => None,
        (out, err) => Some(out.unwrap_or(0).saturating_add(err.unwrap_or(0))),
    }
}

/// The exit code a failed shell's message starts with: `Exit code N` on its first line. A code
/// past `i32::MAX`, as Windows reports some, is read as the 32 bits it is: `4294967295` is -1.
fn exit_code(message: &str) -> Option<i32> {
    let code: i64 = message
        .lines()
        .next()?
        .strip_prefix("Exit code ")?
        .trim()
        .parse()
        .ok()?;
    i32::try_from(code).ok().or_else(|| {
        u32::try_from(code)
            .ok()
            .map(|code| i32::from_ne_bytes(code.to_ne_bytes()))
    })
}
