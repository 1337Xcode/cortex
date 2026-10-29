//! Swift: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Swift files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Swift,
    extensions: &["swift"],
    file_names: &[],
    interpreters: &["swift"],
    linguist_names: &["swift"],
};
