//! C#: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How C# files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::CSharp,
    extensions: &["cs", "csx"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["c#", "csharp", "cs"],
    tags: Some(TagsSpec {
        grammar,
        // Cortex's own query: the grammar's uses a capture outside the tags format.
        queries: &[include_str!("../../queries/csharp-tags.scm")],
        imports: include_str!("../../queries/csharp-imports.scm"),
        // Types, namespaces, methods and constructors have a body. A constant's value has no
        // field of its own, so a constant is all header.
        body_paths: &[&["body"]],
        // The names test projects usually give their files. No test goes by its name in C#, so
        // these mark only the files, for the links from a test file to what it tests.
        test_files: &[
            NamePattern::Suffix("Tests.cs"),
            NamePattern::Suffix("Test.cs"),
        ],
        // NUnit, xUnit and MSTest mark tests with an attribute, read anywhere; none goes by name.
        test_functions: &[],
    }),
};

/// The C# grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}
