//! The Augment (Auggie CLI) adapter, built from Augment's documentation of Auggie's hooks (0.x and
//! 1.x pages, read 2026-11-06; 0.37.0 the latest stable) and not yet run live. Auggie names its
//! session `conversation_id` and its 1.x prompt `user_prompt`, renamed here to Claude Code's names;
//! its 0.x payloads name no working folder, which its hooks' environment does
//! (`AUGMENT_PROJECT_DIR`). Text goes back at a session's start (0.x; 1.x ignores it) and after a
//! tool, appended to the tool's result; its prompt event takes none, and nothing is given before
//! a tool. A call's result is `tool_is_error`.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::table::{Outcome, PromptField, ToolTable};
use crate::shaped::{normalize, output};

/// The most characters Augment is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Auggie's tools Cortex reads, by their 1.x names and the 0.x names they replaced.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read", ToolKind::Read),
        ("view", ToolKind::Read),
        ("edit", ToolKind::Edit),
        ("str-replace-editor", ToolKind::Edit),
        ("write", ToolKind::Write),
        ("save-file", ToolKind::Write),
        ("grep", ToolKind::Grep),
        ("find", ToolKind::Glob),
        ("bash", ToolKind::Shell),
        ("launch-process", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[("conversation_id", "session_id"), ("user_prompt", "prompt")],
    outcome: Outcome::ToolIsError,
    prompt: PromptField::Prompt,
    folder_variable: Some("AUGMENT_PROJECT_DIR"),
};

/// Augment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Augment;

/// The hooks Cortex registers with Augment.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PromptSubmit",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: false,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some(
            "^(read|view|edit|str-replace-editor|write|save-file|grep|find|bash|launch-process)$",
        ),
        hook: HookKind::PostTool,
        hard_timeout: Duration::from_millis(500),
        gives_context: true,
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

impl Adapter for Augment {
    fn agent(&self) -> Agent {
        Agent::Augment
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
        normalize::event((Agent::Augment, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, entry: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        Some(output::context(entry.event_name, text))
    }
}
