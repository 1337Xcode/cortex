//! Redacting a command's words one by one with the detectors. Every word must be one the shell
//! passes as written and must hold no control, bidirectional or invisible character; the bytes a
//! detector marks become a secret piece, and a word a credential's option or `Bearer` points at
//! becomes a secret whole. Anything a detector cannot bound makes the whole command uncertain.

use std::ops::Range;

use crate::commands::shell::Word;

use super::Unsure;
use super::detectors::{self, Finding};
use super::render::is_unsafe_char;

/// A word kept, in pieces: the text it keeps and the secrets taken out of it.
#[derive(Debug)]
pub(super) struct Kept {
    /// The pieces, in order.
    pub(super) pieces: Vec<Piece>,
}

/// A piece of a kept word.
#[derive(Debug)]
pub(super) enum Piece {
    /// Text kept as the shell passed it.
    Text(String),
    /// A secret, taken out.
    Secret,
}

/// `words`, each redacted.
///
/// # Errors
///
/// [`Unsure`] when a word is not passed as written, holds a character that must not be stored, or
/// holds a credential whose value cannot be bounded, or when a credential's option has no value
/// after it, or one that may be the next option.
pub(super) fn redact(words: &[Word]) -> Result<Vec<Kept>, Unsure> {
    let mut kept = Vec::with_capacity(words.len());
    let mut is_value_next = false;
    for word in words {
        let text = word.literal().ok_or(Unsure)?;
        if text.chars().any(is_unsafe_char) {
            return Err(Unsure);
        }
        if is_value_next {
            // A value starting with `-` may be the next option, the credential's a switch.
            if text.starts_with('-') {
                return Err(Unsure);
            }
            kept.push(Kept {
                pieces: vec![Piece::Secret],
            });
            is_value_next = false;
            continue;
        }
        let findings = detectors::find_all(text);
        if findings.contains(&Finding::Unbounded) {
            return Err(Unsure);
        }
        is_value_next = findings.contains(&Finding::ValueNext);
        kept.push(split(text, findings)?);
    }
    if is_value_next { Err(Unsure) } else { Ok(kept) }
}

/// `text` in pieces, the secrets of `findings` taken out.
fn split(text: &str, findings: Vec<Finding>) -> Result<Kept, Unsure> {
    let spans = findings
        .into_iter()
        .filter_map(|finding| match finding {
            Finding::Secret(span) => Some(span),
            Finding::ValueNext | Finding::Unbounded => None,
        })
        .collect();
    let mut pieces = Vec::new();
    let mut at = 0;
    for span in merged(spans) {
        // `get` refuses a span that does not fall on the text's characters.
        let before = text.get(at..span.start).ok_or(Unsure)?;
        if !before.is_empty() {
            pieces.push(Piece::Text(before.to_owned()));
        }
        pieces.push(Piece::Secret);
        at = span.end;
    }
    let after = text.get(at..).ok_or(Unsure)?;
    if !after.is_empty() {
        pieces.push(Piece::Text(after.to_owned()));
    }
    Ok(Kept { pieces })
}

/// `spans` in order, those that overlap or touch made one.
fn merged(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}
