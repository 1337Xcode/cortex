//! Swift: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Swift files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Swift,
    extensions: &["swift"],
    file_names: &[],
    interpreters: &["swift"],
    linguist_names: &["swift"],
    tags: Some(TagsSpec {
        grammar,
        // Cortex's own query: the grammar's own takes a method's whole class for the method.
        queries: &[include_str!("../../queries/swift-tags.scm")],
        imports: include_str!("../../queries/swift-imports.scm"),
        // Types, protocols, functions and initializers have a body. A constant's value has no
        // field of its own, so a constant is all header.
        body_paths: &[&["body"]],
        // The names test files take in Swift packages and Xcode projects, as `OrderTests.swift`.
        test_files: &[
            NamePattern::Suffix("Tests.swift"),
            NamePattern::Suffix("Test.swift"),
        ],
        // XCTest runs the methods named `test...`; Swift Testing marks tests with `@Test`, read
        // anywhere.
        test_functions: &[NamePattern::Prefix("test")],
    }),
};

/// The Swift grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_swift::LANGUAGE.into()
}
