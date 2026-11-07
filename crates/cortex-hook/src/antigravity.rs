//! The Google Antigravity adapter, for its app, its `agy` command-line agent and its editor, which
//! share one hooks file; built from Antigravity's hooks documentation (read 2026-11-06) and not
//! yet run live. Antigravity's payloads are its own, in camel case, and are moved to Claude Code's
//! places before they are read: the session is `conversationId`, the working folder the first of
//! `workspacePaths`, the tool call `toolCall` with its name and its arguments, whose names are
//! Pascal case (`AbsolutePath`, `TargetFile`, `CommandLine`). It has no prompt event: its hook
//! before each model call carries the last prompt, and is the prompt on the first model call of a
//! turn only. A call's result is the payload's `error`, empty for a success. Its transcript's
//! path is written with `~` and its lines are not documented, so no transcript is named. Text
//! goes back as a step the hook injects, at a session's start and before the turn's first model
//! call; its hooks around a tool take none.

use std::path::Path;
use std::time::Duration;

use cortex_core::{Agent, Event, HookKind, ToolKind};
use serde_json::json;

use crate::adapter::{Adapter, HookEntry, Located, NormalizeError, Stamp, Versions};
use crate::shaped::normalize;
use crate::shaped::table::{Outcome, PromptField, ToolTable};

/// The most characters Antigravity is given at once. Its documentation names no limit; Claude
/// Code's is taken.
const MAX_CONTEXT_CHARS: usize = 10_000;

/// The hard timeout of the events whose budget names none: a margin over their 60 ms budget.
const OTHER_HARD_TIMEOUT: Duration = Duration::from_millis(300);

/// Antigravity's tools Cortex reads, by their names, and its payload's fields moved to Claude
/// Code's places. The tool call's arguments move first, so the argument moves after them find
/// them in the tool's input.
const TOOLS: ToolTable = ToolTable {
    tools: &[
        ("view_file", ToolKind::Read),
        ("write_to_file", ToolKind::Write),
        ("replace_file_content", ToolKind::Edit),
        ("multi_replace_file_content", ToolKind::MultiEdit),
        ("grep_search", ToolKind::Grep),
        ("find_by_name", ToolKind::Glob),
        ("run_command", ToolKind::Shell),
    ],
    subcommands: &[],
    renames: &[
        ("/conversationId", "/session_id"),
        ("/workspacePaths/0", "/cwd"),
        ("/modelName", "/model"),
        ("/lastUserInput", "/prompt"),
        ("/invocationNum", "/invocation_num"),
        ("/toolCall/name", "/tool_name"),
        ("/toolCall/args", "/tool_input"),
        ("/tool_input/AbsolutePath", "/tool_input/file_path"),
        ("/tool_input/TargetFile", "/tool_input/file_path"),
        ("/tool_input/StartLine", "/tool_input/start_line"),
        ("/tool_input/EndLine", "/tool_input/end_line"),
        ("/tool_input/SearchPath", "/tool_input/path"),
        ("/tool_input/SearchDirectory", "/tool_input/path"),
        ("/tool_input/Query", "/tool_input/pattern"),
        ("/tool_input/Pattern", "/tool_input/pattern"),
        ("/tool_input/CommandLine", "/tool_input/command"),
    ],
    outcome: Outcome::PayloadError,
    prompt: PromptField::FirstInvocation,
    folder_variable: None,
};

/// Google Antigravity.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Antigravity;

/// The hooks Cortex registers with Antigravity.
const HOOKS: &[HookEntry] = &[
    HookEntry {
        event_name: "SessionStart",
        matcher: None,
        hook: HookKind::SessionStart,
        hard_timeout: Duration::from_millis(1_500),
        gives_context: true,
    },
    HookEntry {
        event_name: "PreInvocation",
        matcher: None,
        hook: HookKind::Prompt,
        hard_timeout: Duration::from_millis(1_000),
        gives_context: true,
    },
    HookEntry {
        event_name: "PostToolUse",
        matcher: Some(
            "^(view_file|write_to_file|replace_file_content|multi_replace_file_content|grep_search|find_by_name|run_command)$",
        ),
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
];

impl Adapter for Antigravity {
    fn agent(&self) -> Agent {
        Agent::Antigravity
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
        normalize::event((Agent::Antigravity, &TOOLS, entry), payload, locate, stamp)
    }

    fn context_output(&self, _: &HookEntry, text: &str) -> Option<String> {
        // INVARIANT: budgeted. A text past the agent's limit is not given in part.
        if text.encode_utf16().count() > MAX_CONTEXT_CHARS {
            return None;
        }
        // A system message stays in the conversation for the turn's later model calls, where an
        // ephemeral one is, by the documentation's word, transient.
        let step = json!({"systemMessage": {"systemMessage": text}});
        Some(json!({ "injectSteps": [step] }).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text goes back as a system message the hook injects, its quote and line break escaped
    /// in the JSON string; past 10,000 characters none is given.
    #[test]
    fn the_text_is_an_injected_system_message() {
        let (start, prompt) = (&HOOKS[0], &HOOKS[1]);
        assert_eq!(
            Antigravity.context_output(start, "a \"b\"\n").as_deref(),
            Some(r#"{"injectSteps":[{"systemMessage":{"systemMessage":"a \"b\"\n"}}]}"#)
        );
        assert_eq!(
            Antigravity.context_output(prompt, "x").as_deref(),
            Some(r#"{"injectSteps":[{"systemMessage":{"systemMessage":"x"}}]}"#)
        );
        assert_eq!(
            Antigravity.context_output(prompt, &"x".repeat(10_001)),
            None
        );
    }
}
