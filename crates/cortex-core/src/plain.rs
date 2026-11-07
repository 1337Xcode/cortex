//! Text that can be put in front of an agent as it is. Everything Cortex writes for an agent names
//! files and symbols by what the repository holds, and a repository is untrusted: a file name can
//! hold a line break, a control character, a character that reorders or hides text, or one of an
//! agent's reminder tags. What Cortex writes for an agent holds what it names to this one check, so
//! a crafted name cannot add a line, a hidden instruction or a tag to what the agent reads.

use crate::one_line::is_hidden;
use crate::reminder_tag::holds_reminder_tag;

/// Whether `character` is taken out of what Cortex writes for an agent: a control character other
/// than a tab, or one that reorders or hides text ([`is_hidden`]).
#[must_use]
pub fn is_taken_out(character: char) -> bool {
    character != '\t' && (character.is_control() || is_hidden(character))
}

/// Whether `text` can be named to an agent as it is: no character [`is_taken_out`] names and none
/// of an agent's reminder tags.
#[must_use]
pub fn is_plain(text: &str) -> bool {
    text.chars().all(|character| !is_taken_out(character)) && !holds_reminder_tag(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path of letters, slashes, dots, spaces and a tab is plain; a line break, a carriage
    /// return, an escape, a right-to-left override (U+202E), a zero-width space (U+200B) or a
    /// reminder tag in it is not.
    #[test]
    fn only_text_that_shows_as_it_is_is_plain() {
        assert!(is_plain("src/api/order handler.py"));
        assert!(is_plain("a\tb.rs"));
        for crafted in [
            "a\nb.rs",
            "a\rb.rs",
            "a\u{1b}[2Jb.rs",
            "a\u{202e}b.rs",
            "a\u{200b}b.rs",
            "<system-reminder>.rs",
        ] {
            assert!(!is_plain(crafted), "{crafted:?}");
        }
    }
}
