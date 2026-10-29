//! The `//` comments of a source file.
//!
//! The invariant check needs every line comment that is real commentary, not text inside a string
//! literal or a block comment. A full parser is the wrong tool for finding comments (parsers drop
//! them), so this is a small lexer that knows the constructs that can hide or fake one: strings
//! that span lines, raw strings, character literals (which may hold a quote), lifetimes (which look
//! like an unclosed one) and nested block comments. Which comments sit in test-only code is decided
//! separately, by parsing ([`crate::compiled`]).
//!
//! Script files (TypeScript and JavaScript) get single-line `'` and `"` strings, template literals
//! and block comments. Regular expression literals, JSX text and templates nested inside `${...}`
//! are not recognised.

use std::ops::Range;

/// One `//` comment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LineComment<'a> {
    /// One-based line number.
    pub(crate) line: usize,
    /// Byte offset of the first slash.
    pub(crate) offset: usize,
    /// The text after the two slashes, up to the end of the line.
    pub(crate) text: &'a str,
}

/// The languages the lexer tells apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    /// Rust: nested block comments, raw strings and character literals.
    Rust,
    /// TypeScript and JavaScript: three string quotes, flat block comments.
    Script,
}

/// Every `//` comment in `text`, in order. The file extension of `path` picks the language.
pub(crate) fn line_comments<'a>(path: &str, text: &'a str) -> Vec<LineComment<'a>> {
    let language = if path.ends_with(".rs") {
        Language::Rust
    } else {
        Language::Script
    };
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    comment_ranges(text.as_bytes(), language)
        .into_iter()
        .map(|range| LineComment {
            line: line_starts.partition_point(|start| *start <= range.start),
            offset: range.start,
            text: text.get(range.start + 2..range.end).unwrap_or_default(),
        })
        .collect()
}

/// The byte range of each `//` comment in `bytes`, from the first slash to the end of the line.
fn comment_ranges(bytes: &[u8], language: Language) -> Vec<Range<usize>> {
    let mut comments = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        index = match (byte, next, language) {
            (b'/', Some(b'/'), _) => {
                let end = line_end(bytes, index);
                comments.push(index..end);
                end
            }
            (b'/', Some(b'*'), _) => block_comment_end(bytes, index, language == Language::Rust),
            (b'"', _, Language::Rust) => quoted_end(bytes, index, true),
            (b'"' | b'\'', _, Language::Script) => quoted_end(bytes, index, false),
            (b'\'', _, Language::Rust) => char_literal_end(bytes, index).unwrap_or(index + 1),
            (b'`', _, Language::Script) => quoted_end(bytes, index, true),
            (b'r', _, Language::Rust) => raw_string_end(bytes, index).unwrap_or(index + 1),
            _ => index + 1,
        };
    }
    comments
}

/// The index of the newline that ends the line holding `index`, or the end of the input.
fn line_end(bytes: &[u8], index: usize) -> usize {
    bytes
        .iter()
        .skip(index)
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| index + offset)
}

