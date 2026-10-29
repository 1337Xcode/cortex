//! Java: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Java files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Java,
    extensions: &["java"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["java"],
};
