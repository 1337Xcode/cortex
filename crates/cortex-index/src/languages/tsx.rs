//! TypeScript with JSX: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How TypeScript with JSX files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Tsx,
    extensions: &["tsx"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["tsx"],
};
