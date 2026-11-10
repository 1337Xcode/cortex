//! The text a parser reads: the tail of the output, cut at a whole line, with terminal escape
//! sequences removed, so colour codes never sit inside a path or a number.

use super::MAX_OUTPUT_BYTES;

/// The escape character that opens every sequence this module removes.
const ESCAPE: char = '\u{1b}';

/// The bell character, which ends an operating system command.
const BELL: char = '\u{7}';

/// The last [`MAX_OUTPUT_BYTES`] of `output`, starting at the first whole line when the output was
/// cut, with CSI sequences (`ESC [ ... final byte`, the colours) and OSC sequences (`ESC ] ...`
/// ended by BEL or `ESC \`, the window titles and hyperlinks) removed. The result is never longer
/// than the cut, so the bound holds for output of any size.
pub(super) fn clean_tail(output: &str) -> String {
    let mut text = output;
    if output.len() > MAX_OUTPUT_BYTES {
        let mut start = output.len() - MAX_OUTPUT_BYTES;
        // A cut inside a multi-byte character would not be a string slice.
        while !output.is_char_boundary(start) {
            start += 1;
        }
        let tail = output.get(start..).unwrap_or_default();
        text = tail.split_once('\n').map_or(tail, |(_, rest)| rest);
    }
    strip_escapes(text)
}

/// `text` without its CSI and OSC sequences. An escape that is none of those is dropped alone.
fn strip_escapes(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ESCAPE {
            clean.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                skip_csi(&mut chars);
            }
            Some(']') => {
                chars.next();
                skip_osc(&mut chars);
            }
            _ => {}
        }
    }
    clean
}

/// Skips the rest of a CSI sequence: parameter and intermediate bytes (`0x20..=0x3F`), then one
/// final byte (`0x40..=0x7E`). Any other character means the sequence was cut off; it is left in
/// the text so that a stray escape never swallows a line.
fn skip_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(&next) = chars.peek() {
        match next {
            '\u{20}'..='\u{3f}' => {
                chars.next();
            }
            '\u{40}'..='\u{7e}' => {
                chars.next();
                return;
            }
            _ => return,
        }
    }
}

/// Skips the rest of an OSC sequence, up to and including its BEL or `ESC \` terminator. A line
/// break means the sequence was cut off; it is left in the text.
fn skip_osc(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(&next) = chars.peek() {
        match next {
            '\n' => return,
            BELL => {
                chars.next();
                return;
            }
            ESCAPE => {
                chars.next();
                if chars.peek() == Some(&'\\') {
                    chars.next();
                }
                return;
            }
            _ => {
                chars.next();
            }
        }
    }
}
