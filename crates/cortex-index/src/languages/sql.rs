//! SQL and its common dialects: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};

/// How files of SQL and its common dialects are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Sql,
    extensions: &["sql"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["sql", "plpgsql", "plsql", "tsql"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar ships none.
        queries: &[include_str!("../../queries/sql-tags.scm")],
        // A script imports nothing the index reads.
        imports: "",
        qualified_by: &[],
        // Statements have no body field; their patterns capture where the header ends.
        closed_scopes: &[],
        body_paths: &[],
        // Tests in SQL take no form the index can tell.
        test_files: &[],
        test_functions: &[],
    },
};

/// The SQL grammar from tree-sitter-sequel.
fn grammar() -> tree_sitter::Language {
    tree_sitter_sequel::LANGUAGE.into()
}
