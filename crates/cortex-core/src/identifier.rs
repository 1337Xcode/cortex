//! Names the index may store.
//!
//! Besides paths, the only text the index keeps is names: symbol names, qualified names,
//! configuration keys and document headings. An [`Identifier`] holds one, and its rules are what
//! keep code out of storage: code spans lines and runs long, a name does neither.

use thiserror::Error;

use crate::one_line::is_hidden;

/// Most bytes a stored name may hold. Qualified names of nested items and long headings stay well
/// below this; a longer text is not a name.
pub const MAX_IDENTIFIER_BYTES: usize = 256;

/// Whether `character` may not be in a name: one of those [`is_hidden`] lists, which change how
/// text displays or show nothing, so that a name holding one can display as other text than it is,
/// and a name reaches agents and people. The zero width joiner and non-joiner are the exception:
/// some scripts and emoji need them.
fn is_hiding(character: char) -> bool {
    is_hidden(character) && !matches!(character, '\u{200C}' | '\u{200D}')
}

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
    /// The text holds a character that changes the direction text displays in, or shows nothing.
    #[error("a name holds a hidden or direction-changing character")]
    HiddenCharacter,
}

/// One line of text, at most [`MAX_IDENTIFIER_BYTES`] bytes, with no control characters and none
/// that hides text or changes its direction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

/// How much of a session id a person is shown: enough to tell the sessions of one repository
/// apart at a glance, and short enough to sit inside a line of prose.
pub const SHOWN_SESSION_CHARS: usize = 8;

/// The start of session id `session`, as much of it as a person is shown.
///
/// A session id holds no control or hidden character, and this many characters are too few to
/// hold an agent's reminder tag, so what this returns is plain text wherever it is put.
#[must_use]
pub fn shown_session(session: &str) -> String {
    session.chars().take(SHOWN_SESSION_CHARS).collect()
}

impl Identifier {
    /// Checks `text` and keeps it as a name.
    ///
    /// # Errors
    ///
    /// Refuses empty text, text over [`MAX_IDENTIFIER_BYTES`] bytes, text with a line break or any
    /// other control character, and text with a character that hides text or changes its
    /// direction.
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
        if text.chars().any(is_hiding) {
            return Err(IdentifierError::HiddenCharacter);
        }
        Ok(Self(text.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for Identifier {
    /// The name, as a string.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Identifier {
    /// A name from a string, checked as [`Identifier::new`] checks it.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::new(&text).map_err(serde::de::Error::custom)
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
            "family \u{1F468}\u{200D}\u{1F467}",
            "mi\u{200C}khaham",
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

    /// Code-shaped and empty text is refused with the reason, and so is text holding a character
    /// that hides text or changes its direction, each of them.
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
        // The bidirectional controls Unicode defines (the arabic letter mark, the left-to-right
        // and right-to-left marks, the embeddings and overrides with their pop, the isolates
        // with theirs), the three zero width characters that join nothing, the soft hyphen, the
        // line and paragraph separators, the invisible operators, and the tag characters, which
        // can spell text no one sees (here the first, U+E0001, the "A", and the last).
        let hidden = [
            0x00AD, 0x061C, 0x200B, 0x200E, 0x200F, 0x2028, 0x2029, 0x202A, 0x202B, 0x202C, 0x202D,
            0x202E, 0x2060, 0x2061, 0x2062, 0x2063, 0x2064, 0x2066, 0x2067, 0x2068, 0x2069, 0xFEFF,
            0xE0001, 0xE0041, 0xE007F,
        ];
        for code in hidden {
            let character = char::from_u32(code).unwrap_or_default();
            assert_eq!(
                Identifier::new(&format!("admin{character}user")),
                Err(IdentifierError::HiddenCharacter),
                "{code:04X}"
            );
        }
        let long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        assert_eq!(
            Identifier::new(&long),
            Err(IdentifierError::TooLong { len_bytes: 257 })
        );
    }
}
