//! TypeScript: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How TypeScript files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::TypeScript,
    extensions: &["ts", "mts", "cts"],
    file_names: &[],
    interpreters: &["ts-node", "deno"],
    linguist_names: &["typescript", "ts"],
    tags: TagsSpec {
        grammar,
        queries: QUERIES,
        imports: IMPORTS,
        // `api.fetch()` and the type `geo.Point`: a name reached through a value or a
        // namespace.
        qualified_by: &[
            ("member_expression", "object"),
            ("nested_type_identifier", "module"),
        ],
        body_paths: BODY_PATHS,
        // The names Jest, Vitest and Mocha look for.
        test_files: &[
            NamePattern::Suffix(".test.ts"),
            NamePattern::Suffix(".spec.ts"),
            NamePattern::Suffix(".test.mts"),
            NamePattern::Suffix(".spec.mts"),
            NamePattern::Suffix(".test.cts"),
            NamePattern::Suffix(".spec.cts"),
        ],
        // Tests are calls such as `it("ships", ...)`, not named functions.
        test_functions: &[],
    },
};

/// The tags queries. TypeScript's grammar extends JavaScript's and its own query holds only what
/// TypeScript adds, so the JavaScript query comes first; Cortex's additions follow, so the
/// grammars' patterns win where both find a name. TSX reads the same, plus the components JSX
/// uses.
const QUERIES: &[&str] = &[
    tree_sitter_javascript::TAGS_QUERY,
    tree_sitter_typescript::TAGS_QUERY,
    include_str!("../../queries/ecmascript-tags.scm"),
    include_str!("../../queries/typescript-tags.scm"),
];

/// The imports query TypeScript and TSX share: JavaScript's forms, then TypeScript's own.
pub(super) const IMPORTS: &str = concat!(
    include_str!("../../queries/javascript-imports.scm"),
    include_str!("../../queries/typescript-imports.scm"),
);

/// Where a definition's body is, in TypeScript and TSX: as in JavaScript, a declaration,
/// property, field or assignment holding a function has the function's body, one holding another
/// value has the value; a type alias's body is its type.
pub(super) const BODY_PATHS: &[&[&str]] = &[
    &["body"],
    &["value", "body"],
    &["right", "body"],
    &["value", "right"],
    &["value"],
];

/// The TypeScript grammar.
fn grammar() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
