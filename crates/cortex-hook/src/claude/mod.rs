//! The Claude Code adapter: the hooks Cortex registers with Claude Code, its payloads turned into
//! normalized events, and the text Cortex gives it written as its hook JSON.

mod tool;

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, holds_reminder_tag};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::{normalize, output};
use tool::ClaudeTools;

/// The tools whose results Cortex records: reads, searches, edits and shells. Other tools run
/// without spawning a hook.
const TOOL_RESULTS: &str = "Read|Grep|Glob|Edit|Write|MultiEdit|Bash|PowerShell";

/// The versions of Claude Code Cortex knows: the oldest is 2.1.295, the first to read `onFailure`,
/// which Cortex's hooks set so that a hook that fails never holds up the agent; the newest tested
/// is 2.1.318, whose payloads the fixtures hold beside those of 2.1.312 and 2.1.316.
const VERSIONS: Versions = Versions {
    oldest: "2.1.295",
    newest_tested: "2.1.318",
};

/// The most characters Claude Code takes whole as a hook's `additionalContext`. Past it, Claude
/// Code keeps the text in a file and gives the agent the file's path and the text's first 2,000
/// characters, which is not the text Cortex gives. Counted in UTF-16 code units, as Claude Code, a
/// JavaScript program, measures a string; never fewer than the text's characters.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Claude Code's timeout setting rounds up to.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Claude Code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Claude;

/// The hooks Cortex registers with Claude Code. A tool's result is registered for both its outcomes
/// (`PostToolUse`, `PostToolUseFailure`): a shell that fails reports its exit code only there. A
/// turn that ends on an API error (a usage limit, a billing error, an overloaded service) sends
/// `StopFailure` instead of `Stop`, whose output Claude Code ignores: a stop whose end reason is
/// the error, so that its task ends unfinished.
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
        matcher: Some("Read"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some(TOOL_RESULTS),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUseFailure",
        matcher: Some(TOOL_RESULTS),
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
        event_name: "Stop",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: true,
    },
    HookEntry {
        event_name: "StopFailure",
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

impl Adapter for Claude {
    fn agent(&self) -> Agent {
        Agent::Claude
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
        normalize::event((Agent::Claude, &ClaudeTools, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // INVARIANT: true-at-render. A code line the agent would get altered is not given: Claude
        // Code writes the `<` of its reminder tags as `&lt;` in a hook's text
        // (`cortex_core::REMINDER_TAGS`).
        if holds_reminder_tag(text) {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cortex gives Claude Code text at a session's start, at a prompt, before a read (its hint),
    /// after a tool succeeded (an edit's warnings) and at a stop (strict mode's reason), and at no
    /// other hook: not after a tool failed.
    #[test]
    fn a_session_start_a_prompt_a_read_a_tool_s_success_and_a_stop_give_context() {
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
                "PostToolUse",
                "Stop"
            ]
        );
    }

    /// Claude Code takes 10,000 characters whole: a text of 10,000 is given and one of 10,001 is
    /// not. An accented letter is two bytes and one character; a character past the basic plane
    /// (U+1D11E) is two to Claude Code, so 5,000 of them are given and 5,001 are not.
    #[test]
    fn a_text_past_claude_code_s_limit_is_not_given() {
        let entry = HOOKS[0];
        assert!(Claude.context_output(&entry, &"x".repeat(10_000)).is_some());
        assert_eq!(Claude.context_output(&entry, &"x".repeat(10_001)), None);
        assert!(
            Claude
                .context_output(&entry, &"\u{e9}".repeat(10_000))
                .is_some()
        );
        assert!(
            Claude
                .context_output(&entry, &"\u{1d11e}".repeat(5_000))
                .is_some()
        );
        assert_eq!(
            Claude.context_output(&entry, &"\u{1d11e}".repeat(5_001)),
            None
        );
    }

    /// A text holding a tag Claude Code rewrites, opening or closing, in any case, is not given;
    /// one that only names it is.
    #[test]
    fn a_text_claude_code_would_rewrite_is_not_given() {
        let entry = HOOKS[1];
        for rewritten in [
            "x = '<system-reminder>'\n",
            "</system-reminder>\n",
            "<System-Reminder data=\"1\">\n",
        ] {
            assert_eq!(
                Claude.context_output(&entry, rewritten),
                None,
                "{rewritten}"
            );
        }
        assert!(
            Claude
                .context_output(&entry, "a system-reminder tag\n")
                .is_some()
        );
    }
}
