//! JavaScript, JSX included: how a file is recognised as this language, and how it is parsed.

use cortex_core::Language;

use super::{Spec, TagsSpec};
use crate::path_rules::NamePattern;

/// How JavaScript files are recognised and parsed.
pub const SPEC: Spec = Spec {
    language: Language::JavaScript,
    extensions: &["js", "mjs", "cjs", "jsx"],
    file_names: &[],
    interpreters: &["node", "nodejs"],
    linguist_names: &["javascript", "js", "node", "jsx"],
    tags: TagsSpec {
        grammar,
        // The grammar's patterns first: a constant holding a function is read as a function.
        queries: &[
            tree_sitter_javascript::TAGS_QUERY,
            include_str!("../../queries/javascript-tags.scm"),
            include_str!("../../queries/ecmascript-tags.scm"),
            include_str!("../../queries/jsx-tags.scm"),
        ],
        imports: include_str!("../../queries/javascript-imports.scm"),
        // `api.fetch()`: a name reached through a value or a namespace.
        qualified_by: &[("member_expression", "object")],
        // Functions, methods and classes have a body. A declaration, property, field or
        // assignment holding a function has the function's body; one holding another value has
        // the value, as an exported assignment does.
        closed_scopes: &[],
        body_paths: &[
            &["body"],
            &["value", "body"],
            &["right", "body"],
            &["value", "right"],
            &["value"],
        ],
        // The names Jest, Vitest and Mocha look for.
        test_files: &[
            NamePattern::Suffix(".test.js"),
            NamePattern::Suffix(".spec.js"),
            NamePattern::Suffix(".test.jsx"),
            NamePattern::Suffix(".spec.jsx"),
            NamePattern::Suffix(".test.mjs"),
            NamePattern::Suffix(".spec.mjs"),
            NamePattern::Suffix(".test.cjs"),
            NamePattern::Suffix(".spec.cjs"),
        ],
        // Tests are calls such as `it("ships", ...)`, not named functions.
        test_functions: &[],
    },
};

/// The JavaScript grammar, which reads JSX too.
fn grammar() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}
