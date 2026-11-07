//! The Crush adapter, built from Crush's hooks documentation (read 2026-11-06) and not yet run
//! live. Crush speaks Claude Code's hook JSON under a few names of its own: `TurnStart` begins a
//! turn and carries no prompt, `TurnEnd` ends it. Its file tools' input fields are not spelled out
//! (its hooks' environment names a file path), so a field under another name than Claude Code's is
//! not read. A call's result follows `PostToolUse` (a success) and `PostToolUseFailure`. Text goes
//! back as a top-level `context` around a tool only: before a read and after a tool; its session
//! and turn events only observe.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Crush is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Crush's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("view", ToolKind::Read),
        ("write", ToolKind::Write),
        ("edit", ToolKind::Edit),
        ("multiedit", ToolKind::MultiEdit),
        ("glob", ToolKind::Glob),
        ("grep", ToolKind::Grep),
        ("bash", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::PostIsSuccess {
        failure_event: "PostToolUseFailure",
    },
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Crush.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Crush;

/// The tools Crush's tool hooks match.
const TOOL_MATCHER: Option<&str> = Some("^(view|write|edit|multiedit|glob|grep|bash)$");

/// The hooks Cortex registers with Crush.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: false,
    },
    HookEntry {
        event_name: "TurnStart",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: false,
    },
    HookEntry {
        event_name: "PreToolUse",
        matcher: Some("^view$"),
        hook: HookKind::PreTool,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: TOOL_MATCHER,
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUseFailure",
        matcher: TOOL_MATCHER,
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PreCompact",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "TurnEnd",
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

impl Adapter for Crush {
    fn agent(&self) -> Agent {
        Agent::Crush
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
        normalize::event((Agent::Crush, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // No `decision`: Crush takes an `allow` for a pass past its permission prompt.
        Some(output::top_level("context", text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text goes back as a top-level `context`, with no decision; past 10,000 characters none
    /// is given.
    #[test]
    fn the_text_is_a_context_with_no_decision() {
        let before = &HOOKS[2];
        assert_eq!(
            Crush.context_output(before, "a \"b\"\n").as_deref(),
            Some(r#"{"context":"a \"b\"\n"}"#)
        );
        assert_eq!(Crush.context_output(before, &"x".repeat(10_001)), None);
    }
}
