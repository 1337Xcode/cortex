//! The Codex CLI adapter: the hooks Cortex registers with Codex, its payloads turned into normalized
//! events, and the text Cortex gives it written as its hook JSON. Codex speaks the hook JSON Claude
//! Code defined, with its own tools: every read and search is a shell command, every edit a patch.

mod command;
mod flags;
mod rollout;
mod tool;
mod words;

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::{normalize, output};
use tool::CodexTools;

/// The versions of Codex Cortex knows: 0.165.0, whose payloads the fixtures hold, is the oldest and
/// the newest tested.
const VERSIONS: Versions = Versions {
    oldest: "0.165.0",
    newest_tested: "0.165.0",
};

/// The most bytes of text Codex adds to its context whole from one hook. Past it, Codex keeps the
/// text in a file and gives the model its head, its tail and the file's path: Codex counts a text
/// as one token per four bytes, rounded up, and keeps 2,500 tokens whole.
const MAX_CONTEXT_BYTES: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Codex's timeout setting, in whole seconds, can say.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Codex CLI.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Codex;

/// The hooks Cortex registers with Codex. Every shell command is tool `Bash`, so the moment before
/// one runs is heard for all of them, a read's hint given before the read; a patch is heard after
/// it ran. Codex has no event for a tool that failed: a shell's status is in its rollout file. A
/// compaction is heard before it runs, and the recap given at the start that follows it: Codex
/// drops what a hook gives after a compaction.
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
        matcher: Some("^Bash$"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some("^(Bash|apply_patch)$"),
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
        event_name: "Stop",
        matcher: None,
        hook: HookKind::Stop,
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
        event_name: "SessionEnd",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Codex {
    fn agent(&self) -> Agent {
        Agent::Codex
    }

    fn hooks(&self) -> &'static [HookEntry] {
        HOOKS
    }

    fn versions(&self) -> Option<Versions> {
        Some(VERSIONS)
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
        normalize::event((Agent::Codex, &CodexTools, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.len() > MAX_CONTEXT_BYTES {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cortex gives Codex text at a session's start, at a prompt, before a shell command (a read's
    /// hint) and after a tool (an edit's warnings), and at no other hook: not at a stop, whose
    /// output Codex takes as an order to go on.
    #[test]
    fn a_session_start_a_prompt_and_a_tool_s_two_moments_give_context() {
        let giving: Vec<&str> = HOOKS
            .iter()
            .filter(|entry| entry.gives_context)
            .map(|entry| entry.event_name)
            .collect();
        assert_eq!(
            giving,
            [
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse"
            ]
        );
    }

    /// Codex keeps 2,500 tokens of four bytes whole: a text of 10,000 bytes is given and one of
    /// 10,001 is not. An accented letter is two bytes, so 5,000 of them are given and 5,001 are not.
    #[test]
    fn a_text_past_codex_s_limit_is_not_given() {
        let entry = HOOKS[1];
        assert!(Codex.context_output(&entry, &"x".repeat(10_000)).is_some());
        assert_eq!(Codex.context_output(&entry, &"x".repeat(10_001)), None);
        assert!(
            Codex
                .context_output(&entry, &"\u{e9}".repeat(5_000))
                .is_some()
        );
        assert_eq!(Codex.context_output(&entry, &"\u{e9}".repeat(5_001)), None);
    }
}
