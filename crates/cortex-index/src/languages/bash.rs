//! Bash and the POSIX shells close to it: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How Bash and the POSIX shells close to it files are recognised.
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
};
