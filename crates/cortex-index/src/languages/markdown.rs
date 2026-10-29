//! Markdown: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Markdown files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Markdown,
    extensions: &["md", "markdown", "mdown", "mkd", "mkdn"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["markdown", "md"],
    tags: None,
};
