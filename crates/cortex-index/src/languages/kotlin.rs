//! Kotlin: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Kotlin files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Kotlin,
    extensions: &["kt", "kts"],
    file_names: &[],
    interpreters: &["kotlin"],
    linguist_names: &["kotlin"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/kotlin-tags.scm")],
        imports: include_str!("../../queries/kotlin-imports.scm"),
        // The grammar names no field for a body, so the query captures each definition's body.
        body_paths: &[],
        // The names Gradle and Maven run as unit and integration tests.
        test_files: &[
            NamePattern::Suffix("Test.kt"),
            NamePattern::Suffix("Tests.kt"),
            NamePattern::Suffix("IT.kt"),
        ],
        // JUnit, kotlin.test and TestNG mark tests with an annotation, read anywhere; none goes by
        // name.
        test_functions: &[],
    },
};

/// The Kotlin grammar, which reads scripts (`.kts`) as well as source files.
fn grammar() -> tree_sitter::Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}
