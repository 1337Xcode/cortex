//! Claude Code's hook payload, as much of it as Cortex reads. Every field is optional and read
//! leniently: a field of another type than expected reads as absent, so one odd field never costs
//! the event. The fields Cortex does not read are skipped unread: file contents, edits' old and new
//! text, the assistant's messages. A shell's output is measured, never kept.

use std::fmt;

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer, Visitor};
use serde_json::value::RawValue;

/// The fields of a payload Cortex reads, from every event.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Payload {
    /// The session.
    #[serde(deserialize_with = "lenient")]
    pub(super) session_id: Option<String>,
    /// The session's transcript.
    #[serde(deserialize_with = "lenient")]
    pub(super) transcript_path: Option<String>,
    /// The agent's working folder.
    #[serde(deserialize_with = "lenient")]
    pub(super) cwd: Option<String>,
    /// Why a session started.
    #[serde(deserialize_with = "lenient")]
    pub(super) source: Option<String>,
    /// The model, sent at some session starts.
    #[serde(deserialize_with = "lenient")]
    pub(super) model: Option<String>,
    /// The prompt's text.
    #[serde(deserialize_with = "lenient")]
    pub(super) prompt: Option<String>,
    /// The tool's name.
    #[serde(deserialize_with = "lenient")]
    pub(super) tool_name: Option<String>,
    /// What the tool was given.
    #[serde(deserialize_with = "lenient")]
    pub(super) tool_input: Option<ToolInput>,
    /// What the tool gave back.
    #[serde(deserialize_with = "lenient")]
    pub(super) tool_response: Option<ToolResponse>,
    /// A failed tool's message; a shell's begins `Exit code N`.
    #[serde(deserialize_with = "lenient")]
    pub(super) error: Option<String>,
    /// Whether the user interrupted the failed tool.
    #[serde(deserialize_with = "lenient")]
    pub(super) is_interrupt: Option<bool>,
    /// What started a compaction.
    #[serde(deserialize_with = "lenient")]
    pub(super) trigger: Option<String>,
    /// The subagent the event comes from, if any.
    #[serde(deserialize_with = "lenient")]
    pub(super) agent_id: Option<String>,
    /// That subagent's type; empty for Claude Code's own agents.
    #[serde(deserialize_with = "lenient")]
    pub(super) agent_type: Option<String>,
    /// That subagent's own transcript.
    #[serde(deserialize_with = "lenient")]
    pub(super) agent_transcript_path: Option<String>,
    /// Why the session ended.
    #[serde(deserialize_with = "lenient")]
    pub(super) reason: Option<String>,
}

/// What a tool was given.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct ToolInput {
    /// The file read, edited or written.
    #[serde(deserialize_with = "lenient")]
    pub(super) file_path: Option<String>,
    /// The file or folder searched.
    #[serde(deserialize_with = "lenient")]
    pub(super) path: Option<String>,
    /// The search pattern.
    #[serde(deserialize_with = "lenient")]
    pub(super) pattern: Option<String>,
    /// The first line to read, counted from 1.
    #[serde(deserialize_with = "lenient")]
    pub(super) offset: Option<i64>,
    /// How many lines to read.
    #[serde(deserialize_with = "lenient")]
    pub(super) limit: Option<i64>,
    /// The shell command.
    #[serde(deserialize_with = "lenient")]
    pub(super) command: Option<String>,
    /// Whether the shell runs in the background: its result then comes before it has finished.
    #[serde(deserialize_with = "lenient")]
    pub(super) run_in_background: Option<bool>,
}

/// What a tool gave back.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct ToolResponse {
    /// For a read, which lines of the file came back.
    #[serde(deserialize_with = "lenient")]
    pub(super) file: Option<ReadFile>,
    /// For an edit or a write, the stretches of lines it changed.
    #[serde(rename = "structuredPatch", deserialize_with = "lenient")]
    pub(super) structured_patch: Option<Vec<Hunk>>,
    /// The size of a shell's standard output.
    #[serde(deserialize_with = "lenient")]
    pub(super) stdout: Option<TextLength>,
    /// The size of a shell's standard error.
    #[serde(deserialize_with = "lenient")]
    pub(super) stderr: Option<TextLength>,
    /// Whether the user interrupted the shell.
    #[serde(deserialize_with = "lenient")]
    pub(super) interrupted: Option<bool>,
}

/// The lines a read returned.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct ReadFile {
    /// The first line returned, counted from 1.
    #[serde(deserialize_with = "lenient")]
    pub(super) start_line: Option<i64>,
    /// How many lines came back.
    #[serde(deserialize_with = "lenient")]
    pub(super) num_lines: Option<i64>,
    /// How many lines the file has.
    #[serde(deserialize_with = "lenient")]
    pub(super) total_lines: Option<i64>,
}

/// One stretch of an edit's diff, in the file as it is after the edit: the changed lines and up to
/// three unchanged ones on each side.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Hunk {
    /// The stretch's first line.
    #[serde(deserialize_with = "lenient")]
    pub(super) new_start: Option<i64>,
    /// How many lines it has.
    #[serde(deserialize_with = "lenient")]
    pub(super) new_lines: Option<i64>,
    /// What each line of the diff does, read from its first character only.
    #[serde(deserialize_with = "lenient")]
    pub(super) lines: Option<Vec<LineMark>>,
}

/// What one line of a diff does, as its first character says. The rest of the line, the code, is
/// never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineMark {
    /// ` `: a line the edit left as it was.
    Unchanged,
    /// `+`: a line the edit added.
    Added,
    /// `-`: a line the edit removed.
    Removed,
    /// Anything else, such as `\ No newline at end of file`.
    Other,
}

impl<'de> Deserialize<'de> for LineMark {
    /// The mark of a diff line; anything but a string is not one.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(MarkVisitor)
    }
}

/// Reads a diff line's first character.
struct MarkVisitor;

impl Visitor<'_> for MarkVisitor {
    type Value = LineMark;

    /// What the visitor reads.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a line of a diff")
    }

    /// The mark of the line `text`.
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<LineMark, E> {
        Ok(match text.as_bytes().first() {
            Some(b' ') => LineMark::Unchanged,
            Some(b'+') => LineMark::Added,
            Some(b'-') => LineMark::Removed,
            _ => LineMark::Other,
        })
    }
}

/// Reads a field as `T`, or as absent when it is `null` or of another type. The field's JSON is
/// borrowed from the payload unparsed, then parsed as `T`: a mismatch is dropped with the field
/// alone.
///
/// # Errors
///
/// Only when the payload is not JSON at all.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let raw = <&RawValue>::deserialize(deserializer)?;
    Ok(serde_json::from_str(raw.get()).ok())
}

/// The size of a text in bytes, read without keeping the text.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct TextLength(pub(super) u64);

impl<'de> Deserialize<'de> for TextLength {
    /// The length of a string; anything else is not a text.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(LengthVisitor)
    }
}

/// Measures a string as it is read.
struct LengthVisitor;

impl Visitor<'_> for LengthVisitor {
    type Value = TextLength;

    /// What the visitor reads.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string")
    }

    /// The string's length in bytes.
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<TextLength, E> {
        Ok(TextLength(u64::try_from(text.len()).unwrap_or(u64::MAX)))
    }
}
