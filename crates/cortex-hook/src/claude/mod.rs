//! The Claude Code adapter: the hooks Cortex registers with Claude Code, its payloads turned into
//! normalized events, and the text Cortex gives it written as its hook JSON.

mod normalize;
mod output;
mod payload;
mod tool;

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};

/// The tools whose results Cortex records: reads, searches, edits and shells. Other tools run
/// without spawning a hook.
const TOOL_RESULTS: &str = "Read|Grep|Glob|Edit|Write|MultiEdit|Bash|PowerShell";

/// The versions of Claude Code Cortex knows: the oldest is 2.1.295, the first to read `onFailure`,
/// which Cortex's hooks set so that a hook that fails never holds up the agent; the newest tested
/// is 2.1.316, whose payloads the fixtures hold beside those of 2.1.312.
const VERSIONS: Versions = Versions {
    oldest: "2.1.295",
    newest_tested: "2.1.316",
};

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Claude Code's timeout setting rounds up to.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Claude Code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Claude;

/// The hooks Cortex registers with Claude Code. A tool's result is registered for both its outcomes
/// (`PostToolUse`, `PostToolUseFailure`): a shell that fails reports its exit code only there.
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

    fn versions(&self) -> Versions {
        VERSIONS
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
        normalize::event(entry, payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        output::context(entry.event_name, text)
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
}
