//! Go: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Go files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Go,
    extensions: &["go"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["go", "golang"],
    tags: TagsSpec {
        grammar,
        // Cortex's patterns first: they name methods by their type, read structs and interfaces
        // as such and leave built-in types out, where the grammar's own patterns would not.
        queries: &[
            include_str!("../../queries/go-tags.scm"),
            tree_sitter_go::TAGS_QUERY,
        ],
        imports: include_str!("../../queries/go-imports.scm"),
        // `orders.Place()`, `o.Total()` and the type `orders.Order`: a name reached through a
        // package or a value.
        qualified_by: &[
            ("selector_expression", "operand"),
            ("qualified_type", "package"),
        ],
        // Functions and methods have a body, a constant its value, and a named type the type it
        // stands for.
        body_paths: &[&["body"], &["value"], &["type"]],
        test_files: &[NamePattern::Suffix("_test.go")],
        // What `go test` runs: tests, benchmarks, examples and fuzz targets.
        test_functions: &[
            NamePattern::Prefix("Test"),
            NamePattern::Prefix("Benchmark"),
            NamePattern::Prefix("Example"),
            NamePattern::Prefix("Fuzz"),
        ],
    },
};

/// The Go grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_go::LANGUAGE.into()
}
