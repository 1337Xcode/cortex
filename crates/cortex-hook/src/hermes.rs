//! The Hermes Agent adapter, for Nous Research's agent, built from its event hooks documentation
//! (read 2026-11-06; v0.23.0 the latest release) and not yet run live. Hermes' shell hooks speak
//! Claude Code's hook JSON under event names of their own in snake case, and carry an event's
//! other fields in `extra`: the prompt is `extra.user_message` of `pre_llm_call`, which fires once
//! a turn before the model is called. Its tools' inputs are documented in part (a shell's
//! `command`, a file tool's `path`), and how a call ended is not given to a shell hook, so a
//! call's result is unknown. Text goes back as `context` at `pre_llm_call` only, which Hermes
//! appends to the user's message; its other hooks' output is ignored.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Hermes is given at once. Its documentation names no limit for a shell
/// hook; Claude Code's is taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Hermes' tools Cortex reads, by their names, and the prompt moved to Claude Code's place.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read_file", ToolKind::Read),
        ("write_file", ToolKind::Write),
        ("patch", ToolKind::Edit),
        ("search_files", ToolKind::Grep),
        ("terminal", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[("/extra/user_message", "/prompt")],
    outcome: Outcome::Unknown,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Hermes Agent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hermes;

/// The hooks Cortex registers with Hermes.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "on_session_start",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: false,
    },
    HookEntry {
        event_name: "pre_llm_call",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "post_tool_call",
        matcher: Some("^(read_file|write_file|patch|search_files|terminal)$"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: false,
    },
    HookEntry {
        event_name: "on_session_end",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Hermes {
    fn agent(&self) -> Agent {
        Agent::Hermes
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
        normalize::event((Agent::Hermes, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::top_level("context", text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text goes back as a top-level `context`; past 10,000 characters none is given.
    #[test]
    fn the_text_is_a_context() {
        let prompt = &HOOKS[1];
        assert_eq!(
            Hermes.context_output(prompt, "a \"b\"\n").as_deref(),
            Some(r#"{"context":"a \"b\"\n"}"#)
        );
        assert_eq!(Hermes.context_output(prompt, &"x".repeat(10_001)), None);
    }
}
