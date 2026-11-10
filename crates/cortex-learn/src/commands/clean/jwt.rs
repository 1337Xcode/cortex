//! JSON Web Tokens: a header and a payload, each a JSON object in base64url and so starting
//! `eyJ`, then a signature, joined by dots. The whole token is the secret, wherever it stands in
//! the word (`--jwt=...`, `Authorization:...`). The entropy detector alone would miss the payload
//! and signature when they are short.

use super::detectors::{Finding, run_end, starts_token};

/// How a header and a payload start: `{"` in base64url.
const OBJECT_START: &str = "eyJ";

/// The JSON Web Tokens in `word`.
pub(super) fn find(word: &str) -> Vec<Finding> {
    let bytes = word.as_bytes();
    word.match_indices(OBJECT_START)
        .filter(|&(at, _)| starts_token(bytes, at))
        .filter_map(|(at, _)| token_end(bytes, at).map(|end| Finding::Secret(at..end)))
        .collect()
}

/// Where the token whose header starts at `at` ends, when a dot, a payload starting `eyJ`, a dot
/// and a signature follow the header.
fn token_end(bytes: &[u8], at: usize) -> Option<usize> {
    let header_end = run_end(bytes, at, is_base64url);
    let payload_at = after_dot(bytes, header_end)?;
    if !bytes
        .get(payload_at..)?
        .starts_with(OBJECT_START.as_bytes())
    {
        return None;
    }
    let payload_end = run_end(bytes, payload_at, is_base64url);
    let signature_at = after_dot(bytes, payload_end)?;
    Some(run_end(bytes, signature_at, is_base64url))
}

/// The byte after the dot at `at`, when there is a dot there.
fn after_dot(bytes: &[u8], at: usize) -> Option<usize> {
    (bytes.get(at) == Some(&b'.')).then_some(at + 1)
}

/// A character of base64url: a letter, digit, `_` or `-`.
fn is_base64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}
