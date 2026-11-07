//! The cap on what one tool call gives the agent: 1,200 tokens, estimated as Cortex estimates
//! every text it gives (`estimate_tokens`). A longer answer is cut after its last whole line that
//! fits with the note saying so, so no line it shows is partial.

use cortex_core::estimate_tokens;

/// The most tokens one tool call's text may hold, the note of a cut included.
pub(crate) const MAX_OUTPUT_TOKENS: usize = 1_200;

/// `text` within [`MAX_OUTPUT_TOKENS`]: whole when it fits; otherwise its first lines that fit
/// with a note saying how many lines were left out and how to ask for less, or, when not even the
/// first line fits, a note saying so alone.
pub(crate) fn capped(text: &str) -> String {
    if estimate_tokens(text) <= MAX_OUTPUT_TOKENS {
        return text.to_owned();
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = String::new();
    for (shown, line) in lines.iter().enumerate() {
        let candidate = format!("{kept}{line}\n");
        let note = cut_note(lines.len() - shown - 1);
        // INVARIANT: budgeted. Only whole lines that fit with the note are given.
        if estimate_tokens(&format!("{candidate}{note}")) > MAX_OUTPUT_TOKENS {
            if shown == 0 {
                return FIRST_LINE_TOO_LONG.to_owned();
            }
            return format!("{kept}{}", cut_note(lines.len() - shown));
        }
        kept = candidate;
    }
    kept
}

/// What is given when the answer's first line alone is past the cap.
const FIRST_LINE_TOO_LONG: &str =
    "[cortex] the answer's first line alone is past 1200 tokens; nothing is shown\n";

/// The note closing a cut answer, `left_out` lines short.
fn cut_note(left_out: usize) -> String {
    format!(
        "[cortex] cut at {MAX_OUTPUT_TOKENS} tokens: {left_out} more lines; ask with a smaller limit\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text within the cap is given whole; past it, the text is cut after a whole line and
    /// closed with the note, and the whole stays within the cap. Worked by hand: lines of 99
    /// characters and a line break are 100 bytes each; 300 of them are 7,500 tokens. The note,
    /// for any three-digit count, is 70 bytes. `k` lines and the note are `100k + 70` bytes,
    /// `25k + 18` tokens (17.5 rounded up): 47 lines make 1,193, 48 make 1,218, past the cap. So
    /// 47 lines are kept and 253 left out.
    #[test]
    fn a_long_answer_is_cut_after_a_whole_line_and_says_so() {
        assert_eq!(capped("short\nanswer\n"), "short\nanswer\n");
        let line = "x".repeat(99);
        let text: String = (0..300).map(|_| format!("{line}\n")).collect();
        let cut = capped(&text);
        let kept: Vec<&str> = cut.lines().collect();
        assert_eq!(kept.len(), 48);
        assert!(kept.iter().take(47).all(|kept| *kept == line));
        assert_eq!(
            kept.last().copied(),
            Some("[cortex] cut at 1200 tokens: 253 more lines; ask with a smaller limit")
        );
        assert!(estimate_tokens(&cut) <= MAX_OUTPUT_TOKENS);
    }

    /// The edges: a text of exactly 1,200 tokens (4,800 bytes) is given whole; one byte more is
    /// cut; a first line alone past the cap gives the note saying so and no partial line; a text
    /// with Windows line breaks within the cap is given as it is; a line of multibyte characters
    /// is counted by its bytes (`é` is two: 2,400 of them are 4,800 bytes, 1,200 tokens).
    #[test]
    fn the_edges_of_the_cap() {
        let exact = format!("{}\n", "x".repeat(4_799));
        assert_eq!(capped(&exact), exact);
        let over = format!("{}\n", "x".repeat(4_800));
        let first_too_long =
            "[cortex] the answer's first line alone is past 1200 tokens; nothing is shown\n";
        assert_eq!(capped(&over), first_too_long);
        assert_eq!(capped("a\r\nb\r\n"), "a\r\nb\r\n");
        let accented = "é".repeat(2_400);
        assert_eq!(capped(&accented), accented);
        assert_eq!(capped(&format!("{accented}é")), first_too_long);
    }
}
