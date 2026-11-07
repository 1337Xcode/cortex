//! The TRAE IDE adapter, built from TRAE's documentation (hooks overview and configuration
//! reference, read 2026-11-06; hooks since TRAE 3.5.66) and not yet run live. TRAE speaks Claude
//! Code's hook JSON with its own tool names, gives text back at a session's start, at a prompt and
//! at a tool's two moments, and has no event for a session's end or a compaction. Its
//! `PostToolUse` follows every call, so a call's result is unknown.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters TRAE is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// TRAE's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("Read", ToolKind::Read),
        ("Write", ToolKind::Write),
        ("Edit", ToolKind::Edit),
        ("Glob", ToolKind::Glob),
        ("Grep", ToolKind::Grep),
        ("RunCommand", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::Unknown,
    prompt: PromptField::Prompt,
    folder_variable: Some("TRAE_PROJECT_DIR"),
};

/// TRAE.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Trae;

/// The hooks Cortex registers with TRAE.
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
        event_name: "PreToolUse",
        matcher: Some("^Read$"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some("^(Read|Write|Edit|Glob|Grep|RunCommand)$"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "Stop",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Trae {
    fn agent(&self) -> Agent {
        Agent::Trae
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
        normalize::event((Agent::Trae, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}
