//! The Factory Droid adapter, built from Droid's documentation (hooks reference and guide, read
//! 2026-11-06) and not yet run live. Droid speaks Claude Code's hook JSON with its own tool names.
//! At a session's start and at a prompt it adds a hook's plain standard output to the model's
//! context, the form its documentation shows for those events; after a tool, the text goes as
//! `hookSpecificOutput.additionalContext`. Its `PostToolUse` follows every call, so a call's result
//! is unknown; its documentation gives no event before a tool that adds text, so no read's hint is
//! given.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Droid is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Droid's timeout setting, in whole seconds, can say.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Droid's tools Cortex reads, by their names.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("Read", ToolKind::Read),
        ("Edit", ToolKind::Edit),
        ("Create", ToolKind::Write),
        ("ApplyPatch", ToolKind::ApplyPatch),
        ("Glob", ToolKind::Glob),
        ("Grep", ToolKind::Grep),
        ("Execute", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::Unknown,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Droid.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Droid;

/// The hooks Cortex registers with Droid.
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
        matcher: Some("^(Read|Edit|Create|ApplyPatch|Glob|Grep|Execute)$"),
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

impl Adapter for Droid {
    fn agent(&self) -> Agent {
        Agent::Droid
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
        normalize::event((Agent::Droid, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        match entry.hook {
            // Droid adds a hook's plain standard output to the model's context at these two.
            HookKind::SessionStart | HookKind::Prompt => Some(text.trim_end().to_owned()),
            _ => Some(output::context(entry.event_name, text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session's start and a prompt give the text as it is, without its last line break, which
    /// the hook writes after it; after a tool, the text goes in the hook JSON; past 10,000
    /// characters none is given.
    #[test]
    fn the_text_takes_the_form_each_event_reads() {
        let (start, prompt, after) = (&HOOKS[0], &HOOKS[1], &HOOKS[2]);
        assert_eq!(
            Droid.context_output(start, "[cortex] map\n").as_deref(),
            Some("[cortex] map")
        );
        assert_eq!(
            Droid.context_output(prompt, "a < b\n").as_deref(),
            Some("a < b")
        );
        assert_eq!(
            Droid.context_output(after, "x\n").as_deref(),
            Some(
                r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"x\n"}}"#
            )
        );
        assert_eq!(Droid.context_output(prompt, &"x".repeat(10_001)), None);
    }
}
