//! What a Claude Code tool call aimed at and how it ended: the file or folder in the repository,
//! the lines read or changed, the search pattern or the command, the exit code and the size of
//! what it printed.

use std::path::Path;

use cortex_core::{
    HookKind, LineRange, MAX_COMMAND_CHARS, MAX_RANGES, RepoPath, ShellCommand, ToolKind,
    ToolResult, ToolTarget,
};

use super::payload::{Hunk, LineMark, Payload, ReadFile, ToolInput, ToolResponse};
use crate::adapter::Located;

/// What a tool call aimed at.
pub(super) enum Aim {
    /// Nothing Cortex records: a tool of another kind, or a call with no input.
    Nothing,
    /// A target inside the repository, or one with no file at all, such as a shell's command.
    Target(ToolTarget),
    /// A file or folder outside the repository: the call is none of the repository's business.
    Outside,
}

/// Where a file or folder a tool names lies.
enum Place {
    /// The tool names none.
    Unnamed,
    /// Inside the repository, at this path.
    Inside(RepoPath),
    /// Outside the repository.
    Outside,
}

/// The kind of the tool Claude Code names `name`.
pub(super) fn kind(name: &str) -> ToolKind {
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
/// after, the ones it returned. A search without a path searched the working folder.
pub(super) fn aim(tool: ToolKind, hook: HookKind, payload: &Payload, located: &Located) -> Aim {
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
                    HookKind::PostTool => response
                        .and_then(|response| response.file.as_ref())
                        .map(lines_read)
                        .unwrap_or_default(),
                    _ => lines_asked(input),
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
                ..ToolTarget::default()
            },
        },
        ToolKind::Shell => ToolTarget {
            command: input.command.as_deref().map(ShellCommand::cut),
            ..ToolTarget::default()
        },
        ToolKind::Other => return Aim::Nothing,
    };
    Aim::Target(target)
}

impl Place {
    /// The repository path, when the place is one inside the repository.
    fn path(self) -> Option<RepoPath> {
        match self {
            Self::Inside(path) => Some(path),
            Self::Unnamed | Self::Outside => None,
        }
    }
}

/// Where `path` lies: a relative path is the agent's working folder `cwd` joined with it, then the
/// whole is taken relative to the repository's root as the agent spells it, else as it is on
/// disk.
fn place(path: Option<&str>, cwd: Option<&str>, located: &Located) -> Place {
    let Some(path) = path else {
        return Place::Unnamed;
    };
    let path = Path::new(path);
    let full = match cwd {
        Some(cwd) if path.is_relative() => Path::new(cwd).join(path),
        _ => path.to_path_buf(),
    };
    RepoPath::from_native(&located.agent_root, &full)
        .or_else(|_| RepoPath::from_native(&located.root, &full))
        .map_or(Place::Outside, Place::Inside)
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
pub(super) fn result(tool: ToolKind, event_name: &str, payload: &Payload) -> ToolResult {
    if event_name == "PostToolUseFailure" {
        let message = payload.error.as_deref();
        return ToolResult {
            exit_code: message
                .and_then(exit_code)
                .filter(|_| tool == ToolKind::Shell),
            is_success: false,
            is_interrupted: payload.is_interrupt.unwrap_or(false),
            output_bytes: message.map(|message| u64::try_from(message.len()).unwrap_or(u64::MAX)),
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
