//! PHP: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How PHP files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Php,
    extensions: &["php", "phtml"],
    file_names: &[],
    interpreters: &["php"],
    linguist_names: &["php"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar's reads properties with a kind outside the format.
        queries: &[include_str!("../../queries/php-tags.scm")],
        imports: include_str!("../../queries/php-imports.scm"),
        // Classes, functions, methods and namespaces have a body. A constant's value has no field
        // of its own, so a constant is all header.
        body_paths: &[&["body"]],
        // PHPUnit's file names and test method names.
        test_files: &[NamePattern::Suffix("Test.php")],
        test_functions: &[NamePattern::Prefix("test")],
    },
};

/// The PHP grammar, which reads the HTML around PHP code too.
fn grammar() -> tree_sitter::Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}
