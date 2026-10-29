//! YAML: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How YAML files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Yaml,
    extensions: &["yaml", "yml"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["yaml", "yml"],
};
