//! C++: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How C++ files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::Cpp,
    extensions: &[
        "cc", "cpp", "cxx", "c++", "hh", "hpp", "hxx", "h++", "ipp", "tpp", "inl",
    ],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["c++", "cpp"],
    tags: TagsSpec {
        grammar,
        // Cortex's own query: the grammar's reads no references, gives functions the span of
        // their declarator alone, and uses a capture Cortex reads differently.
        queries: &[include_str!("../../queries/cpp-tags.scm")],
        imports: include_str!("../../queries/cpp-imports.scm"),
        qualified_by: &[],
        // Functions, classes, namespaces and enums have a body; a macro, a constant or a member
        // constant its value.
        closed_scopes: &[],
        body_paths: &[&["body"], &["value"], &["default_value"]],
        // Google Test's names, and the Unity and CMocka names C uses.
        test_files: &[
            NamePattern::Suffix("_test.cc"),
            NamePattern::Suffix("_test.cpp"),
            NamePattern::Suffix("_test.cxx"),
            NamePattern::Suffix("_tests.cc"),
            NamePattern::Suffix("_tests.cpp"),
            NamePattern::Suffix("_unittest.cc"),
            NamePattern::Suffix("_unittest.cpp"),
            NamePattern::Prefix("test_"),
        ],
        // Google Test's tests are marked by their macro; other frameworks' are named `test_`.
        test_functions: &[NamePattern::Prefix("test_")],
    },
};

/// The C++ grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_cpp::LANGUAGE.into()
}
