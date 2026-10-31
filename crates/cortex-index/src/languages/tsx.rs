//! TypeScript with JSX: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::typescript::{BODY_PATHS, IMPORTS};
use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How TypeScript with JSX files are recognised and parsed: as TypeScript, plus the components
/// JSX uses.
pub const SPEC: Spec = Spec {
    language: Language::Tsx,
    extensions: &["tsx"],
    file_names: &[],
    interpreters: &[],
    linguist_names: &["tsx"],
    tags: TagsSpec {
        grammar,
        queries: &[
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
            include_str!("../../queries/ecmascript-tags.scm"),
            include_str!("../../queries/typescript-tags.scm"),
            include_str!("../../queries/jsx-tags.scm"),
        ],
        imports: IMPORTS,
        // `api.fetch()` and the type `geo.Point`: a name reached through a value or a
        // namespace.
        qualified_by: &[
            ("member_expression", "object"),
            ("nested_type_identifier", "module"),
        ],
        closed_scopes: &[],
        body_paths: BODY_PATHS,
        // The names Jest, Vitest and Mocha look for.
        test_files: &[
            NamePattern::Suffix(".test.tsx"),
            NamePattern::Suffix(".spec.tsx"),
        ],
        // Tests are calls such as `it("ships", ...)`, not named functions.
        test_functions: &[],
    },
};

/// The TypeScript grammar with JSX.
fn grammar() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
