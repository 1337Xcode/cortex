//! The credential detectors, one row each in [`DETECTORS`], and what they find. Each reads one
//! word as the shell passes it and says which of its bytes are a secret, that the next word is
//! one, or that it holds one whose extent it cannot tell. Adding a detector is a module of its own
//! and a row here; removing one deletes both.

use std::ops::Range;

use super::{bearer, entropy, flags, jwt, keys, prefixes, urls};

/// What a detector found in a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Finding {
    /// These bytes of the word are a secret.
    Secret(Range<usize>),
    /// The word names a credential whose value is the next word.
    ValueNext,
    /// The word holds a credential, but where its value ends is not plain to see.
    Unbounded,
}

/// A detector: what it finds in one word.
pub(super) type Detector = fn(&str) -> Vec<Finding>;

/// The detectors.
pub(super) const DETECTORS: &[Detector] = &[
    // Tokens of a service's known shape: `ghp_`, `sk-`, `xoxb-`, `AKIA` and the like.
    prefixes::find,
    // JSON Web Tokens.
    jwt::find,
    // `Bearer` and the value after it.
    bearer::find,
    // The value after a credential's option, `--token` or `--password`.
    flags::find,
    // The value after a credential's name and `=` or `:`, `API_KEY=...` or `--token=...`.
    keys::find,
    // The user information of a URL, and its credential query values.
    urls::find,
    // Strings of 20 characters or more that look random.
    entropy::find,
];

/// What every detector finds in `word`.
pub(super) fn find_all(word: &str) -> Vec<Finding> {
    DETECTORS.iter().flat_map(|find| find(word)).collect()
}

/// Whether a token may start at byte `at` of `bytes`: at their start, or after a byte that is not
/// a letter or digit.
pub(super) fn starts_token(bytes: &[u8], at: usize) -> bool {
    at.checked_sub(1)
        .and_then(|before| bytes.get(before))
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
}

/// Where the run of bytes from `from` on that `is_part` takes ends: the first byte it does not
/// take, or the end of `bytes`.
pub(super) fn run_end(bytes: &[u8], from: usize, is_part: impl Fn(u8) -> bool) -> usize {
    bytes.get(from..).map_or(from, |rest| {
        from + rest
            .iter()
            .position(|&byte| !is_part(byte))
            .unwrap_or(rest.len())
    })
}
