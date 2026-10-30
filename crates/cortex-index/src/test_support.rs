//! What the unit tests share: rules for the Python grammar with queries of their own, parsing,
//! and the tags those queries find.

use std::error::Error;
use std::time::Duration;

use cortex_core::Language;
use tree_sitter::{Parser, QueryCursor, Tree};

use crate::deadline::Deadline;
use crate::grammar::Grammar;
use crate::languages::TagsSpec;
pub(crate) use crate::languages::python_grammar as python;
use crate::tags::{Tag, read_tags};

/// An imports query for tests about something else.
pub(crate) const PLAIN_IMPORTS: &str =
    "(import_statement name: (dotted_name) @import.module) @import";

/// Rules for Python files with `tags` and `imports` as the queries, no body paths and no test
/// names.
pub(crate) const fn python_spec(tags: &'static [&'static str], imports: &'static str) -> TagsSpec {
    TagsSpec {
        grammar: python,
        queries: tags,
        imports,
        body_paths: &[],
        test_files: &[],
        test_functions: &[],
    }
}

/// `source` parsed as Python.
pub(crate) fn parse_python(source: &str) -> Result<Tree, Box<dyn Error>> {
    let mut parser = Parser::new();
    parser.set_language(&python())?;
    Ok(parser.parse(source, None).ok_or("no tree")?)
}

/// A deadline no test reaches.
pub(crate) fn distant_deadline() -> Deadline {
    Deadline::after(Duration::from_secs(60))
}

/// What `use_tags` makes of the tags that `spec`, for Python, finds in `source`.
pub(crate) fn with_tags<T>(
    spec: &'static TagsSpec,
    source: &str,
    use_tags: impl FnOnce(&[Tag<'_>]) -> T,
) -> Result<T, Box<dyn Error>> {
    let grammar = Grammar::compile(Language::Python, spec)?;
    let tree = parse_python(source)?;
    let read = read_tags(
        &mut QueryCursor::new(),
        &grammar,
        tree.root_node(),
        source.as_bytes(),
        &distant_deadline(),
    )
    .ok_or("out of time")?;
    Ok(use_tags(&read.tags))
}

/// Python source holding `count` small functions, each with one call: enough work that the query
/// cursor checks its deadline many times.
pub(crate) fn many_functions(count: usize) -> String {
    "def f(x):\n    return g(x) + 1\n".repeat(count)
}
