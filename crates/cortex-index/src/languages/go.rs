//! Go: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Go files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Go,
    extensions: &["go"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["go", "golang"],
};
