//! The GitHub Copilot adapter, for the hook harness Copilot CLI and Visual Studio Code's agent mode
//! share, built from GitHub's hooks reference (read 2026-11-06) and not yet run live. Its hooks
//! are registered under the Claude-Code-style event names, which select the payloads with Claude
//! Code's field names. Text goes back as a top-level `additionalContext` at a session's start and
//! after a tool; its prompt hook's output is dropped, and its hook before a tool denies the call on
//! any failure, so none is registered there. A call's result follows `PostToolUse` (a success)
//! and `PostToolUseFailure`.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most bytes GitHub Copilot is given at once: it caps a tool's added context at 10 KB, counted
/// here in bytes so that text it would cut, inside a code line, is never given.
const MAX_CONTEXT_BYTES: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Copilot's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("view", ToolKind::Read),
        ("create", ToolKind::Write),
        ("edit", ToolKind::Edit),
        ("glob", ToolKind::Glob),
        ("grep", ToolKind::Grep),
        ("bash", ToolKind::Shell),
        ("powershell", ToolKind::Shell),
        // Visual Studio Code's agent mode reports Claude Code's names for these.
        ("Read", ToolKind::Read),
        ("Write", ToolKind::Write),
        ("Edit", ToolKind::Edit),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::PostIsSuccess {
        failure_event: "PostToolUseFailure",
    },
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// GitHub Copilot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Copilot;

/// The hooks Cortex registers with GitHub Copilot.
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
        gives_context: false,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some("view|create|edit|glob|grep|bash|powershell"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUseFailure",
        matcher: Some("view|create|edit|glob|grep|bash|powershell"),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: false,
    },
    HookEntry {
        event_name: "PreCompact",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "SubagentStart",
        matcher: None,
        hook: HookKind::SubagentStart,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
    HookEntry {
        event_name: "SubagentStop",
        matcher: None,
        hook: HookKind::SubagentStop,
        hard_timeout: OTHER_HARD_TIMEOUT,
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

impl Adapter for Copilot {
    fn agent(&self) -> Agent {
        Agent::Copilot
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
        normalize::event((Agent::Copilot, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.len() > MAX_CONTEXT_BYTES {
            return None;
        }
        entry
            .gives_context
            .then(|| output::top_level("additionalContext", text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook of `name`.
    fn entry(name: &str) -> Option<&'static HookEntry> {
        HOOKS.iter().find(|entry| entry.event_name == name)
    }

    /// After a tool the text goes back as a top-level `additionalContext`; the cap counts bytes,
    /// as Copilot's 10 KB does: 10,000 bytes are given, 5,001 two-byte characters (10,002 bytes)
    /// are not.
    #[test]
    fn the_text_is_a_top_level_additional_context_within_10_kb() -> Result<(), &'static str> {
        let after = entry("PostToolUse").ok_or("PostToolUse")?;
        assert_eq!(
            Copilot.context_output(after, "a \"b\"\n").as_deref(),
            Some(r#"{"additionalContext":"a \"b\"\n"}"#)
        );
        assert!(Copilot.context_output(after, &"x".repeat(10_000)).is_some());
        assert_eq!(Copilot.context_output(after, &"é".repeat(5_001)), None);
        Ok(())
    }
}
