//! The hook payload Claude Code defined, as much of it as Cortex reads. Every field is optional and read
//! leniently: a field of another type than expected reads as absent, so one odd field never costs
//! the event. The fields Cortex does not read are skipped unread: file contents, edits' old and new
//! text, the assistant's messages. The one file text read, an edit's `originalFile`, is read only
//! to be hashed and is dropped with the payload. A shell's output is measured, never kept. A
//! response's `content` is held as the payload's own bytes, unread, and only a search's is then
//! read for the paths it names, never for the text it matched.

use std::borrow::Cow;
use std::fmt;

use cortex_core::MAX_HITS;
use serde::Deserialize;
use serde::de::{Deserializer, Visitor};
use serde_json::value::RawValue;

/// The fields of a payload Cortex reads, from every event, borrowing from the payload's bytes.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Payload<'a> {
    /// The session.
    #[serde(deserialize_with = "lenient")]
    pub(crate) session_id: Option<String>,
    /// The session's transcript.
    #[serde(deserialize_with = "lenient")]
    pub(crate) transcript_path: Option<String>,
    /// The agent's working folder.
    #[serde(deserialize_with = "lenient")]
    pub(crate) cwd: Option<String>,
    /// Why a session started.
    #[serde(deserialize_with = "lenient")]
    pub(crate) source: Option<String>,
    /// The model, sent at some session starts.
    #[serde(deserialize_with = "lenient")]
    pub(crate) model: Option<String>,
    /// The prompt's text.
    #[serde(deserialize_with = "lenient")]
    pub(crate) prompt: Option<String>,
    /// The tool's name.
    #[serde(deserialize_with = "lenient")]
    pub(crate) tool_name: Option<String>,
    /// Whether the tool's call failed, as Augment's payloads say.
    #[serde(deserialize_with = "lenient")]
    pub(crate) tool_is_error: Option<bool>,
    /// Which model call of its turn the event comes before, from 0, as Antigravity's hook before
    /// each model call says.
    #[serde(deserialize_with = "lenient")]
    pub(crate) invocation_num: Option<i64>,
    /// The prompt as the user typed it, where an agent gives it apart from `prompt`, which may
    /// be a send of its own (Qwen Code's tool results).
    #[serde(deserialize_with = "lenient")]
    pub(crate) submitted_prompt: Option<String>,
    /// The id of the tool call, which Codex's rollout names the call's result by.
    #[serde(deserialize_with = "lenient")]
    pub(crate) tool_use_id: Option<String>,
    /// What the tool was given.
    #[serde(deserialize_with = "lenient")]
    pub(crate) tool_input: Option<ToolInput>,
    /// What the tool gave back.
    #[serde(borrow, deserialize_with = "response")]
    pub(crate) tool_response: Option<ToolResponse<'a>>,
    /// A failed tool's message, a shell's beginning `Exit code N`; at a stop, the error the turn
    /// ended on (`rate_limit`, `billing_error`, ...). Kept as given, since a value that is not
    /// text still says that something failed.
    #[serde(deserialize_with = "lenient")]
    pub(crate) error: Option<serde_json::Value>,
    /// Whether the user interrupted the failed tool.
    #[serde(deserialize_with = "lenient")]
    pub(crate) is_interrupt: Option<bool>,
    /// What started a compaction.
    #[serde(deserialize_with = "lenient")]
    pub(crate) trigger: Option<String>,
    /// The subagent the event comes from, if any.
    #[serde(deserialize_with = "lenient")]
    pub(crate) agent_id: Option<String>,
    /// That subagent's type; empty for Claude Code's own agents.
    #[serde(deserialize_with = "lenient")]
    pub(crate) agent_type: Option<String>,
    /// That subagent's own transcript.
    #[serde(deserialize_with = "lenient")]
    pub(crate) agent_transcript_path: Option<String>,
    /// Why the session ended.
    #[serde(deserialize_with = "lenient")]
    pub(crate) reason: Option<String>,
    /// Whether the call is `cortex doctor`'s probe rather than the agent's.
    #[serde(deserialize_with = "lenient")]
    pub(crate) cortex_probe: Option<bool>,
}

/// The one field of a payload that says whether the call is `cortex doctor`'s probe.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProbeMark {
    /// Whether the call is a probe.
    #[serde(deserialize_with = "lenient")]
    cortex_probe: Option<bool>,
}

/// Whether `payload`, a hook's JSON (a byte-order mark before it allowed), may be `cortex
/// doctor`'s probe: it says so, or it is no JSON the hook can read.
pub(crate) fn may_be_probe(payload: &[u8]) -> bool {
    let payload = payload.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(payload);
    serde_json::from_slice::<ProbeMark>(payload)
        .map_or(true, |mark| mark.cortex_probe == Some(true))
}

