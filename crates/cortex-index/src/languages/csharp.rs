//! C#: how a file is recognised as this language.

use cortex_core::Language;

use super::Spec;

/// How C# files are recognised.
pub const SPEC: Spec = Spec {
    language: Language::CSharp,
    extensions: &["cs", "csx"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["c#", "csharp", "cs"],
};
