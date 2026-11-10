//! The kept command as text: its words joined by single spaces, each written as the shell passed
//! it, quoted when it holds anything a shell would read as more than text, and the whole cut at a
//! word to [`MAX_KEPT_CHARS`]. Also what characters are never kept: control characters, which
//! could rewrite what a terminal shows, and the bidirectional and invisible ones, which could make
//! the text read otherwise than it is.

use super::redact::{Kept, Piece};
use super::{REDACTED, Unsure};

/// The most characters a kept command may have: enough for a runner, its options and a test id
/// or two, and short enough that a handoff stays small.
pub(super) const MAX_KEPT_CHARS: usize = 200;

/// What ends a command cut short, after a space.
const CUT_MARK: &str = " ...";

/// The punctuation a word may hold and still be written without quotes.
const PLAIN_PUNCTUATION: &str = "_-./:=@%+,";

/// The command of `words`, cut at a word to [`MAX_KEPT_CHARS`] when it is longer.
///
/// # Errors
///
/// [`Unsure`] when not even the program fits.
pub(super) fn command(words: &[Kept]) -> Result<String, Unsure> {
    let shown: Vec<String> = words.iter().map(written).collect();
    let whole = shown.join(" ");
    if whole.chars().count() <= MAX_KEPT_CHARS {
        return Ok(whole);
    }
    let mut cut = String::new();
    let mut cut_chars = 0;
    for word in &shown {
        let added = word.chars().count() + usize::from(cut_chars > 0);
        if cut_chars + added + CUT_MARK.len() > MAX_KEPT_CHARS {
            break;
        }
        if cut_chars > 0 {
            cut.push(' ');
        }
        cut.push_str(word);
        cut_chars += added;
    }
    if cut.is_empty() {
        return Err(Unsure);
    }
    cut.push_str(CUT_MARK);
    Ok(cut)
}

/// `word` as written: its text, each secret as [`REDACTED`], in single quotes when its text holds
/// anything but letters, digits and [`PLAIN_PUNCTUATION`] or it is empty.
fn written(word: &Kept) -> String {
    let is_quoted = word.pieces.is_empty()
        || word.pieces.iter().any(|piece| match piece {
            Piece::Text(text) => !text.chars().all(is_plain),
            Piece::Secret => false,
        });
    let mut text_written = String::new();
    if is_quoted {
        text_written.push('\'');
    }
    for piece in &word.pieces {
        match piece {
            // In single quotes, a quote is written as one closing them, an escaped quote, and one
            // opening them again.
            Piece::Text(text) if is_quoted => text_written.push_str(&text.replace('\'', r"'\''")),
            Piece::Text(text) => text_written.push_str(text),
            Piece::Secret => text_written.push_str(REDACTED),
        }
    }
    if is_quoted {
        text_written.push('\'');
    }
    text_written
}

/// Whether `c` can stand in a word written without quotes.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || PLAIN_PUNCTUATION.contains(c)
}

/// Whether `c` must never be kept: a control character, or a bidirectional or invisible one.
pub(super) fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}