/// What a tool was given.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ToolInput {
    /// The file read, edited or written.
    #[serde(deserialize_with = "lenient")]
    pub(crate) file_path: Option<String>,
    /// The file or folder searched.
    #[serde(deserialize_with = "lenient")]
    pub(crate) path: Option<String>,
    /// The search pattern.
    #[serde(deserialize_with = "lenient")]
    pub(crate) pattern: Option<String>,
    /// The first line to read, counted from 1.
    #[serde(deserialize_with = "lenient")]
    pub(crate) offset: Option<i64>,
    /// How many lines to read.
    #[serde(deserialize_with = "lenient")]
    pub(crate) limit: Option<i64>,
    /// The shell command.
    #[serde(deserialize_with = "lenient")]
    pub(crate) command: Option<String>,
    /// Whether the shell runs in the background: its result then comes before it has finished.
    #[serde(deserialize_with = "lenient")]
    pub(crate) run_in_background: Option<bool>,
    /// The lines a view names, as OpenHands' file editor takes them: a range whose counting
    /// Cortex does not read, held only to tell that one is named.
    #[serde(deserialize_with = "lenient")]
    pub(crate) view_range: Option<serde_json::Value>,
    /// The first line a read names, as Gemini CLI's `read_file` takes it.
    #[serde(deserialize_with = "lenient")]
    pub(crate) start_line: Option<serde_json::Value>,
    /// The last line a read names, as Gemini CLI's `read_file` takes it.
    #[serde(deserialize_with = "lenient")]
    pub(crate) end_line: Option<serde_json::Value>,
}

/// What a tool gave back.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ToolResponse<'a> {
    /// For a read, which lines of the file came back.
    #[serde(deserialize_with = "lenient")]
    pub(crate) file: Option<ReadFile>,
    /// For an edit or a write, the stretches of lines it changed.
    #[serde(rename = "structuredPatch", deserialize_with = "lenient")]
    pub(crate) structured_patch: Option<Vec<Hunk>>,
    /// For an edit or a write of a file that existed, the whole file as it was before: read only
    /// to be hashed, never kept.
    #[serde(rename = "originalFile", borrow, deserialize_with = "lenient")]
    pub(crate) original_file: Option<Cow<'a, str>>,
    /// The size of a shell's standard output.
    #[serde(deserialize_with = "lenient")]
    pub(crate) stdout: Option<TextLength>,
    /// The size of a shell's standard error.
    #[serde(deserialize_with = "lenient")]
    pub(crate) stderr: Option<TextLength>,
    /// The error a failed call reports, as Gemini CLI's responses carry it.
    #[serde(deserialize_with = "lenient")]
    pub(crate) error: Option<serde_json::Value>,
    /// The command's exit code, as OpenHands' terminal responses say.
    #[serde(deserialize_with = "lenient")]
    pub(crate) exit_code: Option<i64>,
    /// Whether the call succeeded, as Devin's responses say.
    #[serde(deserialize_with = "lenient")]
    pub(crate) success: Option<bool>,
    /// Whether the user interrupted the shell.
    #[serde(deserialize_with = "lenient")]
    pub(crate) interrupted: Option<bool>,
    /// The files a search found: what Glob lists, and Grep in its files mode.
    #[serde(deserialize_with = "lenient")]
    pub(crate) filenames: Option<Vec<String>>,
    /// Grep's output in its content and count modes, a written file's text, or the whole response
    /// of an agent that gives a tool's output as one text, as Codex does: the payload's bytes,
    /// unread.
    #[serde(borrow, deserialize_with = "lenient")]
    pub(crate) content: Option<&'a RawValue>,
}

/// The lines a read returned.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ReadFile {
    /// The first line returned, counted from 1.
    #[serde(deserialize_with = "lenient")]
    pub(crate) start_line: Option<i64>,
    /// How many lines came back.
    #[serde(deserialize_with = "lenient")]
    pub(crate) num_lines: Option<i64>,
    /// How many lines the file has.
    #[serde(deserialize_with = "lenient")]
    pub(crate) total_lines: Option<i64>,
}

/// One stretch of an edit's diff: the changed lines and up to three unchanged ones on each side,
/// where it starts in the file before the edit and in the file after it.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Hunk {
    /// The stretch's first line in the file before the edit.
    #[serde(deserialize_with = "lenient")]
    pub(crate) old_start: Option<i64>,
    /// The stretch's first line in the file after the edit.
    #[serde(deserialize_with = "lenient")]
    pub(crate) new_start: Option<i64>,
    /// How many lines it has.
    #[serde(deserialize_with = "lenient")]
    pub(crate) new_lines: Option<i64>,
    /// What each line of the diff does, read from its first character only.
    #[serde(deserialize_with = "lenient")]
    pub(crate) lines: Option<Vec<LineMark>>,
}

