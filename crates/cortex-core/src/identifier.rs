//! Names the index may store.
//!
//! Besides paths, the only text the index keeps is names: symbol names, qualified names,
//! configuration keys and document headings. An [`Identifier`] holds one, and its rules are what
//! keep code out of storage: code spans lines and runs long, a name does neither.

use thiserror::Error;

/// Most bytes a stored name may hold. Qualified names of nested items and long headings stay well
/// below this; a longer text is not a name.
pub const MAX_IDENTIFIER_BYTES: usize = 256;

/// Characters that change the direction text displays in, or show nothing: the bidirectional
/// marks, embeddings, overrides and isolates, the zero width space, the word joiner and the zero
/// width no-break space. A name holding one can display as other text than it is, and a name
/// reaches agents and people. The zero width joiner and non-joiner are allowed: some scripts and
/// emoji need them.
const HIDDEN_CHARACTERS: &[char] = &[
    '\u{061C}', '\u{200B}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
    '\u{202E}', '\u{2060}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{FEFF}',
];

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
        if text
            .chars()
            .any(|character| HIDDEN_CHARACTERS.contains(&character))
        {
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
        // with theirs) and the three zero width characters that join nothing.
        let hidden = [
            0x061C, 0x200B, 0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2060, 0x2066,
            0x2067, 0x2068, 0x2069, 0xFEFF,
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
