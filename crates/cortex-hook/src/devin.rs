//! The Devin adapter, for the Devin CLI and Devin Desktop, built from Devin's documentation (hooks
//! overview and lifecycle hooks, read 2026-11-06; CLI 3000.11.228) and not yet run live. Devin
//! speaks Claude Code's hook JSON with its own tool names. Its payloads name no working folder: the
//! project's folder is in its hooks' environment, `DEVIN_PROJECT_DIR`. A call's result is its
//! response's `success`. Its documentation gives text back at a session's start, at a prompt and
//! after a tool; not before one, so no read's hint is given. Devin's cloud sessions run on Devin's
//! own machines, where no daemon of the user's runs: they are not served.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Devin is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Devin's timeout setting, in whole seconds, can say.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Devin's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read", ToolKind::Read),
        ("edit", ToolKind::Edit),
        ("write", ToolKind::Write),
        ("apply_patch", ToolKind::ApplyPatch),
        ("grep", ToolKind::Grep),
        ("glob", ToolKind::Glob),
        ("exec", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::ResponseSuccess,
    prompt: PromptField::Prompt,
    folder_variable: Some("DEVIN_PROJECT_DIR"),
};

/// Devin.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Devin;

/// The hooks Cortex registers with Devin.
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
        matcher: Some("^(read|edit|write|apply_patch|grep|glob|exec)$"),
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
    HookEntry {
        event_name: "SessionEnd",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Devin {
    fn agent(&self) -> Agent {
        Agent::Devin
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
        normalize::event((Agent::Devin, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}
