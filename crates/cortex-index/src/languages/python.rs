//! Python: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How Python files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Python,
    extensions: &["py", "pyi", "pyw"],
    file_names: &[],
    interpreters: &["python", "pypy"],
    linguist_names: &["python", "py", "python3"],
    tags: TagsSpec {
        grammar,
        // Cortex's patterns first: they read a decorated definition with its decorators, where
        // the grammar's own would read it without them.
        queries: &[
            include_str!("../../queries/python-tags.scm"),
            tree_sitter_python::TAGS_QUERY,
        ],
        imports: include_str!("../../queries/python-imports.scm"),
        // Functions and classes have a body, and so does a decorated one, inside it. An
        // assignment's value and a type alias's type change without changing what the name is.
        body_paths: &[&["body"], &["right"], &["definition", "body"]],
        // pytest's default file names.
        test_files: &[
            NamePattern::Prefix("test_"),
            NamePattern::Suffix("_test.py"),
        ],
        // pytest's default function names, which unittest's test methods follow too.
        test_functions: &[NamePattern::Prefix("test")],
    },
};

/// The Python grammar.
pub(crate) fn grammar() -> tree_sitter::Language {
    tree_sitter_python::LANGUAGE.into()
}
