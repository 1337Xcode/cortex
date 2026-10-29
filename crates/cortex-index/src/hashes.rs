//! The hashes a definition carries, of its header up to where its body starts and of the whole
//! definition. Each run of whitespace counts as one space and whitespace at either end is ignored,
//! so reindenting or rewrapping a definition keeps its hashes.

use cortex_core::ShortHash;

/// The hash of `text` with each run of ASCII whitespace (space, tab, line feed, form feed,
/// carriage return) read as one space, and none at either end.
pub(crate) fn collapsed_hash(text: &[u8]) -> ShortHash {
    ShortHash::of(&collapse_whitespace(text))
}

/// `text` with each run of ASCII whitespace replaced by one space, and none at either end.
fn collapse_whitespace(text: &[u8]) -> Vec<u8> {
    let mut collapsed = Vec::with_capacity(text.len());
    let mut after_space = false;
    for &byte in text {
        if byte.is_ascii_whitespace() {
            after_space = true;
            continue;
        }
        if after_space && !collapsed.is_empty() {
            collapsed.push(b' ');
        }
        after_space = false;
        collapsed.push(byte);
    }
    collapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs of every kind of whitespace become one space; whitespace at either end goes.
    #[test]
    fn whitespace_runs_become_one_space() {
        let cases: [(&[u8], &[u8]); 5] = [
            (b"  def  f(a,\n\tb):  ", b"def f(a, b):"),
            (b"x\r\n\r\ny", b"x y"),
            (b"a\x0cb", b"a b"),
            (b"", b""),
            (b" \n\t ", b""),
        ];
        for (text, expected) in cases {
            assert_eq!(collapse_whitespace(text), expected, "{text:?}");
        }
    }

    /// A definition rewrapped and reindented hashes as the same text written on one line.
    #[test]
    fn reindenting_keeps_the_hash() {
        let one_line = ShortHash::of(b"def f(a, b): return a");
        assert_eq!(collapsed_hash(b"def f(a,  b):\n    return a\n"), one_line);
        assert_eq!(collapsed_hash(b"def f(a, b):\n\treturn a"), one_line);
        assert_ne!(collapsed_hash(b"def f(a, b): return b"), one_line);
    }
}
