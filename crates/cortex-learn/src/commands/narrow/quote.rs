//! Writing a word into a command line so that the shell passes it as it is.

/// Whether `byte` may stand in a word unquoted: a letter, digit, `_`, `.`, `/` or `-`.
fn is_unquoted(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-')
}

/// `text` as one shell word: as it is when it holds only `[A-Za-z0-9_./-]`, otherwise in single
/// quotes, a quote inside it written `'\''`. The empty text is `''`.
pub(super) fn word(text: &str) -> String {
    if !text.is_empty() && text.bytes().all(is_unquoted) {
        return text.to_owned();
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        if c == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    quoted
}
