//! Bash and the POSIX shells close to it: how a file is recognised as this language, and how it is
//! parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How files of Bash and the POSIX shells close to it are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Bash,
    extensions: &["sh", "bash", "zsh", "ksh"],
    file_names: &[
        ".bashrc",
        ".bash_profile",
        ".bash_logout",
        ".profile",
        ".zshrc",
    ],
    interpreters: &["sh", "bash", "dash", "ash", "ksh", "mksh", "zsh"],
    linguist_names: &["shell", "bash", "sh", "shell-script", "zsh"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/bash-tags.scm")],
        imports: include_str!("../../queries/bash-imports.scm"),
        // A function has a body. A constant spans its assignment, which has none.
        body_paths: &[&["body"]],
        // The names shell test scripts usually take.
        test_files: &[
            NamePattern::Suffix("_test.sh"),
            NamePattern::Suffix("_test.bash"),
            NamePattern::Suffix("_test.zsh"),
            NamePattern::Suffix("_test.ksh"),
            NamePattern::Prefix("test_"),
        ],
        // shUnit2 runs the functions named `test...`.
        test_functions: &[NamePattern::Prefix("test")],
    },
};

/// The Bash grammar, which reads the other POSIX shells' scripts as far as they agree with Bash.
fn grammar() -> tree_sitter::Language {
    tree_sitter_bash::LANGUAGE.into()
}
