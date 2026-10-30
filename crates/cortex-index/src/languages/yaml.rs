//! YAML: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};

/// How YAML files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Yaml,
    extensions: &["yaml", "yml"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["yaml", "yml"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/yaml-tags.scm")],
        // A YAML file imports nothing.
        imports: "",
        // A key's value is its body, so its header is the key.
        body_paths: &[&["value"]],
        // A YAML file holds no tests.
        test_files: &[],
        test_functions: &[],
    },
};

/// The YAML grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_yaml::LANGUAGE.into()
}
