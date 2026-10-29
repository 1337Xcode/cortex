//! The `linguist-generated` and `linguist-vendored` attributes of a `.gitattributes` file, which
//! let a repository say which of its files a tool or a third party wrote.
//!
//! Git's pattern rules apply, matched relative to the folder the file is in: a pattern without a
//! slash matches a name at any depth, one with a slash is anchored to the folder, and a folder
//! pattern (`docs/`) does not reach the files inside it (`docs/**` does). Matching is
//! case-sensitive on every platform. A later matching line wins over an earlier one. `attr` sets
//! an attribute and `-attr` clears it; `attr=v` sets it unless `v` is `false`, as GitHub's linguist
//! reads these attributes; `!attr` forgets what earlier lines said. Each file's patterns are
//! compiled into one set, so a file is checked against all of them in one pass.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::patterns::literal_braces;

/// The attribute that marks files a tool wrote.
const GENERATED_ATTRIBUTE: &str = "linguist-generated";

/// The attribute that marks files a third party wrote.
const VENDORED_ATTRIBUTE: &str = "linguist-vendored";

/// What attributes say about one file; `None` where they say nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Whether a tool wrote the file.
    pub is_generated: Option<bool>,
    /// Whether a third party wrote the file.
    pub is_vendored: Option<bool>,
}

/// What one line sets an attribute to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    /// Set or cleared.
    To(bool),
    /// Back to unspecified.
    Forget,
}

/// What one line does to the two attributes.
#[derive(Debug, Clone, Copy, Default)]
struct Settings {
    /// What it does to `linguist-generated`.
    generated: Option<Setting>,
    /// What it does to `linguist-vendored`.
    vendored: Option<Setting>,
}

/// One attributes file's rules for the two attributes, in line order.
#[derive(Debug)]
pub struct AttributesFile {
    /// Each rule's pattern; the index of a pattern is the index of its settings.
    patterns: GlobSet,
    /// Each rule's settings.
    settings: Vec<Settings>,
}

/// What one line of an attributes file is to Cortex.
enum Line {
    /// A rule for an attribute Cortex reads: its glob and settings.
    Rule(String, Settings),
    /// A line that mentions such an attribute but cannot be used.
    Unusable,
    /// A blank line, a comment, or a line about other attributes.
    Other,
}

impl AttributesFile {
    /// The rules in an attributes file's `bytes`, and the 1-based numbers of the lines that
    /// cannot be used: a line that is not UTF-8, and a line about an attribute Cortex reads with
    /// a negative or invalid pattern, a quote left open, or a macro definition (`[attr]`), which
    /// Cortex does not expand.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> (Self, Vec<usize>) {
        let mut builder = GlobSetBuilder::new();
        let mut settings = Vec::new();
        let mut rule_lines = Vec::new();
        let mut unusable = Vec::new();
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let Ok(line) = std::str::from_utf8(line) else {
                unusable.push(index + 1);
                continue;
            };
            match read_line(line) {
                Line::Rule(glob, line_settings) => match compile(&glob) {
                    Some(compiled) => {
                        builder.add(compiled);
                        settings.push(line_settings);
                        rule_lines.push(index + 1);
                    }
                    None => unusable.push(index + 1),
                },
                Line::Unusable => unusable.push(index + 1),
                Line::Other => {}
            }
        }
        // Each glob compiled on its own above. Together they can still exceed the matcher's size
        // limits; then none of the file's rules apply, and each of their lines is reported.
        let patterns = if let Ok(patterns) = builder.build() {
            patterns
        } else {
            unusable.extend(rule_lines);
            unusable.sort_unstable();
            settings.clear();
            GlobSet::empty()
        };
        (Self { patterns, settings }, unusable)
    }

    /// Applies the lines whose patterns match `relative`, a path relative to this file's folder
    /// written with `/`, to `verdict`, in line order.
    pub fn apply(&self, relative: &str, verdict: &mut Verdict) {
        let mut matched = self.patterns.matches(relative);
        matched.sort_unstable();
        for settings in matched.iter().filter_map(|index| self.settings.get(*index)) {
            apply(&mut verdict.is_generated, settings.generated);
            apply(&mut verdict.is_vendored, settings.vendored);
        }
    }
}

