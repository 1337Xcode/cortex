//! TOML: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How TOML files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Toml,
    extensions: &["toml"],
    file_names: &["Pipfile"],
    interpreters: &[],
    linguist_names: &["toml"],
    tags: None,
};
