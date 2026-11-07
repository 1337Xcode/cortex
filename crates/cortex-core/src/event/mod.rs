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

/// Most line ranges a tool target holds. An edit that changed more stretches than this, such as a
/// file rewritten throughout, is an edit of the whole file and holds none; the bound also keeps an
/// event small.
pub const MAX_RANGES: usize = 16;

/// Most files a search's hits name on an event: more than a task opens next from one search, few
/// enough that an event stays small.
pub const MAX_HITS: usize = 50;

/// Most bytes the paths of a search's hits hold together: room for [`MAX_HITS`] paths of over 80
/// bytes, and a bound that keeps an event a few kilobytes however long the files' names are, in a
/// frame and in the daemon's queue of events.
pub const MAX_HITS_BYTES: usize = 4 * 1024;

/// Most files one edit names on an event, the first and the others together: a patch of more
/// files (Codex's `apply_patch` can change any number) is recorded for its first 32, enough for
/// any change a task makes in one step, few enough that an event stays small.
pub const MAX_EDITED_FILES: usize = 32;

/// The fewest characters other than spaces a prompt holds to ask for code: four, so that "ok",
/// "yes" and "go" ask for nothing while a file's name alone, `a.py`, still may.
pub const MIN_WORK_PROMPT_CHARS: usize = 4;

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
    /// Whether the event is a probe: `cortex doctor`'s synthetic hook call, which a daemon answers
    /// as it answers the agent's and does not record.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_probe: bool,
}

impl Event {
    /// The file this event says a tool changed: a tool's result, successful, of a tool that edits,
    /// naming a file inside the repository.
    #[must_use]
    pub fn edited_file(&self) -> Option<&RepoPath> {
        let is_edit = self.tool.is_some_and(ToolKind::is_edit);
        let is_success = self.tool_result.is_some_and(|result| result.is_success);
        if self.hook != HookKind::PostTool || !is_edit || !is_success {
            return None;
        }
        self.tool_target.as_ref()?.path.as_ref()
    }

    /// The file this event says a tool is about to read: the moment before a read runs, naming a
    /// file inside the repository.
    #[must_use]
    pub fn file_to_read(&self) -> Option<&RepoPath> {
        if self.hook != HookKind::PreTool || self.tool != Some(ToolKind::Read) {
            return None;
        }
        self.tool_target.as_ref()?.path.as_ref()
    }

    /// The prompt this event submits when it may ask for code: none for an event that is no
    /// prompt, a slash command (a first word of `/` and a name, such as `/review`; a path such as
    /// `/usr/lib/x.py` is no command), a shell escape (`!` first), or a prompt of fewer than
    /// [`MIN_WORK_PROMPT_CHARS`] characters other than spaces, such as "ok" or "yes", which names
    /// nothing to look for. Such a prompt is still recorded: it starts a task.
    #[must_use]
    pub fn prompt_for_work(&self) -> Option<&str> {
        if self.hook != HookKind::Prompt {
            return None;
        }
        let prompt = self.prompt_text.as_deref()?;
        let start = prompt.trim_start();
        let first_word = start.split_whitespace().next().unwrap_or_default();
        let is_slash_command = first_word
            .strip_prefix('/')
            .is_some_and(|name| !name.contains('/'));
        let is_command = is_slash_command || start.starts_with('!');
        let chars = prompt.chars().filter(|c| !c.is_whitespace()).count();
        (!is_command && chars >= MIN_WORK_PROMPT_CHARS).then_some(prompt)
    }
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
    /// For a read that asked for the whole file and was cut short by the agent's tool, the last
    /// line it returned: the read is still the whole file's, which it has no ranges for, and the
    /// agent holds its lines up to this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_after: Option<u32>,
    /// The search pattern, for a search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The command, for a shell tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<ShellCommand>,
    /// The files a search found inside the repository, in the order the tool listed them, each once:
    /// at most [`MAX_HITS`] of them and [`MAX_HITS_BYTES`] of paths. They travel to the daemon with
    /// the event and are never stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<RepoPath>,
    /// For an edit of several files, the files it changed after [`ToolTarget::path`], each once, in
    /// the order the edit names them: at most [`MAX_EDITED_FILES`] files with the first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_paths: Vec<RepoPath>,
}

impl ToolTarget {
    /// Every file the target names as changed or read: [`ToolTarget::path`], then
    /// [`ToolTarget::more_paths`].
    pub fn paths(&self) -> impl Iterator<Item = &RepoPath> {
        self.path.iter().chain(&self.more_paths)
    }
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

    /// Whether this range and `other` share a line: lines 3 to 5 and 5 to 9 do, 3 to 5 and 6 to 9
    /// do not.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.start_line <= other.end_line && other.start_line <= self.end_line
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
            .field("is_probe", &self.is_probe)
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
            .field("cut_after", &self.cut_after)
            .field("pattern", &self.pattern.as_deref().map(Withheld::of))
            .field("command", &self.command)
            .field("hits", &self.hits)
            .field("more_paths", &self.more_paths)
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
