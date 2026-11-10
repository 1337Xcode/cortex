//! What a Codex tool call aimed at and how it ended. Codex reads and searches through its shell
//! (tool `Bash`), whose command is read for what it does (see `command`); a search's hits are the
//! files its output names. Edits arrive as `apply_patch`, the whole patch in `tool_input.command`:
//! its `*** Update File:`, `*** Add File:`, `*** Delete File:` and `*** Move to:` headers name the
//! files it changes, and its hunks carry no line numbers. A shell's exit code is not in the payload
//! but at the end of the session's rollout file (see `rollout`); a patch's response opens with
//! its own. A shell that failed gives what it printed, its response.

use std::borrow::Cow;

use cortex_core::{
    HookKind, MAX_COMMAND_CHARS, MAX_EDITED_FILES, RepoPath, ShellCommand, ToolKind, ToolResult,
    ToolTarget,
};
use serde_json::value::RawValue;

use super::command::{Step, step};
use super::rollout;
use crate::adapter::Located;
use crate::shaped::payload::Payload;
use crate::shaped::place::{Place, hits, place};
use crate::shaped::{Aim, Tools};

/// The headers of an `apply_patch` that name a file it changes; a moved file's new name
/// (`*** Move to:`) follows its old name's header.
const PATCH_HEADERS: [&str; 4] = [
    "*** Update File: ",
    "*** Add File: ",
    "*** Delete File: ",
    "*** Move to: ",
];

/// How Codex's tools read.
pub(super) struct CodexTools;

impl Tools for CodexTools {
    fn kind(&self, name: &str, payload: &Payload<'_>) -> ToolKind {
        match name {
            "apply_patch" => ToolKind::ApplyPatch,
            "Bash" => match command(payload).map(step) {
                Some(Step::Read { .. }) => ToolKind::Read,
                Some(Step::Search { .. }) => ToolKind::Grep,
                Some(Step::Other) | None => ToolKind::Shell,
            },
            _ => ToolKind::Other,
        }
    }

    fn aim(&self, tool: ToolKind, hook: HookKind, payload: &Payload<'_>, located: &Located) -> Aim {
        let Some(command) = command(payload) else {
            return Aim::Nothing;
        };
        let cwd = payload.cwd.as_deref();
        let target = match (tool, step(command)) {
            (ToolKind::Read, Step::Read { path, lines }) => match place(Some(path), cwd, located) {
                Place::Outside => return Aim::Outside,
                place => ToolTarget {
                    path: place.path(),
                    ranges: lines.into_iter().collect(),
                    ..ToolTarget::default()
                },
            },
            (ToolKind::Grep, Step::Search { pattern, path }) => match place(path, cwd, located) {
                Place::Outside => return Aim::Outside,
                place => ToolTarget {
                    path: place
                        .path()
                        .or_else(|| RepoPath::from_native(&located.root, &located.cwd).ok()),
                    pattern: Some(pattern.chars().take(MAX_COMMAND_CHARS).collect()),
                    // Before a search runs it has found nothing.
                    hits: match (hook, payload.tool_response.as_ref()) {
                        (HookKind::PostTool, Some(response)) => hits(response, cwd, located),
                        _ => Vec::new(),
                    },
                    ..ToolTarget::default()
                },
            },
            (ToolKind::ApplyPatch, _) => return patch_aim(command, cwd, located),
            (ToolKind::Shell, _) => ToolTarget {
                command: Some(ShellCommand::cut(command)),
                ..ToolTarget::default()
            },
            _ => return Aim::Nothing,
        };
        Aim::Target(target)
    }

    fn result(&self, tool: ToolKind, _: &str, payload: &Payload<'_>) -> Option<ToolResult> {
        // A patch's response opens with its exit code. A shell's is in the rollout file only,
        // written before the hook runs; a search's says too little (no match is exit 1), and is
        // not read.
        let code = match tool {
            ToolKind::ApplyPatch => payload
                .tool_response
                .as_ref()?
                .content
                .and_then(patch_exit_code)?,
            ToolKind::Shell | ToolKind::Read => rollout::exit_code(
                payload.transcript_path.as_deref()?,
                payload.tool_use_id.as_deref()?,
            )?,
            _ => return None,
        };
        Some(ToolResult {
            exit_code: Some(code),
            is_success: code == 0,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
        })
    }

