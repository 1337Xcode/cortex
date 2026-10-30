//! TOML: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};

/// How TOML files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Toml,
    extensions: &["toml"],
    file_names: &["Pipfile"],
    interpreters: &[],
    linguist_names: &["toml"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/toml-tags.scm")],
        // A TOML file imports nothing.
        imports: "",
        // Keys and tables have no body field; their patterns capture where the header ends.
        body_paths: &[],
        // A TOML file holds no tests.
        test_files: &[],
        test_functions: &[],
    },
};

/// The TOML grammar from tree-sitter-grammars, the maintained one.
fn grammar() -> tree_sitter::Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}
