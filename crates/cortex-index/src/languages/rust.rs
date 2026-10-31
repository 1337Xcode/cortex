//! Rust: how a file is recognised as this language, and how it is parsed.

use cortex_core::{Language, SymbolKind};

use super::{Spec, TagsSpec};

/// How Rust files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Rust,
    extensions: &["rs"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["rust", "rs"],
    tags: TagsSpec {
        grammar,
        // Cortex's patterns first: they mark tests, name methods by their type and read type
        // aliases, module functions and inherent impls as such. The uses of type names come
        // last, after every definition.
        queries: &[
            include_str!("../../queries/rust-tags.scm"),
            tree_sitter_rust::TAGS_QUERY,
            include_str!("../../queries/rust-type-uses.scm"),
        ],
        imports: include_str!("../../queries/rust-imports.scm"),
        // `x.len()`, `Order::new()` and the type `models::Order`: a name reached through a
        // value, a type or a module path.
        qualified_by: &[
            ("field_expression", "value"),
            ("scoped_identifier", "path"),
            ("scoped_type_identifier", "path"),
        ],
        // Items with braces have a body, a constant or static its value, and a type alias the
        // type it stands for.
        closed_scopes: &[SymbolKind::Module],
        body_paths: &[&["body"], &["value"], &["type"]],
        // Every file under `tests/` is a test file; inside any file, `#[test]` marks a test.
        test_files: &[],
        test_functions: &[],
    },
};

/// The Rust grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_rust::LANGUAGE.into()
}
