//! Scala: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Scala files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Scala,
    extensions: &["scala", "sc", "sbt"],
    file_names: &[],
    interpreters: &["scala"],
    linguist_names: &["scala"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar's reads kinds outside the tags format.
        queries: &[include_str!("../../queries/scala-tags.scm")],
        imports: include_str!("../../queries/scala-imports.scm"),
        // Classes, objects, traits and functions have a body. A value's has a field of its own,
        // which is not read as a body, so a constant is all header.
        body_paths: &[&["body"]],
        // The names ScalaTest, MUnit and JUnit suites take, and those sbt runs as integration
        // tests, for the links from a test file to what it tests.
        test_files: &[
            NamePattern::Suffix("Spec.scala"),
            NamePattern::Suffix("Test.scala"),
            NamePattern::Suffix("Tests.scala"),
            NamePattern::Suffix("Suite.scala"),
            NamePattern::Suffix("IT.scala"),
        ],
        // ScalaTest's and MUnit's tests are strings passed to calls; JUnit's are annotated, read
        // anywhere. None goes by name.
        test_functions: &[],
    },
};

/// The Scala grammar, which reads Scala 2 and Scala 3.
fn grammar() -> tree_sitter::Language {
    tree_sitter_scala::LANGUAGE.into()
}
