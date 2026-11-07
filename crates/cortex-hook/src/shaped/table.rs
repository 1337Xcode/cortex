//! Tools read from a table, for an agent whose hooks speak Claude Code's JSON and whose adapter is
//! built from the agent's documentation: its tool names and their kinds, a file in the input's
//! `file_path` or `path`, a search's `pattern` and folder in `pattern` and `path`, a shell's
//! `command`; where its prompt and its working folder come from; and what its tool events say of
//! how a call ended. What the documentation does not say is not guessed: a read that names an
//! offset or a limit, whose lines the documentation does not count, is not recorded; a search's
//! hits, read from a response the documentation does not describe, are none; a shell's exit code
//! is unknown.

use std::borrow::Cow;
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
    /// A tool whose kind its input's `command` decides, as OpenHands' file editor's `view` and
    /// `create`: the tool's name, the command, and the kind; looked up before `tools`.
    pub(crate) subcommands: &'static [(&'static str, &'static str, ToolKind)],
    /// Fields the agent names otherwise than Claude Code's hook JSON, as the agent names them and
    /// as Claude Code does: renamed before the payload is read, the agent's field taking the place
    /// of one the payload also holds under Claude Code's name.
    pub(crate) renames: &'static [(&'static str, &'static str)],
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
    /// The payload's `tool_is_error` field says.
    ToolIsError,
    /// The response says: one with an `error` is a failure, one without a success.
    ResponseError,
    /// The response's `exit_code` says, 0 a success; a negative code is a command not finished.
    ResponseExitCode,
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
    fn kind(&self, name: &str, payload: &Payload<'_>) -> ToolKind {
        let command = payload
            .tool_input
            .as_ref()
            .and_then(|input| input.command.as_deref());
        let by_command = self
            .subcommands
            .iter()
            .find(|(tool, subcommand, _)| *tool == name && Some(*subcommand) == command);
        if let Some((_, _, kind)) = by_command {
            return *kind;
        }
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
            ToolKind::Read
                if input.offset.is_some()
                    || input.limit.is_some()
                    || input.view_range.is_some()
                    || input.start_line.is_some()
                    || input.end_line.is_some() =>
            {
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
        let mut exit_code = None;
        let is_success = match self.outcome {
            Outcome::Unknown => return None,
            Outcome::PostIsSuccess { failure_event } => event_name != failure_event,
            Outcome::ResponseSuccess => payload.tool_response.as_ref()?.success?,
            Outcome::ToolIsError => !payload.tool_is_error?,
            Outcome::ResponseError => payload
                .tool_response
                .as_ref()?
                .error
                .as_ref()
                .is_none_or(serde_json::Value::is_null),
            Outcome::ResponseExitCode => {
                let code = i32::try_from(payload.tool_response.as_ref()?.exit_code?).ok()?;
                if code < 0 {
                    return None;
                }
                exit_code = Some(code);
                code == 0
            }
        };
        Some(ToolResult {
            exit_code,
            is_success,
            is_interrupted: false,
            output_bytes: None,
        })
    }

    fn renamed<'p>(&self, payload: &'p [u8]) -> Cow<'p, [u8]> {
        if self.renames.is_empty() {
            return Cow::Borrowed(payload);
        }
        // A payload that is no JSON object is read as it is, and refused there.
        let Ok(serde_json::Value::Object(mut fields)) = serde_json::from_slice(payload) else {
            return Cow::Borrowed(payload);
        };
        for (theirs, ours) in self.renames {
            if let Some(value) = fields.remove(*theirs) {
                fields.insert((*ours).to_owned(), value);
            }
        }
        serde_json::to_vec(&fields).map_or(Cow::Borrowed(payload), Cow::Owned)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A table renaming `conversation_id` to `session_id`, as Cursor's.
    const RENAMING: ToolTable = ToolTable {
        tools: &[],
        subcommands: &[],
        renames: &[("conversation_id", "session_id")],
        outcome: Outcome::Unknown,
        prompt: PromptField::Prompt,
        folder_variable: None,
    };

    /// The agent's field takes the place of the one under Claude Code's name, whether or not the
    /// payload holds both; a payload that is no object is read as it is.
    #[test]
    fn the_agent_s_field_takes_the_place_of_claude_code_s() -> Result<(), serde_json::Error> {
        let read = |payload: &str| -> Result<serde_json::Value, serde_json::Error> {
            serde_json::from_slice(&RENAMING.renamed(payload.as_bytes()))
        };
        assert_eq!(
            read(r#"{"conversation_id":"c1","session_id":"s1"}"#)?,
            serde_json::json!({"session_id": "c1"})
        );
        assert_eq!(
            read(r#"{"conversation_id":"c1"}"#)?,
            serde_json::json!({"session_id": "c1"})
        );
        assert_eq!(RENAMING.renamed(b"[1]").as_ref(), b"[1]");
        Ok(())
    }
}
