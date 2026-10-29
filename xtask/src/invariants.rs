//! Invariant tags.
//!
//! Each line that enforces one of the product's invariants carries a comment naming it, so one
//! text search finds every enforcement point. This check refuses tags with unknown names (a typo
//! would hide an enforcement point) and, once the crate that owns an invariant exists, requires at
//! least one enforcing tag for it in that crate's `src/`. A tag is a `//` line comment (found by
//! [`crate::comments`], so the marker inside a string literal or a block comment is no tag). Every
//! tag is checked for its name, but only a tag in code the product compiles enforces anything: not
//! in a file no target reaches, such as a test module's own file, and not inside code that
//! compiles only for tests ([`crate::compiled`]).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::comments;

/// The marker word. It is assembled at run time so this file never contains a tag itself.
pub(crate) const MARKER_WORD: &str = "INVARIANT";

/// Every invariant and the crate that must hold at least one enforcing line for it.
const INVARIANTS: &[(&str, &str)] = &[
    ("fail-open", "cortex-hook"),
    ("local-only", "cortex-daemon"),
    ("silent-when-unsure", "cortex-retrieve"),
    ("true-at-render", "cortex-retrieve"),
    ("no-content-stored", "cortex-store"),
    ("single-writer", "cortex-daemon"),
    ("deterministic", "cortex-retrieve"),
    ("budgeted", "cortex-retrieve"),
    ("measured-claims", "cortex-meter"),
];

/// One tagged line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Tag {
    /// Repository-relative path with `/` separators.
    pub(crate) path: String,
    /// One-based line number.
    pub(crate) line: usize,
    /// Byte offset of the comment in its file.
    pub(crate) offset: usize,
    /// The invariant name as written after the marker.
    pub(crate) name: String,
}

/// The files the product compiles, by repository-relative path, each with the byte ranges inside
/// it that compile only for tests.
pub(crate) type CompiledCode = BTreeMap<String, Vec<Range<usize>>>;

/// Finds the tags in one file: a `//` comment containing the marker word and a colon, then a name.
pub(crate) fn scan(path: &str, text: &str) -> Vec<Tag> {
    let marker = format!("{MARKER_WORD}:");
    comments::line_comments(path, text)
        .into_iter()
        .filter_map(|comment| {
            let (_, rest) = comment.text.split_once(marker.as_str())?;
            let name = rest
                .trim_start()
                .chars()
                .take_while(|character| character.is_ascii_lowercase() || *character == '-')
                .collect();
            Some(Tag {
                path: path.to_owned(),
                line: comment.line,
                offset: comment.offset,
                name,
            })
        })
        .collect()
}

/// Whether a tag sits in code the product compiles, outside test-only ranges.
fn is_compiled(tag: &Tag, compiled: &CompiledCode) -> bool {
    compiled
        .get(&tag.path)
        .is_some_and(|test_ranges| !test_ranges.iter().any(|range| range.contains(&tag.offset)))
}

