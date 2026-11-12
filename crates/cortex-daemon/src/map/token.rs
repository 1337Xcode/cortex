//! The map's access token: 32 bytes from the operating system's random number generator, written
//! as 64 lowercase hexadecimal digits, the first segment of every path the map serves. Whoever
//! holds it can read the map, so it never reaches a log, and a candidate is compared with it in
//! time that does not depend on where the two first differ.

use std::fmt::{self, Write as _};

/// How many random bytes a token holds: 256 bits, too many to guess.
const TOKEN_BYTES: usize = 32;

/// The map's access token.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Token(String);

impl Token {
    /// A new token from the operating system's random number generator.
    ///
    /// # Errors
    ///
    /// The operating system gave no random bytes.
    pub(crate) fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(Self::from_bytes(&bytes))
    }

    /// The token written for `bytes`: two lowercase hexadecimal digits a byte.
    fn from_bytes(bytes: &[u8; TOKEN_BYTES]) -> Self {
        let mut text = String::with_capacity(TOKEN_BYTES * 2);
        for byte in bytes {
            // Writing to a string cannot fail.
            let _ = write!(text, "{byte:02x}");
        }
        Self(text)
    }

    /// The token as written in the map's address.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `candidate` is this token. Every byte is compared whatever the earlier ones held,
    /// so the time taken says nothing of how much of a guess was right; only the length, which
    /// is no secret, ends the comparison early.
    pub(crate) fn matches(&self, candidate: &str) -> bool {
        let (ours, theirs) = (self.0.as_bytes(), candidate.as_bytes());
        if ours.len() != theirs.len() {
            return false;
        }
        let difference = ours
            .iter()
            .zip(theirs)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b));
        std::hint::black_box(difference) == 0
    }
}

impl fmt::Debug for Token {
    /// Never the token itself: a debug line could reach a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes 0x00, 0x01, ..., 0x1f are written two digits each, lowercase, worked out by hand:
    /// `00 01 02 ... 0f 10 11 ... 1f`.
    #[test]
    fn a_token_is_its_bytes_in_lowercase_hex() {
        let mut bytes = [0u8; TOKEN_BYTES];
        for (byte, value) in bytes.iter_mut().zip(0u8..) {
            *byte = value;
        }
        let token = Token::from_bytes(&bytes);
        assert_eq!(
            token.as_str(),
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
        assert_eq!(format!("{token:?}"), "Token(..)");
    }

    /// A generated token is 64 lowercase hex digits, and two are not alike.
    #[test]
    fn generated_tokens_are_long_and_differ() -> Result<(), getrandom::Error> {
        let (first, second) = (Token::generate()?, Token::generate()?);
        for token in [&first, &second] {
            assert_eq!(token.as_str().len(), 64);
            assert!(
                token
                    .as_str()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
        }
        assert_ne!(first, second);
        Ok(())
    }

    /// Only the token itself matches: one digit changed at either end, a prefix, a longer text
    /// and the empty text do not.
    #[test]
    fn only_the_token_matches() {
        let token = Token::from_bytes(&[0xab; TOKEN_BYTES]);
        let text = token.as_str().to_owned();
        assert!(token.matches(&text));
        let first_changed = format!("c{}", &text[1..]);
        let last_changed = format!("{}c", &text[..63]);
        for wrong in [
            first_changed.as_str(),
            last_changed.as_str(),
            &text[..63],
            &format!("{text}0"),
            "",
        ] {
            assert!(!token.matches(wrong), "{wrong}");
        }
    }
}
