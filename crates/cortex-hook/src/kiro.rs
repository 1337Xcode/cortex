//! The Kiro adapter, for its editor and its `kiro-cli` command-line agent, built from Kiro's hooks
//! and tools documentation (read 2026-11-06) and not yet run live. Kiro's command hooks speak
//! Claude Code's hook JSON. Its tools' inputs are documented for the older command-line agent's
//! `read` only, whose operations each name a mode and a path: the first operation is read, and a
//! read of lines (mode `Line`) is the only one recorded as a read. Its current tools' inputs are
//! not documented, so a field under another name than Claude Code's is not read. A call's result
//! is its response's `success`. Text goes back as the hook's plain output, which Kiro adds to the
//! agent's context at a session's start and at a prompt only.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The most characters Kiro is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Kiro's tools Cortex reads, by their names; the older agent's `read` by the mode of its first
/// operation, which moves to the input's `command` with the operation's path and lines.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read_file", ToolKind::Read),
        ("fs_write", ToolKind::Write),
        ("fs_append", ToolKind::Edit),
        ("str_replace", ToolKind::Edit),
        ("grep_search", ToolKind::Grep),
        ("file_search", ToolKind::Glob),
        ("execute_bash", ToolKind::Shell),
        ("execute_pwsh", ToolKind::Shell),
    ],
    subcommands: &[("read", "Line", ToolKind::Read)],
    renames: &[
        ("/tool_input/operations/0/mode", "/tool_input/command"),
        ("/tool_input/operations/0/path", "/tool_input/path"),
        (
            "/tool_input/operations/0/start_line",
            "/tool_input/start_line",
        ),
        ("/tool_input/operations/0/end_line", "/tool_input/end_line"),
    ],
    outcome: Outcome::ResponseSuccess,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Kiro.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Kiro;

/// The hooks Cortex registers with Kiro, by its triggers' names.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "UserPromptSubmit",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some(
            "^(read|read_file|fs_write|fs_append|str_replace|grep_search|file_search|execute_bash|execute_pwsh)$",
        ),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: false,
    },
    HookEntry {
        event_name: "Stop",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "SessionEnd",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Kiro {
    fn agent(&self) -> Agent {
        Agent::Kiro
    }

    fn hooks(&self) -> &'static [HookEntry] {
        HOOKS
    }

    fn versions(&self) -> Option<Versions> {
        None
    }

    fn normalize(
        &self,
        event_name: &str,
        payload: &[u8],
        locate: &dyn Fn(&Path) -> Option<Located>,
        stamp: Stamp,
    ) -> Result<Option<Event>, NormalizeError> {
        let Some(entry) = HOOKS.iter().find(|entry| entry.event_name == event_name) else {
            return Ok(None);
        };
        normalize::event((Agent::Kiro, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // The hook writes the last line break after the text.
        Some(text.trim_end().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text goes back as it is, without its last line break; past 10,000 characters none is
    /// given.
    #[test]
    fn the_text_is_the_plain_output() {
        let (start, prompt) = (&HOOKS[0], &HOOKS[1]);
        assert_eq!(
            Kiro.context_output(start, "[cortex] map\n").as_deref(),
            Some("[cortex] map")
        );
        assert_eq!(
            Kiro.context_output(prompt, "a < \"b\"\n").as_deref(),
            Some("a < \"b\"")
        );
        assert_eq!(Kiro.context_output(prompt, &"x".repeat(10_001)), None);
    }
}
