//! What the unit tests share: rules for the Python grammar with queries of their own, parsing,
//! the tags those queries find, and the paths and imports import resolution reads.

use std::error::Error;
use std::time::Duration;

use cortex_core::{Identifier, IdentifierError, Language, RepoPath};
use tree_sitter::{Parser, QueryCursor, Tree};

use crate::deadline::Deadline;
use crate::grammar::Grammar;
use crate::imports::Targets;
use crate::languages::TagsSpec;
pub(crate) use crate::languages::python_grammar as python;
use crate::specifiers::{Import, Imported};
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

/// The repository path `text` spells.
///
/// # Panics
///
/// When `text` spells no repository path: a test wrote a path wrong.
#[allow(
    clippy::expect_used,
    reason = "a test's own literal path; one that is not a path is a mistake in the test, which \
              must fail rather than stand for another path"
)]
pub(crate) fn path(text: &str) -> RepoPath {
    RepoPath::new(text).expect("a test path is a repository path")
}

/// An import of the module `specifier` that takes what `imported` says.
pub(crate) fn import(specifier: &str, imported: Imported) -> Result<Import, IdentifierError> {
    Ok(Import {
        specifier: Identifier::new(specifier)?,
        imported,
        alias: None,
        line: 1,
    })
}

/// An import of `name` from the module `specifier`, as Python's `from <specifier> import <name>`
/// makes.
pub(crate) fn import_name(specifier: &str, name: &str) -> Result<Import, IdentifierError> {
    import(specifier, Imported::Name(Identifier::new(name)?))
}

/// The files an import names, as text: the module's, then the submodule.
pub(crate) type Named = (Vec<String>, Option<String>);

/// The files `targets` names, as text.
pub(crate) fn named_by(targets: &Targets) -> Named {
    let module = targets.module.iter().map(|at| at.as_str().to_owned());
    let submodule = targets.submodule.as_ref().map(|at| at.as_str().to_owned());
    (module.collect(), submodule)
}

/// The files an import is expected to name: the module's, then the submodule.
pub(crate) fn named(module: &[&str], submodule: Option<&str>) -> Named {
    let module = module.iter().map(|at| (*at).to_owned()).collect();
    (module, submodule.map(str::to_owned))
}