/// Applies one line's setting to an attribute's state.
fn apply(state: &mut Option<bool>, setting: Option<Setting>) {
    match setting {
        Some(Setting::To(value)) => *state = Some(value),
        Some(Setting::Forget) => *state = None,
        None => {}
    }
}

/// The glob for a line's pattern, as `globset` reads it, relative to the file's folder; `None`
/// for a folder pattern, which never matches a file.
fn glob_for(pattern: &str) -> Option<String> {
    if pattern.ends_with('/') {
        return None;
    }
    if let Some(anchored) = pattern.strip_prefix('/') {
        return Some(literal_braces(anchored));
    }
    if pattern.contains('/') {
        Some(literal_braces(pattern))
    } else {
        Some(format!("**/{}", literal_braces(pattern)))
    }
}

/// The compiled glob, or `None` when it is invalid.
fn compile(glob: &str) -> Option<globset::Glob> {
    GlobBuilder::new(glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .ok()
}

/// What one line of an attributes file is to Cortex.
fn read_line(line: &str) -> Line {
    let line = line.trim();
    let mentions_ours = [GENERATED_ATTRIBUTE, VENDORED_ATTRIBUTE]
        .iter()
        .any(|name| line.contains(name));
    if line.starts_with('#') || !mentions_ours {
        return Line::Other;
    }
    if line.starts_with("[attr]") {
        return Line::Unusable;
    }
    // Git refuses negative patterns in attributes files, so such a line sets nothing.
    let Some((pattern, attributes)) =
        split_pattern(line).filter(|(pattern, _)| !pattern.starts_with('!'))
    else {
        return Line::Unusable;
    };
    let mut settings = Settings::default();
    for attribute in attributes.split_ascii_whitespace() {
        let (name, setting) = setting_of(attribute);
        match name {
            GENERATED_ATTRIBUTE => settings.generated = Some(setting),
            VENDORED_ATTRIBUTE => settings.vendored = Some(setting),
            _ => {}
        }
    }
    if settings.generated.is_none() && settings.vendored.is_none() {
        return Line::Other;
    }
    match glob_for(&pattern) {
        Some(glob) => Line::Rule(glob, settings),
        None => Line::Other,
    }
}

/// A line's pattern and the rest of it; `None` when a quoted pattern is not closed or its escapes
/// do not spell UTF-8.
fn split_pattern(line: &str) -> Option<(String, &str)> {
    let Some(quoted) = line.strip_prefix('"') else {
        let (pattern, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        return Some((pattern.to_owned(), rest));
    };
    let mut bytes = Vec::new();
    let mut characters = quoted.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => {
                let pattern = String::from_utf8(bytes).ok()?;
                return Some((pattern, quoted.get(index + 1..)?));
            }
            '\\' => bytes.push(unescape(&mut characters)?),
            other => {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    None
}

/// The byte one C-style escape in a quoted pattern stands for, reading after its backslash: one
/// of `\a \b \f \n \r \t \v \\ \"`, or three octal digits.
fn unescape(characters: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Option<u8> {
    let (_, first) = characters.next()?;
    let byte = match first {
        'a' => 0x07,
        'b' => 0x08,
        'f' => 0x0c,
        'n' => b'\n',
        'r' => b'\r',
        't' => b'\t',
        'v' => 0x0b,
        '\\' => b'\\',
        '"' => b'"',
        '0'..='3' => {
            let mut value = first.to_digit(8)?;
            for _ in 0..2 {
                let (_, digit) = characters.next()?;
                value = value * 8 + digit.to_digit(8)?;
            }
            u8::try_from(value).ok()?
        }
        _ => return None,
    };
    Some(byte)
}

/// An attribute's name and what one `attr`, `-attr`, `!attr` or `attr=value` does to it.
fn setting_of(attribute: &str) -> (&str, Setting) {
    if let Some(name) = attribute.strip_prefix('-') {
        return (name, Setting::To(false));
    }
    if let Some(name) = attribute.strip_prefix('!') {
        return (name, Setting::Forget);
    }
    match attribute.split_once('=') {
        None => (attribute, Setting::To(true)),
        Some((name, value)) => (name, Setting::To(value != "false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `file`'s rules say about `relative`.
    fn verdict_of(file: &AttributesFile, relative: &str) -> Verdict {
        let mut verdict = Verdict::default();
        file.apply(relative, &mut verdict);
        verdict
    }

    /// A verdict from two optional answers.
    fn verdict(is_generated: Option<bool>, is_vendored: Option<bool>) -> Verdict {
        Verdict {
            is_generated,
            is_vendored,
        }
    }

    /// Setting, clearing, values and forgetting, with a later line winning over an earlier one,
    /// and names matching at any depth while slashed patterns are anchored.
    #[test]
    fn lines_set_clear_and_forget() {
        let (file, unusable) = AttributesFile::parse(
            b"*.pb.go linguist-generated\n\
             gen/** linguist-generated=true linguist-vendored\n\
             gen/keep.go -linguist-generated\n\
             gen/maybe.go linguist-generated=false !linguist-vendored\n\
             docs/** linguist-vendored=yes text eol=lf\n\
             /top.c linguist-generated\n",
        );
        let cases = [
            ("api/user.pb.go", verdict(Some(true), None)),
            ("gen/tables.rs", verdict(Some(true), Some(true))),
            ("gen/keep.go", verdict(Some(false), Some(true))),
            ("gen/maybe.go", verdict(Some(false), None)),
            ("docs/guide.md", verdict(None, Some(true))),
            ("lib/gen/a.rs", verdict(None, None)),
            ("top.c", verdict(Some(true), None)),
            ("src/top.c", verdict(None, None)),
            ("src/main.go", verdict(None, None)),
        ];
        for (relative, expected) in cases {
            assert_eq!(verdict_of(&file, relative), expected, "{relative}");
        }
        assert_eq!(unusable, Vec::<usize>::new());
    }

    /// A folder pattern does not reach the files inside the folder; `folder/**` does. Matching is
    /// case-sensitive.
    #[test]
    fn folder_patterns_and_case() {
        let (file, _) = AttributesFile::parse(
            b"third_party/ linguist-vendored\nlibs/** linguist-vendored\n*.JS linguist-generated\n",
        );
        assert_eq!(verdict_of(&file, "third_party/x.c"), verdict(None, None));
        assert_eq!(
            verdict_of(&file, "libs/deep/x.c"),
            verdict(None, Some(true))
        );
        assert_eq!(verdict_of(&file, "a.js"), verdict(None, None));
        assert_eq!(verdict_of(&file, "a.JS"), verdict(Some(true), None));
    }

    /// Quoted patterns may hold spaces and C-style escapes, octal bytes included; unusable lines
    /// are reported by number and lines about other attributes are left alone.
    #[test]
    fn quoted_and_unusable_lines() {
        let (file, unusable) = AttributesFile::parse(
            b"\"gen files/*.go\" linguist-generated\n\
             # a comment about linguist-generated\n\
             \"open quote linguist-generated\n\
             !negated linguist-generated\n\
             *.txt text\n\
             a[z-a].c linguist-vendored\n\
             [attr]gen linguist-generated\n\
             \"caf\\303\\251.go\" linguist-vendored\n\
             \"bad\\q.go\" linguist-vendored\n",
        );
        assert_eq!(
            verdict_of(&file, "gen files/x.go"),
            verdict(Some(true), None)
        );
        assert_eq!(verdict_of(&file, "café.go"), verdict(None, Some(true)));
        assert_eq!(verdict_of(&file, "agen"), verdict(None, None));
        assert_eq!(unusable, vec![3, 4, 6, 7, 9]);
    }

    /// Braces are literal, as git has no alternation, and a line that is not UTF-8 is reported.
    #[test]
    fn braces_and_bad_bytes() {
        let (file, unusable) =
            AttributesFile::parse(b"{a,b}.h linguist-generated\n\xff\xfe linguist-vendored\n");
        assert_eq!(verdict_of(&file, "{a,b}.h"), verdict(Some(true), None));
        assert_eq!(verdict_of(&file, "a.h"), verdict(None, None));
        assert_eq!(unusable, vec![2]);
    }
}
