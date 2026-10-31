//! The normalized event: what one hook call becomes, whichever agent made it.
//!
//! An agent's adapter turns its hook payload into an [`Event`] once, at the edge; everything after
//! (the daemon, the recorder) works on this type. It carries paths, line ranges, sizes and exit
//! codes, never file contents or tool output. The prompt's text travels in it to the daemon's
//! memory and is never stored as text unless the user asks for an excerpt.

mod kinds;

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use self::kinds::{Agent, CompactTrigger, HookKind, StartSource, ToolKind};
use crate::{Identifier, RepoPath, Ulid};

/// Most characters of a shell command an event carries: enough to tell which program ran and how,
/// short enough that a long script costs nothing to carry.
pub const MAX_COMMAND_CHARS: usize = 1_000;

/// One hook call, normalized.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// The event's id, made by the adapter.
    pub event_id: Ulid,
    /// When the hook process started, in UTC milliseconds by its clock.
    pub ts_ms: i64,
    /// The agent that sent the event.
    pub agent: Agent,
    /// The agent's version, when the payload or its environment says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<Identifier>,
    /// The session, as the agent names it; a subagent's events name the subagent's own session.
    pub session_id: Identifier,
    /// The session that started this one, for a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<Identifier>,
    /// The agent's working folder, canonical.
    pub cwd: PathBuf,
    /// The repository's root folder, canonical.
    pub repo_root: PathBuf,
    /// The point in the agent's loop the event comes from.
    pub hook: HookKind,
    /// Why the session started, for a session start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<StartSource>,
    /// The kind of tool, for a tool event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolKind>,
    /// What the tool aimed at, for a tool event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_target: Option<ToolTarget>,
    /// What the tool returned, after it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
    /// The prompt's text, for a prompt: kept in the daemon's memory, never stored as text unless
    /// the user asks for an excerpt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
    /// The agent's transcript of the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
    /// The model the session runs, when the payload says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Identifier>,
    /// The subagent's type, for a subagent's start and stop, as the agent names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<Identifier>,
    /// What started a compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_trigger: Option<CompactTrigger>,
    /// Why the session ended, as the agent names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<Identifier>,
}

/// What a tool call aimed at.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolTarget {
    /// The file or folder, when it is inside the repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<RepoPath>,
    /// The lines read, or the lines each edited hunk covers after the edit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<LineRange>,
    /// The search pattern, for a search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The command, for a shell tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<ShellCommand>,
}

/// What a tool call returned: how it ended and how much it printed, never what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The exit code, when the agent reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Whether the tool succeeded.
    pub is_success: bool,
    /// Whether the user interrupted the tool.
    pub is_interrupted: bool,
    /// The size of what the tool printed, in bytes, when the agent reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
}

/// Lines `start_line` to `end_line` of a file, both included, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RangeLines")]
pub struct LineRange {
    /// The first line.
    start_line: u32,
    /// The last line.
    end_line: u32,
}

/// A line range as written, before it is checked.
#[derive(Deserialize)]
struct RangeLines {
    /// The first line.
    start_line: u32,
    /// The last line.
    end_line: u32,
}

/// Why two numbers are not a line range.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("lines {start_line} to {end_line} are not a range of lines counted from 1")]
pub struct LineRangeError {
    /// The first line given.
    start_line: u32,
    /// The last line given.
    end_line: u32,
}

impl LineRange {
    /// Lines `start_line` to `end_line`, both included.
    ///
    /// # Errors
    ///
    /// The first line is 0, or the last line comes before the first.
    pub fn new(start_line: u32, end_line: u32) -> Result<Self, LineRangeError> {
        if start_line == 0 || end_line < start_line {
            return Err(LineRangeError {
                start_line,
                end_line,
            });
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }

    /// The first line.
    #[must_use]
    pub fn start_line(self) -> u32 {
        self.start_line
    }

    /// The last line.
    #[must_use]
    pub fn end_line(self) -> u32 {
        self.end_line
    }
}

impl TryFrom<RangeLines> for LineRange {
    type Error = LineRangeError;

    /// The range written, once checked.
    fn try_from(lines: RangeLines) -> Result<Self, Self::Error> {
        Self::new(lines.start_line, lines.end_line)
    }
}

/// A shell command, cut to at most [`MAX_COMMAND_CHARS`] characters.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ShellCommand(String);

/// Why a text is not a shell command an event may carry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("a command of {chars} characters is longer than {MAX_COMMAND_CHARS}")]
pub struct ShellCommandError {
    /// The text's length in characters.
    chars: usize,
}

impl ShellCommand {
    /// `text`, cut after its first [`MAX_COMMAND_CHARS`] characters.
    #[must_use]
    pub fn cut(text: &str) -> Self {
        Self(text.chars().take(MAX_COMMAND_CHARS).collect())
    }

    /// The command's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ShellCommand {
    type Error = ShellCommandError;

    /// `text` as it is, refused when it is longer than [`MAX_COMMAND_CHARS`] characters: whoever
    /// made the event cuts it.
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let chars = text.chars().count();
        if chars > MAX_COMMAND_CHARS {
            return Err(ShellCommandError { chars });
        }
        Ok(Self(text))
    }
}

impl From<ShellCommand> for String {
    /// The command's text.
    fn from(command: ShellCommand) -> Self {
        command.0
    }
}

/// A text that must not reach a log, shown as its length instead.
struct Withheld(usize);

impl Withheld {
    /// The length of `text`, in characters.
    fn of(text: &str) -> Self {
        Self(text.chars().count())
    }
}

impl fmt::Debug for Withheld {
    /// `<n characters withheld>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "<{} characters withheld>", self.0)
    }
}

impl fmt::Debug for Event {
    /// Every field, the prompt's text withheld: debug output may end up in a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Event")
            .field("event_id", &self.event_id)
            .field("ts_ms", &self.ts_ms)
            .field("agent", &self.agent)
            .field("agent_version", &self.agent_version)
            .field("session_id", &self.session_id)
            .field("parent_session_id", &self.parent_session_id)
            .field("cwd", &self.cwd)
            .field("repo_root", &self.repo_root)
            .field("hook", &self.hook)
            .field("source", &self.source)
            .field("tool", &self.tool)
            .field("tool_target", &self.tool_target)
            .field("tool_result", &self.tool_result)
            .field(
                "prompt_text",
                &self.prompt_text.as_deref().map(Withheld::of),
            )
            .field("transcript_path", &self.transcript_path)
            .field("model", &self.model)
            .field("subagent_type", &self.subagent_type)
            .field("compact_trigger", &self.compact_trigger)
            .field("end_reason", &self.end_reason)
            .finish()
    }
}

impl fmt::Debug for ToolTarget {
    /// Every field, the search pattern and the command withheld.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolTarget")
            .field("path", &self.path)
            .field("ranges", &self.ranges)
            .field("pattern", &self.pattern.as_deref().map(Withheld::of))
            .field("command", &self.command)
            .finish()
    }
}

impl fmt::Debug for ShellCommand {
    /// The command's length, its text withheld.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ShellCommand")
            .field(&Withheld::of(&self.0))
            .finish()
    }
}

#[cfg(test)]
mod tests;