    fn failed_output<'p>(&self, _: &str, payload: &'p Payload<'_>) -> Option<Cow<'p, str>> {
        // A shell's response is the output the model gets, as one JSON string.
        let raw = payload.tool_response.as_ref()?.content?;
        serde_json::from_str::<String>(raw.get())
            .ok()
            .map(Cow::Owned)
    }
}

/// The command a call ran, or the patch it applied.
fn command<'a>(payload: &'a Payload<'_>) -> Option<&'a str> {
    payload.tool_input.as_ref()?.command.as_deref()
}

/// What the patch `text` aimed at: the files its headers name inside the repository, the first
/// as the target's path and the others after it; outside, when every file it names is outside
/// the repository; nothing, when it names none.
fn patch_aim(text: &str, cwd: Option<&str>, located: &Located) -> Aim {
    let mut inside: Vec<RepoPath> = Vec::new();
    let mut is_outside = false;
    for named in patched_files(text) {
        match place(Some(named), cwd, located) {
            Place::Inside(path) if !inside.contains(&path) => inside.push(path),
            Place::Outside => is_outside = true,
            Place::Inside(_) | Place::Unnamed => {}
        }
    }
    if inside.is_empty() {
        return if is_outside {
            Aim::Outside
        } else {
            Aim::Nothing
        };
    }
    let more_paths = inside.split_off(1);
    Aim::Target(ToolTarget {
        path: inside.pop(),
        more_paths,
        ..ToolTarget::default()
    })
}

/// The files the headers of the patch `text` name, each once, in order: at most
/// [`MAX_EDITED_FILES`] of them, the rest of the patch not read.
fn patched_files(text: &str) -> Vec<&str> {
    let mut files: Vec<&str> = Vec::new();
    let named = text.lines().filter_map(|line| {
        PATCH_HEADERS
            .iter()
            .find_map(|header| line.strip_prefix(header))
            .map(str::trim)
    });
    for path in named.filter(|path| !path.is_empty()) {
        if files.len() == MAX_EDITED_FILES {
            break;
        }
        if !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// The exit code a patch's response opens with: `Exit code: N` on its first line.
fn patch_exit_code(response: &RawValue) -> Option<i32> {
    let text: String = serde_json::from_str(response.get()).ok()?;
    text.lines()
        .next()?
        .strip_prefix("Exit code: ")?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch names each file its headers name once, in order, a moved file under its new name
    /// too; a header with no name, and a line that only holds a header's words, name none.
    #[test]
    fn a_patch_names_the_files_its_headers_name() {
        let patch = "*** Begin Patch\n*** Update File: calc/ops.py\n@@\n-a\n+b\n\
                     *** Add File: NOTES.md\n+x\n*** Update File:  \n*** Update File: old.py\n\
                     *** Move to: new.py\n*** Delete File: gone.py\n\
                     *** Update File: calc/ops.py\n@@\n- *** Add File: no.py\n*** End Patch";
        assert_eq!(
            patched_files(patch),
            ["calc/ops.py", "NOTES.md", "old.py", "new.py", "gone.py"]
        );
        assert_eq!(
            patched_files("*** Begin Patch\n*** End Patch"),
            Vec::<&str>::new()
        );
    }

    /// A patch of a million bytes of headers is read for its first 32 files, not every header.
    #[test]
    fn a_long_patch_is_read_for_its_first_files() {
        let headers: Vec<String> = (0..50_000)
            .map(|number| format!("*** Add File: f{number}.py\n"))
            .collect();
        let patch = format!("*** Begin Patch\n{}", headers.concat());
        let started = std::time::Instant::now();
        let files = patched_files(&patch);
        assert_eq!(files.len(), 32);
        assert_eq!(
            (files.first(), files.last()),
            (Some(&"f0.py"), Some(&"f31.py"))
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    /// A patch's response opens with its exit code; a response without one gives none.
    #[test]
    fn a_patch_s_exit_code_is_read_from_its_response() {
        let code = |text: &str| {
            let raw = serde_json::value::to_raw_value(text).ok()?;
            patch_exit_code(&raw)
        };
        assert_eq!(
            code("Exit code: 0\nWall time: 0 seconds\nOutput:\n"),
            Some(0)
        );
        assert_eq!(code("Exit code: 1\n"), Some(1));
        assert_eq!(code("Success."), None);
        assert_eq!(code(""), None);
    }
}