/// Every problem with `tags`, given the code the product compiles and the crates that exist.
pub(crate) fn check(
    tags: &[Tag],
    compiled: &CompiledCode,
    crate_names: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems: Vec<String> = tags
        .iter()
        .filter(|tag| !INVARIANTS.iter().any(|(name, _)| *name == tag.name))
        .map(|tag| {
            format!(
                "{}:{}: unknown invariant {:?}",
                tag.path, tag.line, tag.name
            )
        })
        .collect();
    for (name, owner) in INVARIANTS {
        if !crate_names.contains(*owner) {
            continue;
        }
        let owner_dir = format!("crates/{owner}/src/");
        let is_enforced = tags.iter().any(|tag| {
            tag.name == *name && tag.path.starts_with(&owner_dir) && is_compiled(tag, compiled)
        });
        if !is_enforced {
            problems.push(format!("{name}: no tagged enforcing line in {owner_dir}"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag comment for `name`, built so this file holds no real tag.
    fn tag_comment(name: &str) -> String {
        format!("    let x = 1; // {MARKER_WORD}: {name}")
    }

    /// The set of crate names given as literals.
    fn crates(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// Compiled files with the given test-only ranges.
    fn compiled(files: &[(&str, Vec<Range<usize>>)]) -> CompiledCode {
        files
            .iter()
            .map(|(path, ranges)| ((*path).to_owned(), ranges.clone()))
            .collect()
    }

    /// A tag comment is found with its path, 1-based line and byte offset; plain comments are not
    /// tags. Offset 24: "fn f() {\n" is 9 bytes, then 15 bytes of code before the slashes.
    #[test]
    fn scan_finds_tags_with_line_numbers() {
        let text = format!(
            "fn f() {{\n{}\n}}\n// plain comment\n",
            tag_comment("fail-open")
        );
        assert_eq!(
            scan("crates/cortex-hook/src/lib.rs", &text),
            vec![Tag {
                path: "crates/cortex-hook/src/lib.rs".to_owned(),
                line: 2,
                offset: 24,
                name: "fail-open".to_owned(),
            }]
        );
    }

    /// The marker inside code that is not a comment is ignored.
    #[test]
    fn marker_outside_a_comment_is_not_a_tag() {
        let text = format!("let s = \"{MARKER_WORD}: fail-open\";\n");
        assert_eq!(scan("crates/x/src/lib.rs", &text), Vec::new());
    }

    /// A tag naming no known invariant is reported with its location, wherever it sits.
    #[test]
    fn unknown_name_is_reported() {
        let tags = scan(
            "crates/cortex-hook/tests/hook.rs",
            &tag_comment("fial-open"),
        );
        assert_eq!(
            check(&tags, &CompiledCode::new(), &crates(&[])),
            vec!["crates/cortex-hook/tests/hook.rs:1: unknown invariant \"fial-open\"".to_owned()]
        );
    }

    /// Once the owning crate exists, its invariant needs a tagged line it compiles.
    #[test]
    fn existing_owner_needs_a_tag() {
        let lib = "crates/cortex-hook/src/lib.rs";
        let code = compiled(&[(lib, Vec::new())]);
        assert_eq!(
            check(&[], &code, &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
        let tags = scan(lib, &tag_comment("fail-open"));
        assert_eq!(
            check(&tags, &code, &crates(&["cortex-hook"])),
            Vec::<String>::new()
        );
    }

    /// A tag in another crate does not stand in for the owner's.
    #[test]
    fn tag_outside_the_owner_does_not_count() {
        let main = "crates/cortex-cli/src/main.rs";
        let tags = scan(main, &tag_comment("fail-open"));
        assert_eq!(
            check(
                &tags,
                &compiled(&[(main, Vec::new())]),
                &crates(&["cortex-hook", "cortex-cli"])
            ),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
    }

    /// A tag inside a test-only range of a compiled file, or in a file no target compiles (an
    /// integration test, a test module's own file), enforces nothing.
    #[test]
    fn tags_in_test_code_do_not_count() {
        let lib = "crates/cortex-hook/src/lib.rs";
        let mut tags = scan(lib, &tag_comment("fail-open"));
        tags.extend(scan(
            "crates/cortex-hook/src/tests.rs",
            &tag_comment("fail-open"),
        ));
        tags.extend(scan(
            "crates/cortex-hook/tests/hook.rs",
            &tag_comment("fail-open"),
        ));
        let code = compiled(&[(lib, vec![Range { start: 0, end: 30 }])]);
        assert_eq!(
            check(&tags, &code, &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
        let outside = compiled(&[(lib, vec![Range { start: 0, end: 10 }])]);
        assert_eq!(
            check(&tags, &outside, &crates(&["cortex-hook"])),
            Vec::<String>::new()
        );
    }

    /// The marker on the second line of a string that spans lines is not a tag.
    #[test]
    fn marker_in_a_multi_line_string_is_not_a_tag() {
        let text = format!("let s = \"first line\n// {MARKER_WORD}: fail-open\";\n");
        assert_eq!(scan("crates/cortex-hook/src/lib.rs", &text), Vec::new());
    }

    /// `//` inside a string literal starts no comment, in Rust or TypeScript.
    #[test]
    fn marker_in_a_string_literal_is_not_a_tag() {
        let rust = format!("let s = \"// {MARKER_WORD}: fail-open\";\n");
        assert_eq!(scan("crates/x/src/lib.rs", &rust), Vec::new());
        let typescript = format!("const s = '// {MARKER_WORD}: fail-open';\n");
        assert_eq!(scan("ui/src/a.ts", &typescript), Vec::new());
        let template = format!("const s = `// {MARKER_WORD}: fail-open`;\n");
        assert_eq!(scan("ui/src/a.tsx", &template), Vec::new());
    }

    /// A comment after code that holds strings, escapes or lifetimes is still found.
    #[test]
    fn tag_after_strings_and_lifetimes_is_found() {
        let lines = [
            format!("let s = \"a//b\\\"c\"; // {MARKER_WORD}: fail-open"),
            format!("fn f<'a>(x: &'a str) {{}} // {MARKER_WORD}: fail-open"),
        ];
        for line in lines {
            let tags = scan("crates/cortex-hook/src/lib.rs", &line);
            assert_eq!(tags.len(), 1, "{line}");
            assert_eq!(tags[0].name, "fail-open");
        }
    }

    /// An invariant whose owning crate does not exist yet requires no tag.
    #[test]
    fn missing_owner_requires_nothing() {
        assert_eq!(
            check(&[], &CompiledCode::new(), &crates(&["cortex-cli", "xtask"])),
            Vec::<String>::new()
        );
    }
}