/// What one line of a diff does, as its first character says. The rest of the line, the code, is
/// never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineMark {
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
/// borrowed from the payload unparsed, then parsed as `T`, which may borrow from it in turn: a
/// mismatch is dropped with the field alone.
///
/// # Errors
///
/// Only when the payload is not JSON at all.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let raw = <&RawValue>::deserialize(deserializer)?;
    Ok(serde_json::from_str(raw.get()).ok())
}

/// Reads a tool's response: an object as [`ToolResponse`]; a string, a tool's whole output as one
/// text, as its `content`; anything else as absent.
///
/// # Errors
///
/// Only when the payload is not JSON at all.
fn response<'de, D>(deserializer: D) -> Result<Option<ToolResponse<'de>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = <&RawValue>::deserialize(deserializer)?;
    if raw.get().starts_with('"') {
        return Ok(Some(ToolResponse {
            content: Some(raw),
            ..ToolResponse::default()
        }));
    }
    Ok(serde_json::from_str(raw.get()).ok())
}

/// The size of a text in bytes, read without keeping the text.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TextLength(pub(crate) u64);

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

/// The paths Grep's output `content` names, at most [`MAX_HITS`], each once, in order; none when
/// it is not text. The text it matched is dropped as it is read.
pub(crate) fn content_paths(content: &RawValue) -> Vec<String> {
    serde_json::from_str::<ContentPaths>(content.get()).map_or_else(|_| Vec::new(), |paths| paths.0)
}

/// The paths of Grep's output, as [`content_paths`] gives them.
struct ContentPaths(Vec<String>);

impl<'de> Deserialize<'de> for ContentPaths {
    /// The paths of a string's lines; anything else is not an output.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(PathsVisitor)
    }
}

/// Reads the paths of an output's lines as the string is read.
struct PathsVisitor;

impl Visitor<'_> for PathsVisitor {
    type Value = ContentPaths;

    /// What the visitor reads.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string")
    }

    /// The paths the lines start with.
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<ContentPaths, E> {
        let mut paths: Vec<String> = Vec::new();
        for path in text.lines().filter_map(line_path) {
            if paths.len() == MAX_HITS {
                break;
            }
            if !paths.iter().any(|seen| seen == path) {
                paths.push(path.to_owned());
            }
        }
        Ok(ContentPaths(paths))
    }
}

/// The path a line of Grep's output starts with: the text before its first colon (a Windows
/// drive's aside, as in `C:\repo\a.py`), when a line number follows that colon and then a colon
/// (a match line, `calc/ops.py:12:def divide`) or the line's end (a count line, `calc/ops.py:3`).
/// Every other line names none: a context line, whose path ends at a `-11-` before any colon
/// (`calc/ops.py-11-# divides`), whatever colons its text holds after; a line without its number
/// (`ops.py:  at 10:30:00`, with `-n` off); and a line that starts with its number, the output of
/// a search of one file, which prints no file name. A file whose name holds `-11-` is missed rather
/// than a wrong file named. Two lines cannot be told from a match or count line and name the text
/// before their first colon: one of a file whose name holds `:3:`, possible on Unix only, and one
/// of a search of one file with `-n` off whose text reads like a count (`port:80`).
fn line_path(line: &str) -> Option<&str> {
    if number_then(line, &[':', '-']) {
        return None;
    }
    let drive = if has_drive(line) { 2 } else { 0 };
    let colon = drive + line.get(drive..)?.find(':')?;
    let path = line.get(..colon).filter(|path| !path.is_empty())?;
    let after = line.get(colon + 1..)?;
    let is_numbered = number_then(after, &[':']) || after.bytes().all(|byte| byte.is_ascii_digit());
    let is_context = path
        .match_indices('-')
        .any(|(at, _)| number_then(path.get(at + 1..).unwrap_or_default(), &['-']));
    (is_numbered && !after.is_empty() && !is_context).then_some(path)
}

/// Whether `text` starts with a number followed by one of `marks`.
fn number_then(text: &str, marks: &[char]) -> bool {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && text
            .get(digits..)
            .is_some_and(|rest| rest.starts_with(marks))
}

