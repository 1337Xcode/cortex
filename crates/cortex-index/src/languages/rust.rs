//! Rust: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Rust files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Rust,
    extensions: &["rs"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["rust", "rs"],
};
