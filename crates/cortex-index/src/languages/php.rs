//! PHP: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How PHP files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Php,
    extensions: &["php", "phtml"],
    file_names: &[],
    interpreters: &["php"],
    linguist_names: &["php"],
};
