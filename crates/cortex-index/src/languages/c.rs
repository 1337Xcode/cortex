//! C; a `.h` header counts as C unless its start reads as C++: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How C files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::C,
    extensions: &["c", "h"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["c"],
};
