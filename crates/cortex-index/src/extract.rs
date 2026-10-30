//! Reading one file: parsing it with its language's grammar, then reading its definitions, the
//! names it uses and the modules it imports, all within a time limit. A file that runs out of time
//! is reported as such and read no further; it never holds up the files after it.

use std::sync::Arc;
use std::time::Duration;

use cortex_core::{Language, RepoPath};
use cortex_store::{RefRecord, SymbolRecord};
use thiserror::Error;
use tree_sitter::{ParseOptions, ParseState, Parser, QueryCursor, Tree};

use crate::deadline::Deadline;
use crate::grammar::Grammars;
use crate::path_rules::is_test_file;
use crate::references::build_references;
use crate::specifiers::{Import, read_imports};
use crate::symbols::build_symbols;
use crate::tags::{Tag, read_tags};

/// What one file defines, uses and imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// Its definitions, in the order of their starts, each after the one it sits in.
    pub symbols: Vec<SymbolRecord>,
    /// The names it uses, in the order they appear, each name an import takes among them as a
    /// reference of kind `import`.
    pub refs: Vec<RefRecord>,
    /// The modules it imports, in the order they appear.
    pub imports: Vec<Import>,
    /// What its queries found that was left out.
    pub left_out: LeftOut,
}

/// How many of the names a file's queries found were left out: because a syntax error touches
/// them, or because they are not names Cortex can store (one line of at most 256 bytes, the
/// qualified name of a definition included). A definition whose name cannot be stored takes the
/// ones inside it along, and each counts; one whose name a syntax error touches is left out alone,
/// and the ones inside it are recorded as if it were not there. Names a query ignores on purpose
/// are not counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Definitions.
    pub definitions: u64,
    /// References.
    pub references: u64,
    /// Imports.
    pub imports: u64,
}

impl LeftOut {
    /// Whether nothing was left out.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Why a file was not read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtractError {
    /// The parser refused the language's grammar.
    #[error("the {} grammar cannot be used: {message}", .language.name())]
    Grammar {
        /// The language.
        language: Language,
        /// tree-sitter's description of the problem.
        message: String,
    },
    /// Parsing the file and reading it took longer than allowed.
    #[error("reading {path} took longer than allowed")]
    TimedOut {
        /// The file.
        path: RepoPath,
    },
}

/// Parses files and reads what they define, use and import. It keeps a parser, so each thread
/// that reads files needs its own.
pub struct Extractor {
    /// Every language's compiled rules.
    grammars: Arc<Grammars>,
    /// The parser, reused from file to file.
    parser: Parser,
    /// The query cursor, reused from query to query.
    cursor: QueryCursor,
    /// How long one file may take.
    timeout: Duration,
}

impl Extractor {
    /// An extractor that reads files with `grammars`, allowing each one `timeout` (the
    /// `index.parse_timeout_ms` setting).
    #[must_use]
    pub fn new(grammars: Arc<Grammars>, timeout: Duration) -> Self {
        Self {
            grammars,
            parser: Parser::new(),
            cursor: QueryCursor::new(),
            timeout,
        }
    }