/// Whether `line` starts with a Windows drive, a letter, a colon and a slash either way.
fn has_drive(line: &str) -> bool {
    matches!(line.as_bytes(), [letter, b':', b'\\' | b'/', ..] if letter.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload saying it is a probe, or one that is no JSON at all, may be a probe; one that
    /// says nothing, or that it is none, or names the field leniently with another type, is not.
    #[test]
    fn a_probe_is_told_by_its_field() {
        assert!(may_be_probe(br#"{"cortex_probe": true, "cwd": "x"}"#));
        assert!(may_be_probe(b"\xEF\xBB\xBF{\"cortex_probe\": true}"));
        assert!(may_be_probe(b"not json"));
        assert!(!may_be_probe(br#"{"cwd": "x"}"#));
        assert!(!may_be_probe(br#"{"cortex_probe": false}"#));
        assert!(!may_be_probe(br#"{"cortex_probe": "yes"}"#));
    }

    /// Grep's output is read for 50 files at most, each once, so a long output costs no more:
    /// 60 match lines give the first 50 files, and a file's second line adds nothing; output that
    /// is not text names none.
    #[test]
    fn output_is_read_for_fifty_files_at_most() -> Result<(), serde_json::Error> {
        let raw = |value: serde_json::Value| serde_json::value::to_raw_value(&value);
        let lines: Vec<String> = (0..60).map(|number| format!("f{number}.py:1:x")).collect();
        let first_fifty: Vec<String> = (0..50).map(|number| format!("f{number}.py")).collect();
        assert_eq!(content_paths(&raw(lines.join("\n").into())?), first_fifty);
        assert_eq!(content_paths(&raw("a.py:1:x\na.py:2:y".into())?), ["a.py"]);
        assert_eq!(
            content_paths(&raw(serde_json::json!(42))?),
            Vec::<String>::new()
        );
        Ok(())
    }

    /// A response given as one text is held as its `content`, unread, and read for the paths it
    /// names; an object is read field by field; any other value is no response.
    #[test]
    fn a_response_given_as_text_is_its_content() -> Result<(), serde_json::Error> {
        let read = |response: &str| -> Result<Option<Vec<String>>, serde_json::Error> {
            let text = format!(r#"{{"tool_response":{response}}}"#);
            let payload: Payload<'_> = serde_json::from_str(&text)?;
            Ok(payload
                .tool_response
                .map(|response| response.content.map(content_paths).unwrap_or_default()))
        };
        assert_eq!(
            read(r#""calc/ops.py:4:def add(a, b):\nmain.py:3:print(add(1, 2))\n""#)?,
            Some(vec!["calc/ops.py".to_owned(), "main.py".to_owned()])
        );
        assert_eq!(read(r#""""#)?, Some(Vec::new()));
        assert_eq!(read(r#"{"filenames":["a.py"]}"#)?, Some(Vec::new()));
        assert_eq!(read("42")?, None);
        assert_eq!(read("null")?, None);
        Ok(())
    }

    /// Each kind of line Grep prints names its file or none: a match or count line its file, even
    /// when its text holds more numbers and colons, or the file's name dashes; a context line, a
    /// separator, a line without its number, whatever its text holds, and the lines of a one-file
    /// search none; a name holding `-2-` none rather than a part of it.
    #[test]
    fn each_line_names_its_file_or_none() {
        let cases = [
            ("calc/ops.py:12:def divide(a, b):", Some("calc/ops.py")),
            ("calc/ops.py:3", Some("calc/ops.py")),
            ("tests/time.py:7:at 10:30:00", Some("tests/time.py")),
            (
                "C:\\repo\\calc\\ops.py:12:x",
                Some("C:\\repo\\calc\\ops.py"),
            ),
            ("C:/repo/main.py:3", Some("C:/repo/main.py")),
            ("v2-notes.md:3:x", Some("v2-notes.md")),
            (
                "src/my-component/index.ts:8:x-1-2",
                Some("src/my-component/index.ts"),
            ),
            ("notes.md-5-meeting at 10:30", None),
            ("context.py-11-# divides", None),
            ("--", None),
            ("no_numbers.py:def x():", None),
            ("ops.py:  at 10:30:00", None),
            ("cfg.py:  x = a?1:2", None),
            ("main.py:", None),
            ("main.py:3x", None),
            ("12:def divide(a, b):", None),
            ("7:at 10:30:00", None),
            ("12:34:56", None),
            ("11-at 10:30", None),
            ("11-# divides", None),
            ("3", None),
            ("release-2-notes.md:3:x", None),
            (":12:x", None),
            ("C:", None),
            ("", None),
        ];
        for (line, path) in cases {
            assert_eq!(line_path(line), path, "{line}");
        }
    }
}
