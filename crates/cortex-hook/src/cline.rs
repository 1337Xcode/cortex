//! The Cline adapter, for its Visual Studio Code extension's file hooks, built from Cline's hooks
//! documentation (its last full version, read 2026-11-06) and not yet run live. A hook is a file
//! named after its event, and its payload is Cline's own: the session is `taskId`, the working
//! folder the first of `workspaceRoots`, the model `model.slug`, and each event's fields sit under
//! the event's name (`userPromptSubmit.prompt`, `postToolUse.toolName`, `postToolUse.parameters`,
//! `postToolUse.success`), moved to Claude Code's places before the payload is read. Its tools
//! name a file in `path` and a command in `command`. A call's result is `postToolUse.success`.
//! Text goes back as `contextModification` in Cline's one-line JSON, at a task's start or resumption,
//! at a prompt and after a tool, and reaches the model with its next request. Its hook before a
//! tool is not registered: what silence means to it is not documented.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};
use serde_json::json;

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The most characters Cline is given at once. Its documentation names no limit; Claude Code's is
/// taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Cline's tools Cortex reads, by their names, and its payload's fields moved to Claude Code's
/// places.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("read_file", ToolKind::Read),
        ("write_to_file", ToolKind::Write),
        ("replace_in_file", ToolKind::Edit),
        ("search_files", ToolKind::Grep),
        ("execute_command", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[
        ("/taskId", "/session_id"),
        ("/workspaceRoots/0", "/cwd"),
        ("/model/slug", "/model"),
        ("/userPromptSubmit/prompt", "/prompt"),
        ("/postToolUse/toolName", "/tool_name"),
        ("/postToolUse/parameters", "/tool_input"),
        ("/postToolUse/success", "/tool_response/success"),
    ],
    outcome: Outcome::ResponseSuccess,
    prompt: PromptField::Prompt,
    folder_variable: None,
};

/// Cline.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cline;

/// The hooks Cortex registers with Cline, by its hook files' names. Cline runs every hook for
/// every tool, so none names a matcher.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "TaskStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "TaskResume",
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
        matcher: None,
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
        event_name: "TaskComplete",
        matcher: None,
        hook: HookKind::Stop,
        hard_timeout: OTHER_HARD_TIMEOUT,
        gives_context: false,
    },
];

impl Adapter for Cline {
    fn agent(&self) -> Agent {
        Agent::Cline
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
        normalize::event((Agent::Cline, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // Cline reads one line of JSON: the text's line breaks are escaped in its string.
        let fields = json!({"cancel": false, "contextModification": text, "errorMessage": ""});
        Some(fields.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text goes back in Cline's one line of JSON, which cancels nothing and shows no error,
    /// its line break escaped; past 10,000 characters none is given.
    #[test]
    fn the_text_is_a_context_modification() {
        let after = &HOOKS[3];
        assert_eq!(
            Cline.context_output(after, "a \"b\"\nc\n").as_deref(),
            Some(r#"{"cancel":false,"contextModification":"a \"b\"\nc\n","errorMessage":""}"#)
        );
        assert_eq!(Cline.context_output(after, &"x".repeat(10_001)), None);
    }
}
