//! What an object id looks like, for the modules that read ids out of git's text or hand one to
//! git as an argument.

/// Whether `text` is a hexadecimal object id: 40 digits for SHA-1, 64 for SHA-256. Hexadecimal
/// text never reads as an option, so an id that passes may be given to git as an argument.
pub(crate) fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}
