//! The Gemini CLI adapter, built from Gemini CLI's documentation (hooks reference and tools
//! reference, read 2026-11-06; 0.67.0 the latest release) and not yet run live. Gemini CLI speaks
//! Claude Code's hook JSON under its own event names: `BeforeAgent` is the prompt, `AfterTool` a
//! tool's result, `PreCompress` a compaction, `AfterAgent` the end of a turn. Its tools are
//! `read_file`, `write_file`, `replace`, `glob`, `grep_search` and `run_shell_command`; a read
//! naming `start_line` or `end_line` is not recorded, their counting not being documented. A
//! call's result is its response's `error`, present for a failure. Text goes back at a session's
//! start, at a prompt and after a tool; nothing before a tool takes text.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Gemini CLI is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Gemini CLI's tools Cortex reads, by their names and a legacy alias.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read_file", ToolKind::Read),
        ("write_file", ToolKind::Write),
        ("replace", ToolKind::Edit),
        ("glob", ToolKind::Glob),
        ("grep_search", ToolKind::Grep),
        ("search_file_content", ToolKind::Grep),
        ("run_shell_command", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::ResponseError,
    prompt: PromptField::Prompt,
    folder_variable: Some("GEMINI_PROJECT_DIR"),
};

/// Gemini CLI.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gemini;

/// The hooks Cortex registers with Gemini CLI.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "BeforeAgent",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "AfterTool",
        matcher: Some(
            "^(read_file|write_file|replace|glob|grep_search|search_file_content|run_shell_command)$",
        ),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PreCompress",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "AfterAgent",
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

impl Adapter for Gemini {
    fn agent(&self) -> Agent {
        Agent::Gemini
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
        normalize::event((Agent::Gemini, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}
