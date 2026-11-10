//! Tokens of a service's known shape: a fixed prefix, then a body of the characters that
//! service's tokens use. GitHub's tokens start `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` or
//! `github_pat_`; model providers' keys `sk-` and `sk-ant-`; Slack's tokens `xoxb-` and its
//! kin; AWS access key ids `AKIA` or `ASIA`; Google API keys `AIza`; GitLab's tokens `glpat-`;
//! npm's `npm_`.
//!
//! A prefix counts only where a token may start (the start of the word, or after a character that
//! is not a letter or digit), so `risk-free` holds no `sk-` key, and only with at least the
//! shape's least body after it. `sk-` alone also starts hyphenated words (`sk-learn` and what
//! follows it in a long test filter), so its keys must also hold a digit: a random body of 48
//! letters and digits, the shortest those keys have, lacks one about twice in ten thousand
//! ((52/62)^48), and the longer ones less often.

use super::detectors::{Finding, run_end, starts_token};

/// One shape of token.
struct Shape {
    /// The prefix, as the service writes it.
    prefix: &'static str,
    /// Whether a byte belongs to the body.
    is_body: fn(u8) -> bool,
    /// The least characters of body after the prefix: no more than the service's own tokens
    /// have, and fewer, so that a token cut short still counts.
    min_body_chars: usize,
    /// Whether the body must hold a digit, for a prefix that also starts ordinary words.
    needs_digit: bool,
}

/// The shapes. A shape whose prefix starts another's (`sk-` and `sk-ant-`) finds the same token
/// twice, which redacts it once.
const SHAPES: &[Shape] = &[
    shape("ghp_", is_alphanumeric, 20, false),
    shape("gho_", is_alphanumeric, 20, false),
    shape("ghu_", is_alphanumeric, 20, false),
    shape("ghs_", is_alphanumeric, 20, false),
    shape("ghr_", is_alphanumeric, 20, false),
    shape("github_pat_", is_word, 20, false),
    shape("sk-ant-", is_url_safe, 20, true),
    shape("sk-", is_url_safe, 20, true),
    shape("xoxa-", is_slack, 10, false),
    shape("xoxb-", is_slack, 10, false),
    shape("xoxe-", is_slack, 10, false),
    shape("xoxp-", is_slack, 10, false),
    shape("xoxr-", is_slack, 10, false),
    shape("xoxs-", is_slack, 10, false),
    shape("AKIA", is_upper_or_digit, 16, false),
    shape("ASIA", is_upper_or_digit, 16, false),
    shape("AIza", is_url_safe, 30, false),
    shape("glpat-", is_url_safe, 20, false),
    shape("npm_", is_alphanumeric, 30, false),
];

/// A row of [`SHAPES`].
const fn shape(
    prefix: &'static str,
    is_body: fn(u8) -> bool,
    min_body_chars: usize,
    needs_digit: bool,
) -> Shape {
    Shape {
        prefix,
        is_body,
        min_body_chars,
        needs_digit,
    }
}

/// The tokens of a known shape in `word`, each whole.
pub(super) fn find(word: &str) -> Vec<Finding> {
    let bytes = word.as_bytes();
    SHAPES
        .iter()
        .flat_map(|shape| {
            word.match_indices(shape.prefix)
                .filter_map(move |(at, prefix)| token_at(bytes, at, prefix.len(), shape))
        })
        .collect()
}

/// The token of `shape` whose prefix, `prefix_len` bytes long, starts at `at` in `bytes`, if one
/// is there.
fn token_at(bytes: &[u8], at: usize, prefix_len: usize, shape: &Shape) -> Option<Finding> {
    if !starts_token(bytes, at) {
        return None;
    }
    let body_at = at + prefix_len;
    let end = run_end(bytes, body_at, shape.is_body);
    let body = bytes.get(body_at..end)?;
    let has_digit = !shape.needs_digit || body.iter().any(u8::is_ascii_digit);
    (body.len() >= shape.min_body_chars && has_digit).then_some(Finding::Secret(at..end))
}

/// A letter or digit.
fn is_alphanumeric(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// A letter, digit or `_`.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A letter, digit, `_` or `-`: the characters of base64url.
fn is_url_safe(byte: u8) -> bool {
    is_word(byte) || byte == b'-'
}

/// A letter, digit or `-`.
fn is_slack(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// An upper-case letter or a digit.
fn is_upper_or_digit(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit()
}
