//! The Claude Code adapter: the hooks Cortex registers with Claude Code, and its payloads turned
//! into normalized events.

mod normalize;
mod payload;

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp};

/// The tools whose results Cortex records: reads, searches, edits and shells. Other tools run
/// without spawning a hook.
const TOOL_RESULTS: &str = "Read|Grep|Glob|Edit|Write|MultiEdit|Bash|PowerShell";

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
    },
    HookEntry {
        event_name: "UserPromptSubmit",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
    },
    HookEntry {
        event_name: "PreToolUse",
        matcher: Some("Read"),
        hook: HookKind::PreTool,
        hard_timeout: Duration::from_millis(300),
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some(TOOL_RESULTS),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
    },
    HookEntry {
        event_name: "PostToolUseFailure",
        matcher: Some(TOOL_RESULTS),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
    },
    HookEntry {
        event_name: "PreCompact",
        matcher: None,
        hook: HookKind::Compact,
        hard_timeout: OTHER_HARD_TIMEOUT,
    },
    HookEntry {
        event_name: "Stop",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
    },
    HookEntry {
        event_name: "SubagentStart",
        matcher: None,
        hook: HookKind::SubagentStart,
        hard_timeout: OTHER_HARD_TIMEOUT,
    },
    HookEntry {
        event_name: "SubagentStop",
        matcher: None,
        hook: HookKind::SubagentStop,
        hard_timeout: OTHER_HARD_TIMEOUT,
    },
    HookEntry {
        event_name: "SessionEnd",
        matcher: None,
        hook: HookKind::SessionEnd,
        hard_timeout: OTHER_HARD_TIMEOUT,
    },
];

impl Adapter for Claude {
    fn agent(&self) -> Agent {
        Agent::Claude
    }

    fn hooks(&self) -> &'static [HookEntry] {
        HOOKS
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
}
