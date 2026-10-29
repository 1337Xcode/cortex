//! TypeScript: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How TypeScript files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::TypeScript,
    extensions: &["ts", "mts", "cts"],
    file_names: &[],
    interpreters: &["ts-node", "deno"],
    linguist_names: &["typescript", "ts"],
};
