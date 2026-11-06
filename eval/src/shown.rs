//! The code an agent was shown: every numbered line of the work orders in a session's transcript,
//! and whether each is the line its file held when the session began.
//!
//! A work order writes its code under `Current code:`, each span as its path and lines, then each
//! line as `  N | text`; a line of `| ...` marks lines left out between a definition's first line
//! and the span, and `  ... lines A-B not shown` the end of a span that was cut. The runner reads
//! the work orders back from the agent's own transcript, so the check covers what reached the
//! agent, after every step between Cortex and the model.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The words a work order starts with.
const WORK_ORDER: &str = "[cortex] Work order";

/// The line a work order's code follows.
const CODE_HEADING: &str = "Current code:";

/// What the check of the code shown in a run found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shown {
    /// The distinct code lines the run's sessions were shown.
    pub(crate) lines: u64,
    /// Of those, the ones that were not the line their file held when the session began.
    pub(crate) differ: u64,
    /// Sessions whose transcript could not be found or read, so whose code was not checked.
    pub(crate) unread_sessions: u64,
}

impl Shown {
    /// The sum of `self` and `other`.
    #[must_use]
    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            lines: self.lines + other.lines,
            differ: self.differ + other.differ,
            unread_sessions: self.unread_sessions + other.unread_sessions,
        }
    }
}

/// One numbered code line a work order showed: its file, its line number and its text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ShownLine {
    /// The file's path, as the work order wrote it.
    pub(crate) path: String,
    /// The line's number, from 1.
    pub(crate) number: u32,
    /// The text shown for it.
    pub(crate) text: String,
}

