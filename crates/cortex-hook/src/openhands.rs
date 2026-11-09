//! The OpenHands adapter, built from OpenHands' documentation and its agent SDK's source (hooks
//! guides and `openhands-sdk` hook types, read 2026-11-06) and not yet run live. Its hook payloads
//! name the working folder `working_dir` and the prompt `message`, renamed here to Claude Code's
//! names; its file editor is one tool whose `command` says what it does (`view` reads, `create`
//! writes, `str_replace`, `insert` and `undo_edit` edit); a terminal command's result is the
//! response's `exit_code`. Text goes back as a top-level `additionalContext` at a session's start
//! and at a prompt, the two events whose text the SDK gives the model. Its hooks run where its
//! agent server runs: on the user's machine for the local backend only.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters OpenHands is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// OpenHands' tools Cortex reads, by their names and the file editor's commands.
const TOOLS: ToolTable = ToolTable {
    tools: &[("terminal", ToolKind::Shell)],
    subcommands: &[
        ("file_editor", "view", ToolKind::Read),
        ("file_editor", "create", ToolKind::Write),
        ("file_editor", "str_replace", ToolKind::Edit),
        ("file_editor", "insert", ToolKind::Edit),
        ("file_editor", "undo_edit", ToolKind::Edit),
    ],
    renames: &[
        ("/working_dir", "/cwd"),
        ("/message", "/prompt"),
        ("/event_type", "/hook_event_name"),
    ],
    outcome: Outcome::ResponseExitCode,
    prompt: PromptField::Prompt,
    folder_variable: Some("OPENHANDS_PROJECT_DIR"),
};

/// OpenHands.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenHands;

/// The hooks Cortex registers with OpenHands.
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
        matcher: Some("^(terminal|file_editor)$"),
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

impl Adapter for OpenHands {
    fn agent(&self) -> Agent {
        Agent::OpenHands
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
        normalize::event((Agent::OpenHands, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
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

    /// At a prompt the text goes back as a top-level `additionalContext`; past 10,000 characters
    /// nothing is given.
    #[test]
    fn the_text_is_a_top_level_additional_context() -> Result<(), &'static str> {
        let prompt = entry("UserPromptSubmit").ok_or("UserPromptSubmit")?;
        assert_eq!(
            OpenHands.context_output(prompt, "a \"b\"\n").as_deref(),
            Some(r#"{"additionalContext":"a \"b\"\n"}"#)
        );
        assert_eq!(OpenHands.context_output(prompt, &"x".repeat(10_001)), None);
        Ok(())
    }
}
