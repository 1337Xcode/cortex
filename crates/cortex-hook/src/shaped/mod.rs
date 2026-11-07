//! What agents whose hooks speak the JSON Claude Code defined share: a payload on stdin with the
//! session, the working folder, the event's fields and the tool's input and response; one event
//! made of it; and text given back as `hookSpecificOutput.additionalContext`. Each such agent is an
//! adapter of its own, its hooks table and its [`Tools`]: how it names its tools, what a call aimed
//! at, and how it ended.

pub(crate) mod normalize;
pub(crate) mod output;
pub(crate) mod payload;
pub(crate) mod place;
pub(crate) mod table;

use std::path::PathBuf;

use cortex_core::{HookKind, ToolKind, ToolResult, ToolTarget};

use crate::adapter::Located;
use payload::Payload;

/// What a tool call aimed at.
pub(crate) enum Aim {
    /// Nothing Cortex records: a tool of another kind, or a call with no input.
    Nothing,
    /// A target inside the repository, or one with no file at all, such as a shell's command.
    Target(ToolTarget),
    /// A file or folder outside the repository: the call is none of the repository's business.
    Outside,
}

/// Whose a prompt event is.
pub(crate) enum Prompted<'p> {
    /// The user's, with its text when the payload gives it.
    User(Option<&'p str>),
    /// Not the user's: an agent that also runs its prompt hook for its own sends, such as a
    /// tool's result going back to the model. No event is made of it.
    NotUser,
}

/// How one agent's tools read: the agent's names for them, what a call aimed at, and how it ended;
/// and, where the agent differs from Claude Code, where it works and whose a prompt is.
pub(crate) trait Tools: Sync {
    /// The kind of the tool the agent names `name`, called as `payload` says.
    fn kind(&self, name: &str, payload: &Payload<'_>) -> ToolKind;

    /// What a call of `tool` at `hook` aimed at, `located` being where the agent works.
    fn aim(&self, tool: ToolKind, hook: HookKind, payload: &Payload<'_>, located: &Located) -> Aim;

    /// How a call of `tool`, reported at the agent's event `event_name`, ended: none when the
    /// agent does not tell its hooks.
    fn result(&self, tool: ToolKind, event_name: &str, payload: &Payload<'_>)
    -> Option<ToolResult>;

    /// The folder the agent works in: the payload's `cwd`.
    fn working_folder(&self, payload: &Payload<'_>) -> Option<PathBuf> {
        payload.cwd.as_deref().map(PathBuf::from)
    }

    /// Whose the prompt event `payload` is: the user's, its text the payload's `prompt`.
    fn prompt<'p>(&self, payload: &'p Payload<'_>) -> Prompted<'p> {
        Prompted::User(payload.prompt.as_deref())
    }
}
