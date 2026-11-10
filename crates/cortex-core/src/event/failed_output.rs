//! What a failed shell command printed, as an event carries it to the daemon: the end of the
//! output only, from the start of a line, held in memory and never stored. Runners print their
//! summaries, failure lists and last traceback at the end of a run, so the end is the part that
//! says what failed.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Most bytes of a failed command's output an event carries. Claude Code gives a command's output
/// to its hooks cut to about 30,000 characters; 64 KiB holds that and, written as JSON at up to six
/// bytes a byte, keeps an event well under a frame's mebibyte.
pub const MAX_FAILED_OUTPUT_BYTES: usize = 64 * 1024;

/// The end of a failed command's output: at most [`MAX_FAILED_OUTPUT_BYTES`], starting at a line.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FailedOutput(String);

/// Why a text is not a failed command's output an event may carry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("an output of {len_bytes} bytes is longer than {MAX_FAILED_OUTPUT_BYTES}")]
pub struct FailedOutputError {
    /// The text's length in bytes.
    len_bytes: usize,
}

impl FailedOutput {
    /// The last [`MAX_FAILED_OUTPUT_BYTES`] of `text`. When `text` is longer, the cut moves forward
    /// to the next line's start, so that half a line is never read as a whole one; `None` when
    /// nothing is left, such as for empty output.
    ///
    /// ```
    /// use cortex_core::FailedOutput;
    ///
    /// let output = FailedOutput::tail_of("FAILED tests/test_ops.py::test_divide\n");
    /// assert_eq!(
    ///     output.as_ref().map(FailedOutput::as_str),
    ///     Some("FAILED tests/test_ops.py::test_divide\n"),
    /// );
    /// assert_eq!(FailedOutput::tail_of(""), None);
    /// ```
    #[must_use]
    pub fn tail_of(text: &str) -> Option<Self> {
        let tail = if text.len() <= MAX_FAILED_OUTPUT_BYTES {
            text
        } else {
            let mut start = text.len() - MAX_FAILED_OUTPUT_BYTES;
            while !text.is_char_boundary(start) {
                start += 1;
            }
            let cut = text.get(start..).unwrap_or_default();
            cut.split_once('\n').map_or("", |(_, rest)| rest)
        };
        (!tail.is_empty()).then(|| Self(tail.to_owned()))
    }

    /// The output's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FailedOutput {
    type Error = FailedOutputError;

    /// `text` as it is, refused when it is longer than [`MAX_FAILED_OUTPUT_BYTES`]: whoever made
    /// the event cuts it.
    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.len() > MAX_FAILED_OUTPUT_BYTES {
            return Err(FailedOutputError {
                len_bytes: text.len(),
            });
        }
        Ok(Self(text))
    }
}

impl From<FailedOutput> for String {
    /// The output's text.
    fn from(output: FailedOutput) -> Self {
        output.0
    }
}

impl fmt::Debug for FailedOutput {
    /// The output's length, its text withheld: debug output may end up in a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "FailedOutput(<{} bytes withheld>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A long output keeps its last lines only, from a line's start. With 10,000 lines of 14 bytes
    /// ("line 00000 ..\n"), 140,000 bytes, the cut at 140,000 - 65,536 = 74,464 = 14 * 5,318 + 12
    /// falls 12 bytes into line 5,318, so the first line kept is 5,319, and the text kept is
    /// 140,000 - 14 * 5,319 = 65,534 bytes, ending with the output's last line.
    #[test]
    fn a_long_output_keeps_its_end_from_a_line_start() -> TestResult {
        let text = (0..10_000).fold(String::new(), |mut text, at| {
            let _ = writeln!(text, "line {at:05} ..");
            text
        });
        assert_eq!(text.len(), 140_000);
        let output = FailedOutput::tail_of(&text).ok_or("a long output kept nothing")?;
        assert_eq!(output.as_str().len(), 65_534);
        assert!(output.as_str().starts_with("line 05319 ..\n"));
        assert!(output.as_str().ends_with("line 09999 ..\n"));
        Ok(())
    }

    /// A cut that falls inside a character moves forward to the character's end, then to the next
    /// line: `é` is 2 bytes, so with lines of 15 two-byte characters and a newline (31 bytes) a cut
    /// at an odd offset into a line lands inside a character.
    #[test]
    fn a_cut_inside_a_character_does_not_split_it() -> TestResult {
        let line = format!("{}\n", "é".repeat(15));
        let text = line.repeat(MAX_FAILED_OUTPUT_BYTES / line.len() + 3);
        let start = text.len() - MAX_FAILED_OUTPUT_BYTES;
        assert!(
            !text.is_char_boundary(start),
            "the premise: the cut is inside a character"
        );
        let output = FailedOutput::tail_of(&text).ok_or("the output kept nothing")?;
        assert!(output.as_str().starts_with('é'));
        assert_eq!(output.as_str().len() % line.len(), 0);
        Ok(())
    }

    /// The text is refused past the cap when read from a frame, and never shown in debug output.
    #[test]
    fn a_frame_s_output_is_checked_and_withheld_from_debug() -> TestResult {
        let long = "x".repeat(MAX_FAILED_OUTPUT_BYTES + 1);
        assert!(FailedOutput::try_from(long).is_err());
        let output =
            FailedOutput::tail_of("AssertionError: secret value\n").ok_or("kept nothing")?;
        assert_eq!(format!("{output:?}"), "FailedOutput(<29 bytes withheld>)");
        Ok(())
    }
}
