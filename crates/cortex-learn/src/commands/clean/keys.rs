//! A credential's name with its value in the same word, after `=` or a single `:`:
//! `--token=abc`, `API_KEY=abc` given as an argument (`make test API_KEY=abc`),
//! `-Ddb.password=abc`, `-p:ApiToken=abc`, `"X-Api-Key: abc"`, `{"api_key":"abc"}`. The name is
//! the letters, digits, `_`, `-` and `.` right before the separator (a closing quote between them
//! passed over), and is a credential's when `names` says so.
//!
//! The value is everything after the separator and any spaces, to the end of the word. That can
//! take in more than the value (the rest of a JSON object), which only redacts more; a value that
//! stopped short would leave part of the secret. With nothing after the separator the value may
//! be the next word or empty, which cannot be told. A `::` is a path's separator
//! (`tests/test_auth.py::test_token`), not a name's. A URL's text is the `urls` detector's, but
//! only its text: a credential named after it in the word (`BASE=https://h,API_TOKEN=abc`) is
//! still this one's.

use super::detectors::{Finding, run_end};
use super::names;
use super::urls;

/// The credential values named in `word`.
pub(super) fn find(word: &str) -> Vec<Finding> {
    let bytes = word.as_bytes();
    let in_urls = urls::spans(word);
    (0..bytes.len())
        .filter(|at| !in_urls.iter().any(|span| span.contains(at)))
        .filter(|&at| is_separator(bytes, at))
        .filter(|&at| names::is_credential(name_before(word, at)))
        .map(|at| value_after(bytes, at + 1))
        .collect()
}

/// Whether the byte at `at` separates a name from its value: `=`, or `:` alone.
fn is_separator(bytes: &[u8], at: usize) -> bool {
    match bytes.get(at) {
        Some(b'=') => true,
        Some(b':') => {
            let before = at.checked_sub(1).and_then(|before| bytes.get(before));
            before != Some(&b':') && bytes.get(at + 1) != Some(&b':')
        }
        _ => false,
    }
}

/// The name right before the separator at `at`, a closing quote before it passed over and the
/// dashes of an option taken off.
fn name_before(word: &str, at: usize) -> &str {
    let bytes = word.as_bytes();
    let end = match at.checked_sub(1).and_then(|before| bytes.get(before)) {
        Some(b'"' | b'\'') => at - 1,
        _ => at,
    };
    let start = bytes.get(..end).map_or(end, |head| {
        head.iter()
            .rposition(|&byte| !is_name_byte(byte))
            .map_or(0, |before| before + 1)
    });
    word.get(start..end)
        .unwrap_or_default()
        .trim_start_matches('-')
}

/// The value from `from` on: past the spaces there, to the end of the word.
fn value_after(bytes: &[u8], from: usize) -> Finding {
    let start = run_end(bytes, from, |byte| byte == b' ');
    if start < bytes.len() {
        Finding::Secret(start..bytes.len())
    } else {
        Finding::Unbounded
    }
}

/// A byte of a name: a letter, digit, `_`, `-` or `.`.
pub(super) fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}
