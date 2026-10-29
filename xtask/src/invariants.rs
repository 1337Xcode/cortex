//! Invariant tags.
//!
//! Each line that enforces one of the product's invariants carries a comment naming it, so one
//! text search finds every enforcement point. This check refuses tags with unknown names (a typo
//! would hide an enforcement point) and, once the crate that owns an invariant exists, requires at
//! least one tag for it inside that crate.

use std::collections::BTreeSet;

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
}

/// Finds the tags in one file: a `//` comment containing the marker word and a colon, then a name.
pub(crate) fn scan(path: &str, text: &str) -> Vec<Tag> {
    let marker = format!("{MARKER_WORD}:");
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let (_, comment) = line.split_once("//")?;
            let (_, rest) = comment.split_once(marker.as_str())?;
            let name = rest
                .trim_start()
                .chars()
                .take_while(|character| character.is_ascii_lowercase() || *character == '-')
                .collect();
            Some(Tag {
                path: path.to_owned(),
                line: index + 1,
                name,
            })
        })
        .collect()
}

/// Every problem with `tags`, given the names of the crates that exist.
pub(crate) fn check(tags: &[Tag], crate_names: &BTreeSet<String>) -> Vec<String> {
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
        let owner_dir = format!("crates/{owner}/");
        let is_enforced = tags
            .iter()
            .any(|tag| tag.name == *name && tag.path.starts_with(&owner_dir));
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
            check(&tags, &crates(&[])),
            vec!["crates/cortex-hook/src/lib.rs:1: unknown invariant \"fial-open\"".to_owned()]
        );
    }

    /// Once the owning crate exists, its invariant needs a tagged line inside it.
    #[test]
    fn existing_owner_needs_a_tag() {
        assert_eq!(
            check(&[], &crates(&["cortex-hook"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/".to_owned()]
        );
        let tags = scan("crates/cortex-hook/src/lib.rs", &tag_comment("fail-open"));
        assert_eq!(
            check(&tags, &crates(&["cortex-hook"])),
            Vec::<String>::new()
        );
    }

    /// A tag in another crate does not stand in for the owner's.
    #[test]
    fn tag_outside_the_owner_does_not_count() {
        let tags = scan("crates/cortex-cli/src/main.rs", &tag_comment("fail-open"));
        assert_eq!(
            check(&tags, &crates(&["cortex-hook", "cortex-cli"])),
            vec!["fail-open: no tagged enforcing line in crates/cortex-hook/".to_owned()]
        );
    }

    /// An invariant whose owning crate does not exist yet requires no tag.
    #[test]
    fn missing_owner_requires_nothing() {
        assert_eq!(
            check(&[], &crates(&["cortex-cli", "xtask"])),
            Vec::<String>::new()
        );
    }
}
