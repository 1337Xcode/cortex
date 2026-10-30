//! JSON, with comments included: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};

/// How JSON files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Json,
    extensions: &["json", "jsonc"],
    file_names: &[".babelrc", ".eslintrc", ".jshintrc"],
    interpreters: &[],
    linguist_names: &["json", "jsonc", "json-with-comments"],
    tags: Some(TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/json-tags.scm")],
        // A JSON file imports nothing.
        imports: "",
        // A key's value is its body, so its header is the key.
        body_paths: &[&["value"]],
        // A JSON file holds no tests.
        test_files: &[],
        test_functions: &[],
    }),
};

/// The JSON grammar, which reads comments too.
fn grammar() -> tree_sitter::Language {
    tree_sitter_json::LANGUAGE.into()
}
