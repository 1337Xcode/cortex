//! Markdown: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};

/// How Markdown files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Markdown,
    extensions: &["md", "markdown", "mdown", "mkd", "mkdn"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["markdown", "md"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/markdown-tags.scm")],
        // A document imports nothing.
        imports: "",
        qualified_by: &[],
        // A heading has no body: it is all header.
        body_paths: &[],
        // A document holds no tests.
        test_files: &[],
        test_functions: &[],
    },
};

/// The Markdown block grammar, which reads headings and sections and leaves the text inside a
/// block whole.
fn grammar() -> tree_sitter::Language {
    tree_sitter_md::LANGUAGE.into()
}
