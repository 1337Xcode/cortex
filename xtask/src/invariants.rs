//! Invariant tags.
//!
//! Each line that enforces one of the product's invariants carries a comment naming it, so one
//! text search finds every enforcement point. This check refuses tags with unknown names (a typo
//! would hide an enforcement point) and, once the crate that owns an invariant exists, requires at
//! least one tag for it in that crate's `src/`. A tag is a `//` line comment (found by
//! [`crate::comments`], so the marker inside a string literal or a block comment is no tag). A tag
//! in test code is checked for its name but enforces nothing: under `tests/`, inside an item
//! marked `#[cfg(test)]` or `#[test]`, or in the file of a module declared `#[cfg(test)] mod x;`.

use std::collections::BTreeSet;

use crate::comments;

/// The marker word. It is assembled at run time so this file never contains a tag itself.
const MARKER_WORD: &str = "INVARIANT";

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
    /// The invariant name as written after the marker.
    pub(crate) name: String,
    /// Whether the tag sits in an item that only compiles for tests.
    pub(crate) is_test: bool,
}

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
                name,
                is_test: comment.is_test,
            })
        })
        .collect()
}

/// The files holding the modules that the file at `path` declares test-only without a body:
/// `<name>.rs` and everything under `<name>/`, in the folder where that file's submodules live.
/// A `#[path]` attribute on such a declaration is not followed.
pub(crate) fn test_module_paths(path: &str, text: &str) -> Vec<String> {
    let Some((parent, file)) = path.rsplit_once('/') else {
        return Vec::new();
    };
    let module_dir = match file {
        "lib.rs" | "main.rs" | "mod.rs" => parent.to_owned(),
        _ => format!("{parent}/{}", file.trim_end_matches(".rs")),
    };
    comments::test_module_names(path, text)
        .into_iter()
        .flat_map(|name| {
            [
                format!("{module_dir}/{name}.rs"),
                format!("{module_dir}/{name}/"),
            ]
        })
        .collect()
}

/// Every problem with `tags`, given the test module files found by [`test_module_paths`] and
/// the names of the crates that exist.
pub(crate) fn check(
    tags: &[Tag],
    test_paths: &[String],
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
            tag.name == *name
                && !tag.is_test
                && tag.path.starts_with(&owner_dir)
                && !test_paths
                    .iter()
                    .any(|test| tag.path.starts_with(test.as_str()))
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

    /// A tag comment is found with its path and 1-based line; plain comments are not tags.
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
                name: "fail-open".to_owned(),
                is_test: false,
            }]
        );
    }

    /// The marker inside code that is not a comment is ignored.
    #[test]
    fn marker_outside_a_comment_is_not_a_tag() {
        let text = format!("let s = \"{MARKER_WORD}: fail-open\";\n");
        assert_eq!(scan("crates/x/src/lib.rs", &text), Vec::new());
    }

    /// A tag naming no known invariant is reported with its location.
    #[test]
    fn unknown_name_is_reported() {
        let tags = scan("crates/cortex-hook/src/lib.rs", &tag_comment("fial-open"));
        assert_eq!(
            check(&tags, &[], &crates(&[])),
            vec!["crates/cortex-hook/src/lib.rs:1: unknown invariant \"fial-open\"".to_owned()]
        );
    }

    /// Once the owning crate exists, its invariant needs a tagged line inside it.
    #[test]
    fn existing_owner_needs_a_tag() {
        assert_eq!(
            check(&[], &[], &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
        let tags = scan("crates/cortex-hook/src/lib.rs", &tag_comment("fail-open"));
        assert_eq!(
            check(&tags, &[], &crates(&["cortex-hook"])),
            Vec::<String>::new()
        );
    }

    /// A tag in another crate does not stand in for the owner's.
    #[test]
    fn tag_outside_the_owner_does_not_count() {
        let tags = scan("crates/cortex-cli/src/main.rs", &tag_comment("fail-open"));
        assert_eq!(
            check(&tags, &[], &crates(&["cortex-hook", "cortex-cli"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
    }

    /// A tag in the owner's tests is not an enforcing line.
    #[test]
    fn tag_in_owner_tests_does_not_count() {
        let tags = scan(
            "crates/cortex-hook/tests/hook.rs",
            &tag_comment("fail-open"),
        );
        assert_eq!(
            check(&tags, &[], &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
    }

    /// A tag inside the owner's `#[cfg(test)]` module is test code, not an enforcing line.
    #[test]
    fn tag_in_a_test_module_does_not_count() {
        let text = format!(
            "#[cfg(test)]
mod tests {{
{}
}}
",
            tag_comment("fail-open")
        );
        let tags = scan("crates/cortex-hook/src/lib.rs", &text);
        assert_eq!(tags.len(), 1);
        assert!(tags.iter().all(|tag| tag.is_test && tag.line == 3));
        assert_eq!(
            check(&tags, &[], &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
    }

    /// A test module kept in its own file is test code: its tags enforce nothing.
    #[test]
    fn tag_in_a_test_module_file_does_not_count() {
        let test_paths = test_module_paths(
            "crates/cortex-hook/src/lib.rs",
            "#[cfg(test)]
mod tests;
",
        );
        assert_eq!(
            test_paths,
            vec![
                "crates/cortex-hook/src/tests.rs".to_owned(),
                "crates/cortex-hook/src/tests/".to_owned(),
            ]
        );
        assert_eq!(
            test_module_paths(
                "crates/cortex-hook/src/gate.rs",
                "#[cfg(test)]
mod tests;
"
            ),
            vec![
                "crates/cortex-hook/src/gate/tests.rs".to_owned(),
                "crates/cortex-hook/src/gate/tests/".to_owned(),
            ]
        );
        let mut tags = scan("crates/cortex-hook/src/tests.rs", &tag_comment("fail-open"));
        tags.extend(scan(
            "crates/cortex-hook/src/tests/cases.rs",
            &tag_comment("fail-open"),
        ));
        assert_eq!(
            check(&tags, &test_paths, &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/src/".to_owned()]
        );
        let production = scan(
            "crates/cortex-hook/src/testing.rs",
            &tag_comment("fail-open"),
        );
        assert_eq!(
            check(&production, &test_paths, &crates(&["cortex-hook"])),
            Vec::<String>::new()
        );
    }

    /// The marker on the second line of a string that spans lines is not a tag.
    #[test]
    fn marker_in_a_multi_line_string_is_not_a_tag() {
        let text = format!(
            "let s = \"first line
// {MARKER_WORD}: fail-open\";
"
        );
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
            check(&[], &[], &crates(&["cortex-cli", "xtask"])),
            Vec::<String>::new()
        );
    }
}
