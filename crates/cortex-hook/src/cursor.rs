//! The Cursor adapter, for the editor's agent, built from Cursor's hooks documentation (read
//! 2026-11-06) and not yet run live. Cursor's hook payloads are its own: the session is
//! `conversation_id`, stable across turns (its start also names a `session_id`, which is not
//! used); the working folder is its hooks' environment's `CURSOR_PROJECT_DIR` when a payload names
//! no `cwd`. Text goes back as a top-level `additional_context` at a session's start and after a
//! tool; its prompt hook only allows or blocks, and its permission hooks (those before a tool,
//! and a subagent's start) take silence for a refusal, so none is registered there. Its `cursor-agent` command-line agent runs a few of
//! these hooks only.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Cursor is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Cursor's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("Read", ToolKind::Read),
        ("Write", ToolKind::Write),
        ("Grep", ToolKind::Grep),
        ("Shell", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[("/conversation_id", "/session_id")],
    outcome: Outcome::PostIsSuccess {
        failure_event: "postToolUseFailure",
    },
    prompt: PromptField::Prompt,
    folder_variable: Some("CURSOR_PROJECT_DIR"),
};

/// Cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor;

/// The hooks Cortex registers with Cursor.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "sessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "beforeSubmitPrompt",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: false,
    },
    HookEntry {
        event_name: "postToolUse",
        matcher: Some("^(Read|Write|Grep|Shell)$"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "postToolUseFailure",
        matcher: Some("^(Read|Write|Grep|Shell)$"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: false,
    },
    HookEntry {
        event_name: "preCompact",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "stop",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "subagentStop",
        matcher: None,
        hook: HookKind::SubagentStop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "sessionEnd",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Cursor {
    fn agent(&self) -> Agent {
        Agent::Cursor
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
        normalize::event((Agent::Cursor, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        entry
            .gives_context
            .then(|| output::top_level("additional_context", text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook of `name`.
    fn entry(name: &str) -> Option<&'static HookEntry> {
        HOOKS.iter().find(|entry| entry.event_name == name)
    }

    /// After a tool the text goes back as a top-level `additional_context`, quotes and line breaks
    /// escaped; past 10,000 characters, and at a hook that gives none, nothing is given.
    #[test]
    fn the_text_is_a_top_level_additional_context() -> Result<(), &'static str> {
        let after = entry("postToolUse").ok_or("postToolUse")?;
        assert_eq!(
            Cursor.context_output(after, "a \"b\"\n").as_deref(),
            Some(r#"{"additional_context":"a \"b\"\n"}"#)
        );
        assert_eq!(Cursor.context_output(after, &"x".repeat(10_001)), None);
        let stop = entry("stop").ok_or("stop")?;
        assert_eq!(Cursor.context_output(stop, "a"), None);
        Ok(())
    }
}
