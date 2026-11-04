//! How many model tokens a text Cortex injects is estimated to take: its UTF-8 bytes over four,
//! rounded up. No model's tokenizer runs here, and none could for every agent; the estimate only
//! sizes text against caps, and every cap is also held in characters, so a wrong estimate can
//! never push a text past an agent's own limit. One factor serves every model until measured
//! usage gives a reason for another.

/// How many UTF-8 bytes one token is estimated to take.
const BYTES_PER_TOKEN: usize = 4;

/// The estimated number of tokens `text` takes: its UTF-8 bytes over four, rounded up, so that a
/// text of one byte is one token and the empty text none.
///
/// ```
/// use cortex_core::estimate_tokens;
/// assert_eq!(estimate_tokens(""), 0);
/// assert_eq!(estimate_tokens("abcde"), 2);
/// ```
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(BYTES_PER_TOKEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes are counted, not characters: `é` is two bytes and `日` three, each one token, and
    /// four of `é` eight bytes, two tokens. Four bytes are one token, five two, and 8,000 bytes of
    /// ASCII 2,000.
    #[test]
    fn tokens_are_bytes_over_four_rounded_up() {
        let cases = [
            ("", 0),
            ("a", 1),
            ("abcd", 1),
            ("abcde", 2),
            ("\u{e9}", 1),
            ("\u{65e5}", 1),
            ("\u{e9}\u{e9}\u{e9}\u{e9}", 2),
        ];
        for (text, expected) in cases {
            assert_eq!(estimate_tokens(text), expected, "{text:?}");
        }
        assert_eq!(estimate_tokens(&"x".repeat(8_000)), 2_000);
        assert_eq!(estimate_tokens(&"x".repeat(8_001)), 2_001);
    }
}