/// Every distinct code line the work orders in `transcript` (an agent's JSON-lines transcript)
/// showed, in path and line order. A line of the transcript that is not JSON is passed over.
#[must_use]
pub(crate) fn shown_lines(transcript: &str) -> Vec<ShownLine> {
    let mut found = BTreeSet::new();
    for line in transcript.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let mut texts = Vec::new();
        strings_of(&value, &mut texts);
        for text in texts {
            for (at, _) in text.match_indices(WORK_ORDER) {
                if let Some(order) = text.get(at..) {
                    read_code(order, &mut found);
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Pushes every string inside `value` onto `texts`.
fn strings_of<'a>(value: &'a Value, texts: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => texts.push(text),
        Value::Array(values) => values.iter().for_each(|value| strings_of(value, texts)),
        Value::Object(map) => map.values().for_each(|value| strings_of(value, texts)),
        _ => {}
    }
}

/// Adds to `found` the code lines of the work order `order` starts with: those under its
/// `Current code:` line, up to the first line that is not indented.
fn read_code(order: &str, found: &mut BTreeSet<ShownLine>) {
    let mut lines = order.lines().skip_while(|line| *line != CODE_HEADING);
    lines.next();
    let mut path: Option<&str> = None;
    for line in lines {
        let Some(rest) = line.strip_prefix("  ") else {
            break;
        };
        if let Some((number, text)) = numbered(rest) {
            if let Some(path) = path {
                found.insert(ShownLine {
                    path: path.to_owned(),
                    number,
                    text: text.to_owned(),
                });
            }
        } else if let Some(named) = span_path(rest) {
            path = Some(named);
        }
    }
}

/// The number and text of a code line, `rest` being the line without its first two spaces: the
/// number right-aligned, then ` | ` and the text. None for any other line.
fn numbered(rest: &str) -> Option<(u32, &str)> {
    let (number, text) = rest.trim_start().split_once(" |")?;
    let number = number.parse().ok()?;
    // The space after the bar is the work order's; a line with no text may have lost it.
    Some((number, text.strip_prefix(' ').unwrap_or(text)))
}

/// The path of a span's first line, `path:start-end`, `rest` being the line without its first two
/// spaces. None for any other line.
fn span_path(rest: &str) -> Option<&str> {
    let (path, lines) = rest.rsplit_once(':')?;
    let (start, end) = lines.split_once('-')?;
    let is_number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    (is_number(start) && is_number(end) && !path.is_empty()).then_some(path)
}

/// Checks each of `lines` against its file as `file_of` gives it (none for a file that did not
/// exist), reading each file once. Returns the count, and the path and number of each line that
/// differs.
///
/// A shown line matches the file's line when it is the same text, or the same once the file's
/// control characters other than tab are left out, as a work order leaves them out.
///
/// # Errors
///
/// `file_of`'s errors.
pub(crate) fn check(
    lines: &[ShownLine],
    mut file_of: impl FnMut(&str) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<(Shown, Vec<(String, u32)>)> {
    let mut by_path: BTreeMap<&str, Vec<&ShownLine>> = BTreeMap::new();
    for line in lines {
        by_path.entry(line.path.as_str()).or_default().push(line);
    }
    let mut shown = Shown::default();
    let mut differing = Vec::new();
    for (path, lines) in by_path {
        let file = file_of(path)?;
        for line in lines {
            shown.lines += 1;
            let held = file.as_deref().and_then(|file| {
                let at = usize::try_from(line.number).ok()?.checked_sub(1)?;
                file.lines().nth(at)
            });
            if !held.is_some_and(|held| is_same(&line.text, held)) {
                shown.differ += 1;
                differing.push((line.path.clone(), line.number));
            }
        }
    }
    Ok((shown, differing))
}

/// Whether `shown` is the file's line `held`, as is or without its control characters other than
/// tab.
fn is_same(shown: &str, held: &str) -> bool {
    shown == held
        || shown
            == held
                .chars()
                .filter(|character| *character == '\t' || !character.is_control())
                .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transcript line holding `text` as Claude Code writes a hook's added context: inside an
    /// attachment's rendered content.
    fn attachment(text: &str) -> String {
        serde_json::json!({
            "type": "attachment",
            "rendered": [{"content": format!(
                "<system-reminder>\nUserPromptSubmit hook additional context: {text}\n</system-reminder>"
            )}],
        })
        .to_string()
    }

    /// A work order as the renderer writes one: two items, then the code of the first, under its
    /// class's first line with a gap mark, its last line cut; then the code of a second file.
    const ORDER: &str = "[cortex] Work order · from the prompt\n\
Start here:\n  1. src/a.py:10-14  run  (named in the prompt)\n  2. src/b.py:1-2  helper\n\
Current code:\n  src/a.py:10-14\n   3 | class A:\n     | ...\n  10 |     def run(self):\n  11 |         \
return 1\n  ... lines 12-14 not shown\n  src/b.py:1-2\n  1 | def helper():\n  2 | \n";

    /// The work order's code lines are read in path and line order, counted once though the
    /// transcript holds the work order twice; the listing, the gap mark and the cut mark are not
    /// code lines; a line with no text keeps an empty text.
    #[test]
    fn a_transcript_s_work_orders_give_their_code_lines() {
        let transcript = format!(
            "{}\nnot json\n{}\n{}",
            attachment(ORDER),
            serde_json::json!({"type": "user", "message": "no work order here"}),
            attachment(ORDER)
        );
        let lines = shown_lines(&transcript);
        let read: Vec<(&str, u32, &str)> = lines
            .iter()
            .map(|line| (line.path.as_str(), line.number, line.text.as_str()))
            .collect();
        assert_eq!(
            read,
            [
                ("src/a.py", 3, "class A:"),
                ("src/a.py", 10, "    def run(self):"),
                ("src/a.py", 11, "        return 1"),
                ("src/b.py", 1, "def helper():"),
                ("src/b.py", 2, ""),
            ]
        );
    }

    /// Checked against files held by hand: `src/a.py` holds its lines 3, 10 and 11 as shown, line
    /// 10 ending in a carriage return and line 11 holding an escape character, both of which a work
    /// order leaves out; `src/b.py` changed its first line and ends before line 2; a file that is
    /// gone differs for every line. Six lines, three differ, each file read once.
    #[test]
    fn each_line_is_checked_against_its_file() -> anyhow::Result<()> {
        let mut lines = shown_lines(&attachment(ORDER));
        lines.push(ShownLine {
            path: "src/gone.py".to_owned(),
            number: 1,
            text: "x = 1".to_owned(),
        });
        let a = "\n\nclass A:\n\n\n\n\n\n\n    def run(self):\r\n        return\u{1b} 1\n";
        let mut reads = Vec::new();
        let (shown, differing) = check(&lines, |path| {
            reads.push(path.to_owned());
            Ok(match path {
                "src/a.py" => Some(a.to_owned()),
                "src/b.py" => Some("def helper(x):\n".to_owned()),
                _ => None,
            })
        })?;
        assert_eq!(
            shown,
            Shown {
                lines: 6,
                differ: 3,
                unread_sessions: 0
            }
        );
        assert_eq!(
            differing,
            [
                ("src/b.py".to_owned(), 1),
                ("src/b.py".to_owned(), 2),
                ("src/gone.py".to_owned(), 1)
            ]
        );
        assert_eq!(reads, ["src/a.py", "src/b.py", "src/gone.py"]);
        Ok(())
    }

    /// A transcript with no work order shows no code.
    #[test]
    fn no_work_order_shows_nothing() {
        let transcript = attachment("[cortex] Project map · tiny mode · about 3 source files");
        assert!(shown_lines(&transcript).is_empty());
    }
}
