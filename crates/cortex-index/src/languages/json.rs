//! JSON, with comments included: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How JSON files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Json,
    extensions: &["json", "jsonc"],
    file_names: &[".babelrc", ".eslintrc", ".jshintrc"],
    interpreters: &[],
    linguist_names: &["json", "jsonc", "json-with-comments"],
};
