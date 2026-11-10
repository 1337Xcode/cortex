//! Bearer credentials. The word `Bearer` alone, as the shell splits `--header Bearer abc`, says
//! the next word is the credential. `Bearer ` inside a word, as in `"Authorization: Bearer abc"`,
//! is followed by the credential, which runs to the next space or the end of the word; with
//! nothing after it, the credential may be the next word or nowhere, which cannot be told. Case
//! does not matter, as it does not to the servers that read the scheme.

use super::detectors::{Finding, run_end, starts_token};

/// The scheme's name, in lower case.
const SCHEME: &str = "bearer";

/// The scheme's name and the space after it inside a word, in lower case.
const SCHEME_AND_SPACE: &str = "bearer ";

/// The bearer credentials in `word`.
pub(super) fn find(word: &str) -> Vec<Finding> {
    if word.eq_ignore_ascii_case(SCHEME) {
        return vec![Finding::ValueNext];
    }
    let bytes = word.as_bytes();
    // Lower-casing ASCII moves no byte, so positions in `lower` are positions in `word`.
    let lower = word.to_ascii_lowercase();
    lower
        .match_indices(SCHEME_AND_SPACE)
        .filter(|&(at, _)| starts_token(bytes, at))
        .map(|(at, scheme)| {
            let start = run_end(bytes, at + scheme.len(), |byte| byte == b' ');
            let end = run_end(bytes, start, |byte| !byte.is_ascii_whitespace());
            if end > start {
                Finding::Secret(start..end)
            } else {
                Finding::Unbounded
            }
        })
        .collect()
}
