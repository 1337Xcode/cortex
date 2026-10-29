//! The `//` comments of a source file.
//!
//! The invariant check needs every line comment that is real commentary: not text inside a string
//! literal or a block comment, and for Rust, marked when it sits in code that only compiles for
//! tests. A full parser is the wrong tool for a build script, so this is a small lexer that knows
//! the constructs that can hide or fake a comment: strings that span lines, raw strings, character
//! literals (which may hold a quote), lifetimes (which look like an unclosed one), nested block
//! comments, and items marked `#[cfg(test)]` or `#[test]`.
//!
//! Script files (TypeScript and JavaScript) get strings, template literals and block comments.
//! Regular expression literals, JSX text and templates nested inside `${...}` are not recognised.

use std::ops::Range;

/// One `//` comment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LineComment<'a> {
    /// One-based line number.
    pub(crate) line: usize,
    /// The text after the two slashes, up to the end of the line.
    pub(crate) text: &'a str,
    /// Whether the comment sits inside a Rust item that only compiles for tests.
    pub(crate) is_test: bool,
}

/// The languages the lexer tells apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    /// Rust: nested block comments, raw strings, character literals and test attributes.
    Rust,
    /// TypeScript and JavaScript: three string quotes, flat block comments.
    Script,
}

/// The lexer's output: the comments, and the source with every comment and literal blanked out.
struct Lexed {
    /// Byte range of each `//` comment, from the first slash to the end of the line.
    comments: Vec<Range<usize>>,
    /// The source bytes with comments and literal contents replaced by spaces; newlines are kept,
    /// so byte offsets and line numbers match the source.
    code: Vec<u8>,
}

/// Every `//` comment in `text`, in order. The file extension of `path` picks the language.
pub(crate) fn line_comments<'a>(path: &str, text: &'a str) -> Vec<LineComment<'a>> {
    let language = if path.ends_with(".rs") {
        Language::Rust
    } else {
        Language::Script
    };
    let lexed = lex(text.as_bytes(), language);
    let test_items = match language {
        Language::Rust => test_items(&lexed.code),
        Language::Script => Vec::new(),
    };
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    lexed
        .comments
        .into_iter()
        .map(|range| LineComment {
            line: line_starts.partition_point(|start| *start <= range.start),
            text: text.get(range.start + 2..range.end).unwrap_or_default(),
            is_test: test_items
                .iter()
                .any(|item| item.span.contains(&range.start)),
        })
        .collect()
}

/// The modules a Rust file declares test-only without a body (`#[cfg(test)] mod tests;`): their
/// code lives in a file of its own, which the caller must treat as test code too.
pub(crate) fn test_module_names(path: &str, text: &str) -> Vec<String> {
    if !path.ends_with(".rs") {
        return Vec::new();
    }
    let lexed = lex(text.as_bytes(), Language::Rust);
    test_items(&lexed.code)
        .into_iter()
        .filter_map(|item| {
            let declaration = std::str::from_utf8(lexed.code.get(item.body)?).ok()?;
            let words: Vec<&str> = declaration
                .trim()
                .strip_suffix(';')?
                .split_whitespace()
                .collect();
            match words.as_slice() {
                [.., "mod", name]
                    if name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_') =>
                {
                    Some((*name).to_owned())
                }
                _ => None,
            }
        })
        .collect()
}

/// Splits `bytes` into comments, literals and code.
fn lex(bytes: &[u8], language: Language) -> Lexed {
    let mut lexed = Lexed {
        comments: Vec::new(),
        code: bytes.to_vec(),
    };
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        let end = match (byte, next, language) {
            (b'/', Some(b'/'), _) => {
                let end = line_end(bytes, index);
                lexed.comments.push(index..end);
                end
            }
            (b'/', Some(b'*'), _) => block_comment_end(bytes, index, language == Language::Rust),
            (b'"', _, _) => quoted_end(bytes, index, true),
            (b'\'', _, Language::Rust) => char_literal_end(bytes, index).unwrap_or(index + 1),
            (b'\'', _, Language::Script) => quoted_end(bytes, index, false),
            (b'`', _, Language::Script) => quoted_end(bytes, index, true),
            (b'r', _, Language::Rust) => raw_string_end(bytes, index).unwrap_or(index + 1),
            _ => index + 1,
        };
        if end > index + 1 {
            blank(&mut lexed.code, index..end);
        }
        index = end;
    }
    lexed
}

