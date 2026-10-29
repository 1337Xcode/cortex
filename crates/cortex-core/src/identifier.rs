//! Names the index may store.
//!
//! Besides paths, the only text the index keeps is names: symbol names, qualified names,
//! configuration keys and document headings. An [`Identifier`] holds one, and its rules are what
//! keep code out of storage: code spans lines and runs long, a name does neither.

use thiserror::Error;

/// Most bytes a stored name may hold. Qualified names of nested items and long headings stay well
/// below this; a longer text is not a name.
pub const MAX_IDENTIFIER_BYTES: usize = 256;

/// Why a text cannot be stored as a name.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentifierError {
    /// The text is empty.
    #[error("a name is empty")]
    Empty,
    /// The text is longer than [`MAX_IDENTIFIER_BYTES`].
    #[error("a name of {len_bytes} bytes is longer than {MAX_IDENTIFIER_BYTES}")]
    TooLong {
        /// The text's length in bytes.
        len_bytes: usize,
    },
    /// The text holds a line break, tab or other control character.
    #[error("a name holds a control character")]
    ControlCharacter,
}

/// One line of text, at most [`MAX_IDENTIFIER_BYTES`] bytes, with no control characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// Checks `text` and keeps it as a name.
    ///
    /// # Errors
    ///
    /// Refuses empty text, text over [`MAX_IDENTIFIER_BYTES`] bytes, and text with a line break or
    /// any other control character.
    pub fn new(text: &str) -> Result<Self, IdentifierError> {
        if text.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if text.len() > MAX_IDENTIFIER_BYTES {
            return Err(IdentifierError::TooLong {
                len_bytes: text.len(),
            });
        }
        if text.chars().any(char::is_control) {
            return Err(IdentifierError::ControlCharacter);
        }
        Ok(Self(text.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names of every kind the index keeps are accepted as written.
    #[test]
    fn names_are_accepted() {
        for name in [
            "add",
            "Calculator.add",
            "retrieval.max_tokens",
            "Getting started",
            "naïve_größe",
            "operator<=",
        ] {
            assert_eq!(
                Identifier::new(name).map(|id| id.as_str().to_owned()),
                Ok(name.to_owned())
            );
        }
        let longest = "a".repeat(MAX_IDENTIFIER_BYTES);
        assert!(Identifier::new(&longest).is_ok());
    }

    /// Code-shaped and empty text is refused with the reason.
    #[test]
    fn code_shaped_text_is_refused() {
        assert_eq!(Identifier::new(""), Err(IdentifierError::Empty));
        assert_eq!(
            Identifier::new("fn add(a: u8) {\n    a\n}"),
            Err(IdentifierError::ControlCharacter)
        );
        assert_eq!(
            Identifier::new("name\twith tab"),
            Err(IdentifierError::ControlCharacter)
        );
        let long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        assert_eq!(
            Identifier::new(&long),
            Err(IdentifierError::TooLong { len_bytes: 257 })
        );
    }
}
