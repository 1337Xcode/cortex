//! SQL and its common dialects: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How SQL and its common dialects files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::Sql,
    extensions: &["sql"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["sql", "plpgsql", "plsql", "tsql"],
};
