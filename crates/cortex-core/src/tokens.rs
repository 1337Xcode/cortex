//! Identifier tokens: names and paths split into the lower-case words a search matches, split the
//! same way for what is indexed and for what is looked up.
//!
//! Words are separated by whitespace, control characters, ASCII punctuation and symbols, and the
//! general and CJK punctuation of Unicode (dashes, curly quotes, ellipses, ideographic commas);
//! every other character belongs to a word, combining marks included, so a decomposed `é` stays
//! with its letter. Inside a word, a capital after a small letter or a digit starts a new token
//! (`parseRequest`, `base64Encode`), and so does the last capital of a run of capitals when a
//! small letter follows (`HTTPRequest` gives `http` and `request`). Digits stay with what comes
//! before them (`sha256`).

use std::ops::RangeInclusive;

/// Unicode blocks of punctuation that separate words: General Punctuation, and CJK Symbols and
/// Punctuation.
const PUNCTUATION_BLOCKS: [RangeInclusive<char>; 2] =
    ['\u{2000}'..='\u{206F}', '\u{3000}'..='\u{303F}'];

/// The tokens of `text`, lower-case, in the order they appear, repeats kept.
///
/// ```
/// use cortex_core::identifier_tokens;
/// assert_eq!(identifier_tokens("parseHTTPRequest"), ["parse", "http", "request"]);
/// assert_eq!(identifier_tokens("src/shop/order_lines.py"), ["src", "shop", "order", "lines", "py"]);
/// ```
#[must_use]
pub fn identifier_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    // The last letter or digit of the current word; marks inside a word do not replace it.
    let mut previous: Option<char> = None;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if is_separator(character) {
            finish(&mut current, &mut tokens);
            previous = None;
            continue;
        }
        if character.is_uppercase()
            && previous.is_some_and(|previous| starts_hump(previous, characters.peek().copied()))
        {
            finish(&mut current, &mut tokens);
        }
        current.push(character);
        if character.is_alphanumeric() {
            previous = Some(character);
        }
    }
    finish(&mut current, &mut tokens);
    tokens
}

/// Whether `character` separates words.
fn is_separator(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || (character.is_ascii() && !character.is_ascii_alphanumeric())
        || PUNCTUATION_BLOCKS
            .iter()
            .any(|block| block.contains(&character))
}

/// Whether a capital after `previous` and before `next` starts a new token: after a small letter
/// or a digit, or as the last capital of a run when a small letter follows.
fn starts_hump(previous: char, next: Option<char>) -> bool {
    previous.is_lowercase()
        || previous.is_numeric()
        || (previous.is_uppercase() && next.is_some_and(char::is_lowercase))
}

/// Moves the token built in `current`, lower-cased, to `tokens`, unless it is empty.
fn finish(current: &mut String, tokens: &mut Vec<String>) {
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
        current.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Separators, humps, runs of capitals and digits split as the module says; text without
    /// case stays whole, and combining marks stay with their letters.
    #[test]
    fn identifiers_split_into_words() {
        let cases: [(&str, &[&str]); 25] = [
            ("parseHTTPRequest", &["parse", "http", "request"]),
            ("XMLHttpRequest", &["xml", "http", "request"]),
            ("IOError", &["io", "error"]),
            ("getX", &["get", "x"]),
            ("base64Encode", &["base64", "encode"]),
            ("sha256", &["sha256"]),
            ("snake_case_name", &["snake", "case", "name"]),
            ("__init__", &["init"]),
            ("kebab-case", &["kebab", "case"]),
            ("Order.place", &["order", "place"]),
            ("src/shop/orders.py", &["src", "shop", "orders", "py"]),
            ("operator<", &["operator"]),
            ("Getting started", &["getting", "started"]),
            ("ÉtatCivil", &["état", "civil"]),
            ("日本語", &["日本語"]),
            ("ABC", &["abc"]),
            ("place place", &["place", "place"]),
            ("", &[]),
            // A decomposed accent (U+0301) stays in its word, and the hump after it still splits.
            ("cafe\u{301}s", &["cafe\u{301}s"]),
            ("cafe\u{301}Bar", &["cafe\u{301}", "bar"]),
            // Devanagari vowel signs and the virama are marks inside one word.
            ("हिन्दी", &["हिन्दी"]),
            // Curly quotes, dashes and ideographic punctuation separate.
            ("What\u{2019}s new", &["what", "s", "new"]),
            ("naïve\u{2014}approach", &["naïve", "approach"]),
            ("日本語\u{3001}テスト", &["日本語", "テスト"]),
            ("a\u{a0}b", &["a", "b"]),
        ];
        for (text, expected) in cases {
            assert_eq!(identifier_tokens(text), expected, "{text:?}");
        }
    }
}
