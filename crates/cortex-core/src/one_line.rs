//! Text a person reads in a log or a report, made safe to print: one line, of bounded length,
//! whatever text reached it.

/// `text` as one line: its first `max_chars` characters, each control character, a line break
/// included, written as a space. A message that quotes a path, a key or an error from elsewhere
/// then cannot split the line it is printed on or steer the terminal that shows it.
#[must_use]
pub fn one_line(text: &str, max_chars: usize) -> String {
    text.chars()
        .take(max_chars)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
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

    /// The cut counts characters, not bytes: 1,500 two-byte letters keep their first 1,000, and
    /// text no longer than the limit is kept whole.
    #[test]
    fn the_cut_counts_characters() {
        assert_eq!(one_line(&"é".repeat(1_500), 1_000), "é".repeat(1_000));
        assert_eq!(one_line("short", 5), "short");
    }
}
