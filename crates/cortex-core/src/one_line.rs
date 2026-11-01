//! Text a person reads in a log or a report, made safe to print: one line, of bounded length,
//! whatever text reached it.

/// `text` as one line: its first `max_chars` characters, each control character (a line break
/// included) and each character that reorders or hides text written as a space. A message that
/// quotes a path, a key or an error from elsewhere then cannot split the line it is printed on,
/// steer the terminal that shows it, or read differently from what it holds.
#[must_use]
pub fn one_line(text: &str, max_chars: usize) -> String {
    text.chars()
        .take(max_chars)
        .map(|character| {
            if character.is_control() || is_hidden(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Whether `character` changes how the text around it shows without showing itself: the soft
/// hyphen (U+00AD), the bidirectional marks, embeddings, overrides and isolates (U+061C, U+200E
/// and U+200F, U+202A to U+202E, U+2066 to U+2069), the zero-width space, joiners and word joiner
/// (U+200B to U+200D, U+2060), the invisible operators (U+2061 to U+2064), the line and paragraph
/// separators (U+2028, U+2029), the byte order mark (U+FEFF) and the tag characters (U+E0000 to
/// U+E007F), which can spell text no one sees.
fn is_hidden(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line break, a tab and an escape each become a space; everything else, accents included,
    /// stays as it was.
    #[test]
    fn control_characters_become_spaces() {
        assert_eq!(
            one_line("one\ntwo\tthree\u{1b}[31m café", 100),
            "one two three [31m café"
        );
    }

    /// A right-to-left override, an isolate, a zero-width space, a byte order mark, a soft hyphen,
    /// a line separator and a tag character each become a space, so `ab` with an override before
    /// `cd` prints as `ab cd`, not reversed.
    #[test]
    fn hidden_characters_become_spaces() {
        assert_eq!(
            one_line(
                "ab\u{202E}cd\u{2066}e\u{200B}f\u{FEFF}g\u{00AD}h\u{2028}i\u{E0041}j",
                100
            ),
            "ab cd e f g h i j"
        );
    }

    /// The cut counts characters, not bytes: 1,500 two-byte letters keep their first 1,000, and
    /// text no longer than the limit is kept whole.
    #[test]
    fn the_cut_counts_characters() {
        assert_eq!(one_line(&"é".repeat(1_500), 1_000), "é".repeat(1_000));
        assert_eq!(one_line("short", 5), "short");
    }
}