/// Replaces every byte in `range` except newlines with a space.
fn blank(code: &mut [u8], range: Range<usize>) {
    for byte in code.get_mut(range).into_iter().flatten() {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
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

/// Code that only compiles for tests, as byte ranges of the blanked source.
struct TestItem {
    /// From the attribute's `#` to the end of the item or block.
    span: Range<usize>,
    /// What follows the attribute, up to the same end.
    body: Range<usize>,
}

/// Every item that only compiles for tests: an item after `#[cfg(test)]` or `#[test]` up to its
/// closing brace or semicolon, and the rest of the enclosing block after an inner
/// `#![cfg(test)]`. Other `cfg` forms, such as `cfg(not(test))`, are not test code.
fn test_items(code: &[u8]) -> Vec<TestItem> {
    let mut items = Vec::new();
    let mut index = 0;
    while let Some(offset) = code
        .get(index..)
        .and_then(|rest| rest.iter().position(|byte| *byte == b'#'))
    {
        let start = index + offset;
        match attribute_at(code, start) {
            Some((is_inner, content, after)) => {
                if is_test_attribute(content) {
                    let end = if is_inner {
                        enclosing_block_end(code, after)
                    } else {
                        item_end(code, after)
                    };
                    items.push(TestItem {
                        span: start..end,
                        body: after..end,
                    });
                }
                index = after;
            }
            None => index = start + 1,
        }
    }
    items
}

/// The attribute whose `#` is at `start`: whether it is inner (`#!`), the text between its
/// brackets, and the index after its closing bracket.
fn attribute_at(code: &[u8], start: usize) -> Option<(bool, &[u8], usize)> {
    let mut index = start + 1;
    let is_inner = code.get(index) == Some(&b'!');
    if is_inner {
        index += 1;
    }
    while code.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    if code.get(index) != Some(&b'[') {
        return None;
    }
    let close = matching_close(code, index, b'[', b']')?;
    Some((is_inner, code.get(index + 1..close)?, close + 1))
}

/// Whether an attribute's content, whitespace ignored, is `test` or `cfg(test)`.
fn is_test_attribute(content: &[u8]) -> bool {
    let compact: Vec<u8> = content
        .iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .copied()
        .collect();
    compact == b"test" || compact == b"cfg(test)"
}

/// The end of the item that starts after an attribute: past its matching closing brace, or past
/// the first semicolon outside brackets and parentheses when no brace comes first.
fn item_end(code: &[u8], from: usize) -> usize {
    let mut depth = 0_usize;
    let mut index = from;
    while let Some(&byte) = code.get(index) {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => return index + 1,
            b'{' if depth == 0 => {
                return matching_close(code, index, b'{', b'}')
                    .map_or(code.len(), |close| close + 1);
            }
            _ => {}
        }
        index += 1;
    }
    code.len()
}

/// The index of the brace that closes the block enclosing `from`, or the end of the input.
fn enclosing_block_end(code: &[u8], from: usize) -> usize {
    let mut depth = 0_usize;
    for (offset, byte) in code.iter().enumerate().skip(from) {
        match byte {
            b'{' => depth += 1,
            b'}' if depth == 0 => return offset,
            b'}' => depth -= 1,
            _ => {}
        }
    }
    code.len()
}

/// The index of the bracket that closes the one at `open`, counting nested pairs.
fn matching_close(code: &[u8], open: usize, opener: u8, closer: u8) -> Option<usize> {
    let mut depth = 0_usize;
    for (offset, byte) in code.iter().enumerate().skip(open) {
        if *byte == opener {
            depth += 1;
        } else if *byte == closer {
            depth -= 1;
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line numbers, texts and test flags of every comment in `source`.
    fn summary(path: &str, source: &str) -> Vec<(usize, String, bool)> {
        line_comments(path, source)
            .into_iter()
            .map(|comment| (comment.line, comment.text.to_owned(), comment.is_test))
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
                (3, " c1".to_owned(), false),
                (4, " c2".to_owned(), false),
                (5, " c3".to_owned(), false),
                (6, " c4".to_owned(), false),
                (7, " c5".to_owned(), false),
            ]
        );
    }

    /// Comments inside a `#[cfg(test)]` module or a `#[test]` function are test code; a brace in
    /// a string does not end the module early, and `cfg(not(test))` is production code.
    #[test]
    fn rust_test_items_are_marked() {
        let source = concat!(
            "fn real() {} // r1\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    fn helper() { let s = \"}\"; } // t1\n",
            "    #[test]\n",
            "    fn case() {} // t2\n",
            "}\n",
            "#[cfg(not(test))]\n",
            "fn production() {\n",
            "    // r2\n",
            "}\n",
            "#[cfg( test )]\n",
            "use std::fmt;\n",
            "// r3\n",
            "#[test]\n",
            "fn standalone() {\n",
            "    // t3\n",
            "}\n",
        );
        assert_eq!(
            summary("crates/x/src/lib.rs", source),
            vec![
                (1, " r1".to_owned(), false),
                (4, " t1".to_owned(), true),
                (6, " t2".to_owned(), true),
                (10, " r2".to_owned(), false),
                (14, " r3".to_owned(), false),
                (17, " t3".to_owned(), true),
            ]
        );
    }

    /// A test module declared without a body is named, with or without visibility; an inline
    /// test module, a production module and a `cfg(not(test))` module are not.
    #[test]
    fn bodiless_test_modules_are_named() {
        let source = concat!(
            "#[cfg(test)]
",
            "mod tests;
",
            "#[cfg(test)]
",
            "pub(crate) mod test_support;
",
            "#[cfg(test)]
",
            "mod inline {}
",
            "mod production;
",
            "#[cfg(not(test))]
",
            "mod real;
",
        );
        assert_eq!(
            test_module_names("crates/x/src/lib.rs", source),
            vec!["tests".to_owned(), "test_support".to_owned()]
        );
        assert_eq!(
            test_module_names("ui/src/a.ts", source),
            Vec::<String>::new()
        );
    }

    /// An inner `#![cfg(test)]` makes the rest of its file test code.
    #[test]
    fn inner_test_attribute_covers_the_rest_of_the_file() {
        let source = "// r1\n#![cfg(test)]\n// t1\nfn x() {} // t2\n";
        assert_eq!(
            summary("crates/x/src/lib.rs", source),
            vec![
                (1, " r1".to_owned(), false),
                (3, " t1".to_owned(), true),
                (4, " t2".to_owned(), true),
            ]
        );
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
                (2, " s1".to_owned(), false),
                (3, " s2".to_owned(), false),
                (4, " s3".to_owned(), false),
            ]
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
            vec![(2, " s1".to_owned(), false)]
        );
    }
}
