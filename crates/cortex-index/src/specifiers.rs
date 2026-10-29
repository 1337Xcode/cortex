//! The modules a file imports, read with its language's imports query: for each, the module as
//! written, what the file takes from it, and the name it binds when it renames it.

use cortex_core::Identifier;
use tree_sitter::{Node, QueryCapture, QueryCursor, QueryCursorOptions, StreamingIterator};

use crate::deadline::Deadline;
use crate::grammar::{Grammar, ImportCapture};
use crate::lines::line_number;
use crate::name_text::name_text;

/// What an import takes from its module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Imported {
    /// The module itself, as Python's `import os.path` does.
    Module,
    /// One name the module defines, as `from os import path` does.
    Name(Identifier),
    /// Every public name of the module, as `from os import *` does.
    All,
    /// The module's default export, as JavaScript's `import React from "react"` does.
    Default,
}

/// One module a file imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The module as the file writes it, such as `os.path` or `..models`.
    pub specifier: Identifier,
    /// What the file takes from it.
    pub imported: Imported,
    /// The name the import is bound to, when the file renames it.
    pub alias: Option<Identifier>,
    /// The 1-based line where the import statement starts.
    pub line: u32,
}

/// The captures of one match of an imports query.
#[derive(Debug, Default)]
struct Parts<'tree> {
    /// The statement.
    statement: Option<Node<'tree>>,
    /// The module.
    module: Option<Node<'tree>>,
    /// The name taken.
    name: Option<Node<'tree>>,
    /// The name bound instead.
    alias: Option<Node<'tree>>,
    /// The mark that every name is taken.
    all: Option<Node<'tree>>,
    /// The mark that the default export is taken.
    default: Option<Node<'tree>>,
}

/// The imports in the tree under `root`, in the order the query finds them, which follows the
/// source; `None` when the deadline passed before the query finished.
pub(crate) fn read_imports(
    cursor: &mut QueryCursor,
    grammar: &Grammar,
    root: Node<'_>,
    source: &[u8],
    deadline: &Deadline,
) -> Option<Vec<Import>> {
    let mut imports = Vec::new();
    let mut check = |_: &tree_sitter::QueryCursorState| deadline.check();
    let options = QueryCursorOptions::new().progress_callback(&mut check);
    let mut matches = cursor.matches_with_options(&grammar.imports, root, source, options);
    while let Some(found) = matches.next() {
        let parts = parts_of(found.captures(), &grammar.import_captures);
        imports.extend(import_of(&parts, source));
    }
    if deadline.has_passed() {
        return None;
    }
    Some(imports)
}

/// The captures of one match, by what each marks.
fn parts_of<'tree>(captures: &[QueryCapture<'tree>], meanings: &[ImportCapture]) -> Parts<'tree> {
    let mut parts = Parts::default();
    for capture in captures {
        let meaning = usize::try_from(capture.index)
            .ok()
            .and_then(|index| meanings.get(index));
        let slot = match meaning {
            Some(ImportCapture::Statement) => &mut parts.statement,
            Some(ImportCapture::Module) => &mut parts.module,
            Some(ImportCapture::Name) => &mut parts.name,
            Some(ImportCapture::Alias) => &mut parts.alias,
            Some(ImportCapture::All) => &mut parts.all,
            Some(ImportCapture::Default) => &mut parts.default,
            Some(ImportCapture::Helper) | None => continue,
        };
        *slot = Some(capture.node);
    }
    parts
}

/// The import one match describes, unless a part of it is missing, holds a syntax error, or is
/// not a name Cortex can store.
fn import_of(parts: &Parts<'_>, source: &[u8]) -> Option<Import> {
    let specifier = identifier_at(parts.module?, source)?;
    let imported = match (parts.name, parts.all, parts.default) {
        (Some(name), _, _) => Imported::Name(identifier_at(name, source)?),
        (None, Some(_), _) => Imported::All,
        (None, None, Some(_)) => Imported::Default,
        (None, None, None) => Imported::Module,
    };
    let alias = match parts.alias {
        Some(alias) => Some(identifier_at(alias, source)?),
        None => None,
    };
    Some(Import {
        specifier,
        imported,
        alias,
        line: line_number(parts.statement?.start_position().row)?,
    })
}

/// The text of `node` as a name, unless it holds a syntax error or cannot be one.
fn identifier_at(node: Node<'_>, source: &[u8]) -> Option<Identifier> {
    if node.has_error() {
        return None;
    }
    Identifier::new(&name_text(node, source)?).ok()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use cortex_core::Language;

    use super::*;
    use crate::languages::TagsSpec;
    use crate::test_support::{parse_python, python_spec};

    /// Once the deadline has passed, the query stops and reports it instead of a partial list.
    #[test]
    fn imports_stop_at_the_deadline() -> Result<(), Box<dyn Error>> {
        static IMPORTS: TagsSpec = python_spec(
            &["(function_definition name: (identifier) @name) @definition.function"],
            "(import_statement name: (dotted_name) @import.module) @import",
        );
        let grammar = Grammar::compile(Language::Python, &IMPORTS)?;
        let source = "import os\n".repeat(5000);
        let tree = parse_python(&source)?;
        let read = |allowance| {
            read_imports(
                &mut QueryCursor::new(),
                &grammar,
                tree.root_node(),
                source.as_bytes(),
                &Deadline::after(allowance),
            )
            .map(|imports| imports.len())
        };
        assert_eq!(read(Duration::ZERO), None);
        assert_eq!(read(Duration::from_secs(60)), Some(5000));
        Ok(())
    }
}