    /// What the file at `path`, written in `language`, defines, uses and imports, read from
    /// `source`, its contents. Syntax errors do not stop the reading: the definitions and
    /// references outside them are still found.
    ///
    /// # Errors
    ///
    /// [`ExtractError::TimedOut`] when the file takes longer than allowed, and
    /// [`ExtractError::Grammar`] if the parser refuses the language's grammar.
    pub fn extract(
        &mut self,
        path: &RepoPath,
        language: Language,
        source: &[u8],
    ) -> Result<Extraction, ExtractError> {
        let grammar = self.grammars.get(language);
        self.parser
            .set_language(&grammar.tree_sitter)
            .map_err(|error| ExtractError::Grammar {
                language,
                message: error.to_string(),
            })?;
        let timed_out = || ExtractError::TimedOut { path: path.clone() };
        let deadline = Deadline::after(self.timeout);
        let tree = parse(&mut self.parser, source, &deadline).ok_or_else(timed_out)?;
        let root = tree.root_node();
        let read =
            read_tags(&mut self.cursor, grammar, root, source, &deadline).ok_or_else(timed_out)?;
        let imported = read_imports(&mut self.cursor, grammar, root, source, &deadline)
            .ok_or_else(timed_out)?;
        let definitions = read
            .tags
            .iter()
            .filter(|tag| tag.role.is_definition())
            .count();
        let references = read.tags.len() - definitions + imported.names.len();
        let tags = with_imported_names(read.tags, imported.names);
        let symbols = build_symbols(
            &tags,
            source,
            grammar.spec,
            is_test_file(path, grammar.spec.test_files),
        );
        let refs = build_references(&tags, source, &symbols);
        let left_out = LeftOut {
            definitions: read.broken_definitions + unrecorded(definitions, symbols.len()),
            references: read.broken_references + unrecorded(references, refs.len()),
            imports: imported.left_out,
        };
        Ok(Extraction {
            symbols,
            refs,
            imports: imported.imports,
            left_out,
        })
    }
}

/// `tags`, in the order of their names, with `names`, the references to the names the file's
/// imports take, put among them in that order.
fn with_imported_names<'tree>(
    mut tags: Vec<Tag<'tree>>,
    names: Vec<Tag<'tree>>,
) -> Vec<Tag<'tree>> {
    tags.extend(names);
    tags.sort_by_key(|tag| (tag.name.start_byte(), tag.name.end_byte()));
    tags
}

/// How many of the `found` tags of one role made no record, `recorded` having been made.
fn unrecorded(found: usize, recorded: usize) -> u64 {
    u64::try_from(found.saturating_sub(recorded)).unwrap_or(u64::MAX)
}

/// The tree of `source`, or `None` when the deadline passed first.
fn parse(parser: &mut Parser, source: &[u8], deadline: &Deadline) -> Option<Tree> {
    // A parse the deadline stopped would otherwise resume on the next call, with another file.
    parser.reset();
    let mut check = |_: &ParseState| deadline.check();
    let options = ParseOptions::new().progress_callback(&mut check);
    // The parser has a language, so it returns no tree only when the progress check stopped it.
    parser.parse_with_options(
        &mut |offset, _| source.get(offset..).unwrap_or_default(),
        None,
        Some(options),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::test_support::{distant_deadline, many_functions, parse_python, python};

    /// Once the deadline has passed, parsing stops and gives no tree; with time it gives one.
    #[test]
    fn parsing_stops_at_the_deadline() -> Result<(), tree_sitter::LanguageError> {
        let source = many_functions(5000);
        let mut parser = Parser::new();
        parser.set_language(&python())?;
        let stopped = parse(
            &mut parser,
            source.as_bytes(),
            &Deadline::after(Duration::ZERO),
        );
        assert!(stopped.is_none());
        let tree = parse(
            &mut parser,
            source.as_bytes(),
            &Deadline::after(Duration::from_secs(60)),
        );
        assert_eq!(
            tree.map(|tree| tree.root_node().named_child_count()),
            Some(5000)
        );
        Ok(())
    }

    /// A parse the deadline stopped leaves nothing behind: the next file parsed with the same
    /// parser gets the tree a new parser would give it.
    #[test]
    fn a_stopped_parse_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
        let mut parser = Parser::new();
        parser.set_language(&python())?;
        let big = many_functions(5000);
        let stopped = parse(
            &mut parser,
            big.as_bytes(),
            &Deadline::after(Duration::ZERO),
        );
        assert!(stopped.is_none());
        let small = "class C:\n    def m(self):\n        return 1\n";
        let reused = parse(&mut parser, small.as_bytes(), &distant_deadline())
            .map(|tree| tree.root_node().to_sexp());
        assert_eq!(reused, Some(parse_python(small)?.root_node().to_sexp()));
        Ok(())
    }
}
