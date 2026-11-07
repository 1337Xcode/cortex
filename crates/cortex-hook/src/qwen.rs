//! The Qwen Code adapter, built from Qwen Code's documentation (hooks, file system and shell tools,
//! read 2026-11-06) and not yet run live. Qwen Code speaks Claude Code's hook JSON with its own
//! tool names. Its prompt hook also runs for sends that are not the user's (a tool's result going
//! back to the model); only a payload carrying `submitted_prompt` is the user's prompt. It escapes
//! angle brackets in a hook's text, so a text holding one is not given: a code line would reach the
//! model altered.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Qwen Code is given at once. Its documentation names no limit; Claude Code's,
/// counted the same way (UTF-16 code units, as a JavaScript program measures a string), is taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget, and
/// below the one second Qwen Code's timeout setting, in whole seconds, can say.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Qwen Code's tools Cortex reads, by their ids.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read_file", ToolKind::Read),
        ("write_file", ToolKind::Write),
        ("edit", ToolKind::Edit),
        ("glob", ToolKind::Glob),
        ("grep_search", ToolKind::Grep),
        ("run_shell_command", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[],
    outcome: Outcome::PostIsSuccess {
        failure_event: "PostToolUseFailure",
    },
    prompt: PromptField::Submitted,
    folder_variable: None,
};

/// The tools whose results Cortex records.
const TOOL_RESULTS: &str = "^(read_file|write_file|edit|glob|grep_search|run_shell_command)$";

/// Qwen Code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Qwen;

/// The hooks Cortex registers with Qwen Code: as Claude Code's, its read's hint appended to the
/// read's result, and none at a stop, whose text Qwen Code takes as an order to go on.
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
        matcher: Some("^read_file$"),
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

impl Adapter for Qwen {
    fn agent(&self) -> Agent {
        Agent::Qwen
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
        normalize::event((Agent::Qwen, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // INVARIANT: true-at-render. Qwen Code escapes angle brackets in a hook's text: a code
        // line holding one would reach the model altered.
        if text.contains(['<', '>']) {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text Qwen Code would alter, holding an angle bracket either way, is not given; one
    /// without is, up to 10,000 characters.
    #[test]
    fn a_text_qwen_code_would_alter_is_not_given() {
        let entry = HOOKS[1];
        for altered in ["if a < b:\n", "def f() -> int:\n"] {
            assert_eq!(Qwen.context_output(&entry, altered), None, "{altered}");
        }
        assert!(Qwen.context_output(&entry, "x = 1\n").is_some());
        assert!(Qwen.context_output(&entry, &"x".repeat(10_000)).is_some());
        assert_eq!(Qwen.context_output(&entry, &"x".repeat(10_001)), None);
    }
}
