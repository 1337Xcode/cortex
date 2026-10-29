//! Python: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Python files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Python,
    extensions: &["py", "pyi", "pyw"],
    file_names: &[],
    interpreters: &["python", "pypy"],
    linguist_names: &["python", "py", "python3"],
};
