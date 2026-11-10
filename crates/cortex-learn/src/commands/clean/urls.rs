//! Credentials in a URL: its user information (`https://user:pass@host` keeps the scheme and the
//! host, `https://<redacted>@host`), and the values of its query parameters whose names are a
//! credential's (`?access_token=abc&page=2`), each bounded by the `&` or `#` after it. A URL
//! runs from its scheme to the next space or the end of the word.
//!
//! An `@` after a `/` of the path (`https://user:pa/ss@host`, a password a client would refuse
//! but a person might type) leaves no way to tell user information from path, so it is found
//! unbounded.

use super::detectors::{Finding, run_end};
use super::names;

/// What separates a URL's scheme from what follows it.
pub(super) const URL_MARK: &str = "://";

/// The credentials in the URLs of `word`.
pub(super) fn find(word: &str) -> Vec<Finding> {
    word.match_indices(URL_MARK)
        .flat_map(|(at, mark)| in_url(word, at + mark.len()))
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
    found.extend(query_values(word, authority, url_end));
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

/// The values of the credential parameters in the query of the URL from `from` to `to` in `word`.
fn query_values(word: &str, from: usize, to: usize) -> Vec<Finding> {
    let Some(url) = word.get(from..to) else {
        return Vec::new();
    };
    let Some(mark) = url.find('?') else {
        return Vec::new();
    };
    let params_at = mark + 1;
    let params_end = url
        .get(params_at..)
        .and_then(|params| params.find('#'))
        .map_or(url.len(), |hash| params_at + hash);
    let mut found = Vec::new();
    let mut cursor_at = from + params_at;
    for param in url
        .get(params_at..params_end)
        .unwrap_or_default()
        .split('&')
    {
        if let Some((name, value)) = param.split_once('=')
            && !value.is_empty()
            && names::is_credential(name)
        {
            let value_at = cursor_at + name.len() + 1;
            found.push(Finding::Secret(value_at..value_at + value.len()));
        }
        cursor_at += param.len() + 1;
    }
    found
}
