//! Credentials in a URL: its user information (`https://user:pass@host` keeps the scheme and the
//! host, `https://<redacted>@host`), and the values of its `name=value` pairs whose names are a
//! credential's, wherever they stand in it: in its query (`?access_token=abc&page=2`), its
//! fragment (`#access_token=abc`) or after an `&` or `+` with no `?` before it. Each value is
//! bounded by the `&` or `#` after it. A URL runs from its scheme to the next space or the end of
//! the word.
//!
//! A URL's scheme has two characters or more and starts with a letter. A single letter before
//! `://` is a drive (`C://Users/dev`, which Windows reads as `C:\Users\dev`), never a URL, so
//! that it is read as the path it is.
//!
//! An `@` after a `/` of the path (`https://user:pa/ss@host`, a password a client would refuse
//! but a person might type) leaves no way to tell user information from path, so it is found
//! unbounded.
//!
//! Other readers of a word pass over its URLs' text, which is this detector's, and read what
//! follows them ([`spans`]). For them a URL ends sooner: at the first byte a URL does not hold or
//! that more often ends one in a command (a quote, a comma, a brace), so that
//! `{"url":"https://x","api_key":"abc"}` and `BASE=https://h,API_TOKEN=abc` leave their keys to
//! be read. The two ends point the same way: this detector reading too far only redacts more, and
//! the others passing over too little only find more.

use std::ops::Range;

use super::detectors::{Finding, run_end};
use super::keys::is_name_byte;
use super::names;

/// What separates a URL's scheme from what follows it.
const URL_MARK: &str = "://";

/// The fewest characters a URL's scheme has: one alone is a drive.
const MIN_SCHEME_CHARS: usize = 2;

/// The punctuation a URL is read over by [`spans`]: RFC 3986's, but for the `,`, `;` and `'` that
/// in a command more often end a URL than stand in one.
const URL_PUNCTUATION: &[u8] = b"-._~:/?#[]@!$&()*+=%";

/// The bytes of `word` each of its URLs takes after its scheme, from its `://` to the first byte
/// that is not a letter, a digit or [`URL_PUNCTUATION`].
pub(super) fn spans(word: &str) -> Vec<Range<usize>> {
    let bytes = word.as_bytes();
    url_marks(word)
        .map(|at| at..run_end(bytes, at + URL_MARK.len(), is_url_byte))
        .collect()
}

/// Where the `://` of each URL of `word` is: each one after a scheme of [`MIN_SCHEME_CHARS`] or
/// more.
fn url_marks(word: &str) -> impl Iterator<Item = usize> + '_ {
    word.match_indices(URL_MARK)
        .map(|(at, _)| at)
        .filter(|&at| scheme_chars(word.as_bytes(), at) >= MIN_SCHEME_CHARS)
}

/// How many characters the scheme before the `://` at `at` of `bytes` has: the run of scheme
/// bytes right before it, from the first letter in that run.
fn scheme_chars(bytes: &[u8], at: usize) -> usize {
    let head = bytes.get(..at).unwrap_or_default();
    let start = head
        .iter()
        .rposition(|&byte| !is_scheme_byte(byte))
        .map_or(0, |before| before + 1);
    let run = head.get(start..).unwrap_or_default();
    run.iter()
        .position(u8::is_ascii_alphabetic)
        .map_or(0, |letter| run.len() - letter)
}

/// A byte of a URL's scheme: a letter, digit, `+`, `-` or `.`.
fn is_scheme_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

/// A byte [`spans`] reads a URL over.
fn is_url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || URL_PUNCTUATION.contains(&byte)
}

/// The credentials in the URLs of `word`.
pub(super) fn find(word: &str) -> Vec<Finding> {
    url_marks(word)
        .flat_map(|at| in_url(word, at + URL_MARK.len()))
        .collect()
}

/// The credentials of the URL whose authority starts at `authority` in `word`.
fn in_url(word: &str, authority: usize) -> Vec<Finding> {
    let bytes = word.as_bytes();
    let url_end = run_end(bytes, authority, |byte| !byte.is_ascii_whitespace());
    let before_query = run_end(bytes, authority, |byte| {
        !byte.is_ascii_whitespace() && !matches!(byte, b'?' | b'#')
    });
    let mut found = Vec::new();
    if let Some(user) = user_information(bytes, authority, before_query) {
        found.push(user);
    }
    found.extend(pair_values(word, authority, url_end));
    found
}

/// The user information of the URL whose authority starts at `authority`, its path ending at
/// `before_query`: what comes before the last `@` there.
fn user_information(bytes: &[u8], authority: usize, before_query: usize) -> Option<Finding> {
    let head = bytes.get(authority..before_query)?;
    let at_sign = head.iter().rposition(|&byte| byte == b'@')?;
    let user = head.get(..at_sign)?;
    if user.is_empty() {
        return None;
    }
    Some(if user.contains(&b'/') {
        Finding::Unbounded
    } else {
        Finding::Secret(authority..authority + at_sign)
    })
}

/// The values of the credentials' `name=value` pairs in the URL from `from` to `to` in `word`: for
/// each `=` there whose name, the name bytes right before it, is a credential's, the bytes after
/// it to the next `&` or `#` or the URL's end.
fn pair_values(word: &str, from: usize, to: usize) -> Vec<Finding> {
    let bytes = word.as_bytes();
    (from..to)
        .filter(|&at| bytes.get(at) == Some(&b'='))
        .filter(|&at| names::is_credential(name_before(word, from, at)))
        .filter_map(|at| {
            let end = run_end(bytes, at + 1, |byte| !matches!(byte, b'&' | b'#')).min(to);
            (end > at + 1).then(|| Finding::Secret(at + 1..end))
        })
        .collect()
}

/// The name right before the `=` at `at` of `word`, no further back than `from`.
fn name_before(word: &str, from: usize, at: usize) -> &str {
    let start = word.as_bytes().get(from..at).map_or(at, |head| {
        head.iter()
            .rposition(|&byte| !is_name_byte(byte))
            .map_or(from, |before| from + before + 1)
    });
    word.get(start..at).unwrap_or_default()
}
