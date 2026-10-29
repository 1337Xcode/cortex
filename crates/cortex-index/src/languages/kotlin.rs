//! Kotlin: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Kotlin files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Kotlin,
    extensions: &["kt", "kts"],
    file_names: &[],
    interpreters: &["kotlin"],
    linguist_names: &["kotlin"],
};
