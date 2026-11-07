//! The Junie CLI adapter, built from JetBrains' documentation of Junie CLI's hooks (read 2026-11-06;
//! an early-access feature) and not yet run live. Junie speaks Claude Code's hook JSON, but only
//! its session's start and its prompt name the session and the working folder: its tool, stop and
//! end events name neither, so they cannot be told apart by session and are not registered. Its
//! prompt hook runs in the interactive terminal only, and takes text back; its start ignores any.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Junie is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// Junie's tools: none are read, its tool events naming no session.
const TOOLS: ToolTable = ToolTable {
    tools: &[],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::Unknown,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Junie.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Junie;

/// The hooks Cortex registers with Junie.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: false,
    },
    HookEntry {
        event_name: "UserPromptSubmit",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
];

impl Adapter for Junie {
    fn agent(&self) -> Agent {
        Agent::Junie
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
        normalize::event((Agent::Junie, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}
