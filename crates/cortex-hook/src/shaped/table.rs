//! Tools read from a table, for an agent whose hooks speak Claude Code's JSON and whose adapter is
//! built from the agent's documentation: its tool names and their kinds, a file in the input's
//! `file_path` or `path`, a search's `pattern` and folder in `pattern` and `path`, a shell's
//! `command`; where its prompt and its working folder come from; and what its tool events say of
//! how a call ended. What the documentation does not say is not guessed: a read that names an
//! offset or a limit, whose lines the documentation does not count, is not recorded; a search's
//! hits, read from a response the documentation does not describe, are none; a shell's exit code
//! is unknown.

use std::path::PathBuf;

use cortex_core::{
    HookKind, MAX_COMMAND_CHARS, RepoPath, ShellCommand, ToolKind, ToolResult, ToolTarget,
};

use super::payload::{Payload, ToolInput};
use super::place::{Place, place};
use super::{Aim, Prompted, Tools};
use crate::adapter::Located;

/// One agent's tools and payloads, as its documentation describes them.
pub(crate) struct ToolTable {
    /// Each tool's name, as the agent's payloads write it, and its kind; a name not here is a
    /// tool of no kind Cortex reads.
    pub(crate) tools: &'static [(&'static str, ToolKind)],
    /// What the agent's tool events say of how a call ended.
    pub(crate) outcome: Outcome,
    /// Where the user's prompt is.
    pub(crate) prompt: PromptField,
    /// The environment variable naming the agent's working folder, for an agent whose payloads
    /// name none.
    pub(crate) folder_variable: Option<&'static str>,
}

/// What an agent's tool events say of how a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Nothing: a call's result is unknown.
    Unknown,
    /// `PostToolUse` follows only a call that succeeded, and this event a call that failed.
    PostIsSuccess {
        /// The agent's event for a call that failed.
        failure_event: &'static str,
    },
    /// The response's `success` field says.
    ResponseSuccess,
}

/// Where an agent's payload holds the user's prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptField {
    /// In `prompt`.
    Prompt,
    /// In `submitted_prompt`, which only the user's prompts carry: a prompt event without it is
    /// the agent's own send.
    Submitted,
}

impl Tools for ToolTable {
    fn kind(&self, name: &str, _: &Payload<'_>) -> ToolKind {
        self.tools
            .iter()
            .find(|(named, _)| *named == name)
            .map_or(ToolKind::Other, |(_, kind)| *kind)
    }

    fn aim(&self, tool: ToolKind, _: HookKind, payload: &Payload<'_>, located: &Located) -> Aim {
        let Some(input) = payload.tool_input.as_ref() else {
            return Aim::Nothing;
        };
        // Relative paths are the agent's working folder's, which the payload may not name.
        let cwd = located.cwd.to_str();
        let target = match tool {
            ToolKind::Read if input.offset.is_some() || input.limit.is_some() => {
                return Aim::Nothing;
            }
            ToolKind::Read
            | ToolKind::Edit
            | ToolKind::Write
            | ToolKind::MultiEdit
            | ToolKind::ApplyPatch => match place(file_of(input), cwd, located) {
                Place::Outside => return Aim::Outside,
                Place::Unnamed => return Aim::Nothing,
                Place::Inside(path) => ToolTarget {
                    path: Some(path),
                    ..ToolTarget::default()
                },
            },
            ToolKind::Grep | ToolKind::Glob => match place(input.path.as_deref(), cwd, located) {
                Place::Outside => return Aim::Outside,
                place => ToolTarget {
                    path: place
                        .path()
                        .or_else(|| RepoPath::from_native(&located.root, &located.cwd).ok()),
                    pattern: input
                        .pattern
                        .as_deref()
                        .map(|pattern| pattern.chars().take(MAX_COMMAND_CHARS).collect()),
                    ..ToolTarget::default()
                },
            },
            ToolKind::Shell => ToolTarget {
                command: input.command.as_deref().map(ShellCommand::cut),
                ..ToolTarget::default()
            },
            ToolKind::Other => return Aim::Nothing,
        };
        Aim::Target(target)
    }

    fn result(&self, _: ToolKind, event_name: &str, payload: &Payload<'_>) -> Option<ToolResult> {
        let is_success = match self.outcome {
            Outcome::Unknown => return None,
            Outcome::PostIsSuccess { failure_event } => event_name != failure_event,
            Outcome::ResponseSuccess => payload.tool_response.as_ref()?.success?,
        };
        Some(ToolResult {
            exit_code: None,
            is_success,
            is_interrupted: false,
            output_bytes: None,
        })
    }

    fn working_folder(&self, payload: &Payload<'_>) -> Option<PathBuf> {
        let named = payload.cwd.as_deref().map(PathBuf::from);
        named.or_else(|| {
            self.folder_variable
                .and_then(std::env::var_os)
                .map(PathBuf::from)
                .filter(|folder| folder.is_absolute())
        })
    }

    fn prompt<'p>(&self, payload: &'p Payload<'_>) -> Prompted<'p> {
        match self.prompt {
            PromptField::Prompt => Prompted::User(payload.prompt.as_deref()),
            PromptField::Submitted => payload
                .submitted_prompt
                .as_deref()
                .map_or(Prompted::NotUser, |prompt| Prompted::User(Some(prompt))),
        }
    }
}

/// The file a tool's input names: its `file_path`, else its `path`.
fn file_of(input: &ToolInput) -> Option<&str> {
    input.file_path.as_deref().or(input.path.as_deref())
}