/// The end of the block comment opening at `start`. Rust block comments nest; script ones do not.
fn block_comment_end(bytes: &[u8], start: usize, nests: bool) -> usize {
    let mut depth = 0_usize;
    let mut index = start;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        match (byte, next) {
            (b'/', Some(b'*')) if depth == 0 || nests => {
                depth += 1;
                index += 2;
            }
            (b'*', Some(b'/')) => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The end of the quoted literal opening at `start`, which backslashes escape. A literal that may
/// not span lines also ends at the end of its line, so one unclosed quote cannot swallow the file.
fn quoted_end(bytes: &[u8], start: usize, spans_lines: bool) -> usize {
    let Some(&quote) = bytes.get(start) else {
        return bytes.len();
    };
    let mut index = start + 1;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\\' => index += 2,
            b'\n' if !spans_lines => return index,
            _ if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The end of the Rust character literal opening at `start`, or `None` when the quote begins a
/// lifetime or a label (`'a`, `'outer:`) instead.
fn char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start + 1)?;
    if first == b'\\' {
        // An escape such as `'\''` or `'\u{7f}'`: skip the escaped byte, then find the close.
        let close = start
            + 3
            + bytes
                .get(start + 3..)?
                .iter()
                .position(|byte| *byte == b'\'')?;
        return (!bytes.get(start + 1..close)?.contains(&b'\n')).then_some(close + 1);
    }
    let after = start + 1 + utf8_len(first);
    (bytes.get(after) == Some(&b'\'')).then_some(after + 1)
}

/// The length in bytes of the UTF-8 character whose first byte is `lead`.
fn utf8_len(lead: u8) -> usize {
    match lead.leading_ones() {
        2 => 2,
        3 => 3,
        4 => 4,
        _ => 1,
    }
}

/// The end of the Rust raw string (`r"..."`, `r#"..."#`, also after a `b` or `c` prefix) whose
/// `r` is at `start`, or `None` when the `r` is part of something else, such as an identifier or
/// a raw identifier (`r#type`).
fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let prefix: Vec<u8> = bytes
        .get(..start)?
        .iter()
        .rev()
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .copied()
        .collect();
    if !matches!(prefix.as_slice(), [] | [b'b'] | [b'c']) {
        return None;
    }
    let hashes = bytes
        .get(start + 1..)?
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    let open = start + 1 + hashes;
    if bytes.get(open) != Some(&b'"') {
        return None;
    }
    let mut closing = vec![b'"'];
    closing.extend(std::iter::repeat_n(b'#', hashes));
    let body = open + 1;
    let found = bytes
        .get(body..)?
        .windows(closing.len())
        .position(|window| window == closing.as_slice());
    Some(found.map_or(bytes.len(), |offset| body + offset + closing.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line numbers and texts of every comment in `source`.
    fn summary(path: &str, source: &str) -> Vec<(usize, String)> {
        line_comments(path, source)
            .into_iter()
            .map(|comment| (comment.line, comment.text.to_owned()))
            .collect()
    }

    /// Each literal below holds `//` or a quote that a line-by-line scan misreads; only the five
    /// real comments, one per line from line 3, are found.
    #[test]
    fn rust_literals_hide_slashes() {
        let source = concat!(
            "let a = \"one // in a string\n",
            "two // still in the string\";\n",
            "let b = r#\"raw \" // in a raw string\"#; // c1\n",
            "let c = '\"'; let d = \"open\"; // c2\n",
            "fn f<'a>(x: &'a str) -> &'a str { x } // c3\n",
            "/* outer /* nested */ // hidden */ // c4\n",
            "let e = b\"bytes // in\"; let g = '\\''; let h = br\"x\"; // c5\n",
        );
        assert_eq!(
            summary("crates/x/src/lib.rs", source),
            vec![
                (3, " c1".to_owned()),
                (4, " c2".to_owned()),
                (5, " c3".to_owned()),
                (6, " c4".to_owned()),
                (7, " c5".to_owned()),
            ]
        );
    }

    /// A comment's offset is the byte position of its first slash.
    #[test]
    fn offsets_point_at_the_first_slash() {
        let source = "let a = 1; // one\n// two\n";
        let offsets: Vec<usize> = line_comments("crates/x/src/lib.rs", source)
            .iter()
            .map(|comment| comment.offset)
            .collect();
        assert_eq!(offsets, vec![11, 18]);
    }

    /// Script strings of all three quotes hide slashes, templates span lines, block comments
    /// hide line comments.
    #[test]
    fn script_literals_hide_slashes() {
        let source = concat!(
            "const a = `one // in a template\n",
            "two`; // s1\n",
            "const b = '// in single'; const c = \"// in double\"; // s2\n",
            "/* block // hidden */ // s3\n",
        );
        assert_eq!(
            summary("ui/src/a.ts", source),
            vec![
                (2, " s1".to_owned()),
                (3, " s2".to_owned()),
                (4, " s3".to_owned()),
            ]
        );
    }

    /// A stray script quote, as in a regular expression, ends at its line: it neither hides the
    /// next line's comment nor turns a later string into one.
    #[test]
    fn stray_script_quotes_end_at_the_line() {
        assert_eq!(
            summary("ui/src/a.ts", "const re = /\"/;\n// s1\n"),
            vec![(2, " s1".to_owned())]
        );
        assert_eq!(
            summary(
                "ui/src/a.ts",
                "const re = /\"/;\nconst s = \"// not one\";\n"
            ),
            vec![]
        );
    }

    /// Unclosed literals and comments end at the end of the input instead of failing.
    #[test]
    fn unclosed_constructs_end_at_the_input_end() {
        assert_eq!(
            summary("crates/x/src/lib.rs", "let a = \"open // x"),
            vec![]
        );
        assert_eq!(summary("crates/x/src/lib.rs", "/* open // x"), vec![]);
        assert_eq!(
            summary("crates/x/src/lib.rs", "let r = r#\"open // x"),
            vec![]
        );
        assert_eq!(
            summary("ui/src/a.ts", "const a = 'open\n// s1"),
            vec![(2, " s1".to_owned())]
        );
    }
}
