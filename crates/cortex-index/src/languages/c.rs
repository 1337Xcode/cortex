//! C; a `.h` header counts as C unless its start reads as C++: how a file is recognised as this
//! language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How C files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::C,
    extensions: &["c", "h"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["c"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar's reads no references and gives functions the span of
        // their declarator alone.
        queries: &[include_str!("../../queries/c-tags.scm")],
        imports: include_str!("../../queries/c-imports.scm"),
        qualified_by: &[],
        // Functions, structs, unions and enums have a body, a macro or a constant its value.
        body_paths: &[&["body"], &["value"]],
        // The names Unity and CMocka tests usually take.
        test_files: &[NamePattern::Prefix("test_"), NamePattern::Suffix("_test.c")],
        test_functions: &[NamePattern::Prefix("test_")],
    },
};

/// The C grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_c::LANGUAGE.into()
}
