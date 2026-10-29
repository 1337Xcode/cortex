//! Java: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Java files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Java,
    extensions: &["java"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["java"],
    tags: Some(TagsSpec {
        grammar,
        // Cortex's patterns first: they mark annotated tests and read base classes, enums,
        // records, annotation types, constructors and constants. The uses of type names come
        // last, after every definition and base.
        queries: &[
            include_str!("../../queries/java-tags.scm"),
            tree_sitter_java::TAGS_QUERY,
            include_str!("../../queries/java-type-uses.scm"),
        ],
        imports: include_str!("../../queries/java-imports.scm"),
        // Types, methods and constructors have a body, a constant its value.
        body_paths: &[&["body"], &["value"]],
        // The names Maven and Gradle run as unit and integration tests.
        test_files: &[
            NamePattern::Suffix("Test.java"),
            NamePattern::Suffix("Tests.java"),
            NamePattern::Suffix("IT.java"),
        ],
        // JUnit 3's convention; later frameworks mark tests with an annotation, read anywhere.
        test_functions: &[NamePattern::Prefix("test")],
    }),
};

/// The Java grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}
