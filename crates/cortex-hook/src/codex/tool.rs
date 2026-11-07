//! What a Codex tool call aimed at and how it ended. Codex reads and searches through its shell
//! (tool `Bash`), whose command is read for what it does (see `command`); a search's hits are the
//! files its output names. Edits arrive as `apply_patch`, the whole patch in `tool_input.command`:
//! its first `*** Update File:`, `*** Add File:` or `*** Delete File:` header names the file the
//! call aimed at, and its hunks carry no line numbers. A shell's exit code is not in the payload
//! but at the end of the session's rollout file (see `rollout`); a patch's response opens with
//! its own.

use cortex_core::{
    HookKind, MAX_COMMAND_CHARS, RepoPath, ShellCommand, ToolKind, ToolResult, ToolTarget,
};
use serde_json::value::RawValue;

use super::command::{Step, step};
use super::rollout;
use crate::adapter::Located;
use crate::shaped::payload::Payload;
use crate::shaped::place::{Place, hits, place};
use crate::shaped::{Aim, Tools};

/// The headers of an `apply_patch` that name a file it changes: the first of them is the file the
/// call aimed at. A moved file's new name (`*** Move to:`) follows its own header.
const PATCH_HEADERS: [&str; 3] = ["*** Update File: ", "*** Add File: ", "*** Delete File: "];

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
            (ToolKind::ApplyPatch, _) => match first_patched(command) {
                None => return Aim::Nothing,
                Some(path) => match place(Some(path), cwd, located) {
                    Place::Outside => return Aim::Outside,
                    place => ToolTarget {
                        path: place.path(),
                        ..ToolTarget::default()
                    },
                },
            },
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
        })
    }
}

/// The command a call ran, or the patch it applied.
fn command<'a>(payload: &'a Payload<'_>) -> Option<&'a str> {
    payload.tool_input.as_ref()?.command.as_deref()
}

/// The file the first header of the patch `text` names, when one names a file.
fn first_patched(text: &str) -> Option<&str> {
    text.lines()
        .filter_map(|line| {
            PATCH_HEADERS
                .iter()
                .find_map(|header| line.strip_prefix(header))
        })
        .map(str::trim)
        .find(|path| !path.is_empty())
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

    /// A patch's target is the file its first header names, whichever kind of header it is; a
    /// header with no name, a line that only holds a header's words, and a `Move to` line name
    /// none.
    #[test]
    fn a_patch_aims_at_the_file_its_first_header_names() {
        let cases = [
            (
                "*** Begin Patch\n*** Update File: calc/ops.py\n@@\n-a\n+b\n*** End Patch",
                Some("calc/ops.py"),
            ),
            (
                "*** Begin Patch\n*** Add File: NOTES.md\n+x\n*** Update File: a.py\n",
                Some("NOTES.md"),
            ),
            (
                "*** Begin Patch\n*** Delete File: old.py\n*** End Patch",
                Some("old.py"),
            ),
            (
                "*** Begin Patch\n*** Update File:  \n*** Add File: b.py\n",
                Some("b.py"),
            ),
            (
                "*** Begin Patch\n@@\n- *** Add File: no.py\n*** Move to: c.py\n*** End Patch",
                None,
            ),
            ("*** Begin Patch\n*** End Patch", None),
        ];
        for (patch, expected) in cases {
            assert_eq!(first_patched(patch), expected, "{patch}");
        }
    }

    /// A patch of a million bytes of headers is read for its first file at once, not every header.
    #[test]
    fn a_long_patch_costs_one_header() {
        let headers: Vec<String> = (0..50_000)
            .map(|number| format!("*** Add File: f{number}.py\n"))
            .collect();
        let patch = format!("*** Begin Patch\n{}", headers.concat());
        let started = std::time::Instant::now();
        assert_eq!(first_patched(&patch), Some("f0.py"));
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
