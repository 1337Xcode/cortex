//! Writing a word into a command line so that the shell passes it as it is.

/// Whether `byte` may stand in a word unquoted, in a POSIX shell and in PowerShell alike: a
/// letter, digit, `_`, `.`, `/`, `-`, or the `=` and `:` of an option's value (`--tb=short`,
/// `-p no:cacheprovider`). A word is never the first of a command here, so an `=` sets nothing.
fn is_unquoted(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-' | b'=' | b':')
}

/// `text` as one word that a POSIX shell and PowerShell both pass as it is: as it is when it holds
/// only `[A-Za-z0-9_./=:-]` and is not an option holding a `.` (Windows PowerShell splits
/// `-coverpkg=./...` at its first `.`), otherwise in single quotes. `None` for a text holding a
/// single quote, which the two shells escape in ways each reads wrongly, or a control character.
/// The empty text is `''`.
pub(super) fn word(text: &str) -> Option<String> {
    if text.chars().any(|c| c == '\'' || c.is_control()) {
        return None;
    }
    let is_split_by_powershell = text.starts_with('-') && text.contains('.');
    if !text.is_empty() && text.bytes().all(is_unquoted) && !is_split_by_powershell {
        return Some(text.to_owned());
    }
    Some(format!("'{text}'"))
}
