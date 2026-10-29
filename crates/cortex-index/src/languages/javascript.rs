//! JavaScript, JSX included: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How JavaScript files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::JavaScript,
    extensions: &["js", "mjs", "cjs", "jsx"],
    file_names: &[],
    interpreters: &["node", "nodejs"],
    linguist_names: &["javascript", "js", "node", "jsx"],
};
