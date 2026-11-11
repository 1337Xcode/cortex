//! What a git object id looks like, for the code that reads ids out of git's text, hands one to git
//! as an argument or stores one.

/// Whether `text` is a lower-case hexadecimal object id: 40 digits for SHA-1, 64 for SHA-256, as
/// git prints them. Hexadecimal text never reads as an option, so an id that passes may be given
/// to git as an argument. Only one spelling passes, so that one commit is one stored key.
///
/// ```
/// assert!(cortex_core::is_object_id("0123456789abcdef0123456789abcdef01234567"));
/// assert!(!cortex_core::is_object_id("0123456789ABCDEF0123456789ABCDEF01234567"));
/// ```
#[must_use]
pub fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::is_object_id;

    /// Forty and sixty-four lower-case digits pass; no other length does, and neither does a
    /// character outside 0 to 9 and a to f, an upper-case digit, a space, an option, or nothing.
    #[test]
    fn only_forty_or_sixty_four_lower_case_hexadecimal_digits_pass() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = sha1.repeat(2).chars().take(64).collect::<String>();
        assert!(is_object_id(sha1));
        assert!(is_object_id(&sha256));
        let refused = [
            String::new(),
            sha1[..39].to_owned(),
            format!("{sha1}0"),
            sha256[..63].to_owned(),
            format!("{sha256}0"),
            sha1.to_uppercase(),
            format!("A{}", &sha1[1..]),
            format!("g{}", &sha1[1..]),
            format!("-{}", &sha1[1..]),
            format!(" {}", &sha1[1..]),
            "--upload-pack=x".to_owned(),
        ];
        for text in refused {
            assert!(!is_object_id(&text), "{text:?}");
        }
    }
}
