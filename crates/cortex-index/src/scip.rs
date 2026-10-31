//! Ties from a SCIP index: `index.scip` at the repository's root, written by the user's or CI's own
//! indexer (Cortex runs none). A reference occurrence whose symbol has exactly one definition
//! occurrence ties the stored reference with the symbol's name on the occurrence's line to the
//! stored definition with that name around the definition's line, at full confidence.
//!
//! Lines are all this reads of a range, so a document's position encoding does not matter. A
//! document is left out when its path is not a stored file's or its file changed after the index
//! was written, and with it every symbol it defines. A symbol local to a document is left out too,
//! since the tags record no local definitions, and so is any occurrence the store cannot match
//! one for one: a name defined twice around a line, or one line using one name for two symbols.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use cortex_core::{Identifier, RepoPath};
use cortex_store::{FileId, Queries, RefId, RefRow, StoreError, SymbolId, SymbolRow};
use protobuf::Message;
use scip::symbol::{is_local_symbol, parse_symbol};
use scip::types::{Document, Index, Occurrence};

use crate::safe_read::{ReadError, read_whole};

/// The index file, at the repository's root.
const INDEX_FILE: &str = "index.scip";

/// The largest index read: the index of a large repository runs to tens of megabytes, and reading
/// one holds it whole in memory several times over.
const MAX_INDEX_BYTES: u64 = 256 * 1024 * 1024;

/// The role bit that marks an occurrence as a definition.
const DEFINITION_ROLE: i32 = 0x1;

/// What became of a repository's SCIP index in a full index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScipOutcome {
    /// The repository has none.
    Absent,
    /// It was read.
    Read {
        /// The documents it holds.
        documents: u64,
        /// The documents left out: not a stored file's, or changed since the index was written.
        left_out: u64,
        /// The stored references it ties.
        tied: u64,
    },
    /// It could not be used.
    Refused(ScipProblem),
}

/// Why a repository's SCIP index could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScipProblem {
    /// It is a link, a folder or a special file.
    NotAFile,
    /// It holds more than the largest index read.
    TooLarge,
    /// It could not be read.
    Unreadable,
    /// It is not a SCIP index.
    NotAnIndex,
}

/// What the SCIP index of the repository at `root` says, read against the store through
/// `queries`: the outcome, and the stored references it ties, each to its stored definition.
///
/// # Errors
///
/// Store errors.
pub(crate) fn read_index(
    root: &Path,
    queries: Queries<'_>,
) -> Result<(ScipOutcome, BTreeMap<RefId, SymbolId>), StoreError> {
    let (index, written_ns) = match load(&root.join(INDEX_FILE)) {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return Ok((ScipOutcome::Absent, BTreeMap::new())),
        Err(problem) => return Ok((ScipOutcome::Refused(problem), BTreeMap::new())),
    };
    let mut store = Store::new(queries);
    let mut documents: Vec<(&Document, FileId)> = Vec::new();
    for document in &index.documents {
        if let Some(file) = store.current_file(&document.relative_path, written_ns)? {
            documents.push((document, file));
        }
    }
    let definitions = store.definitions(&index.documents, &documents)?;
    let ties = store.ties(&documents, &definitions)?;
    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
    let outcome = ScipOutcome::Read {
        documents: count(index.documents.len()),
        left_out: count(index.documents.len() - documents.len()),
        tied: count(ties.len()),
    };
    Ok((outcome, ties))
}

/// The index at `path` and when it was written, in nanoseconds since the epoch; `None` when there
/// is none.
fn load(path: &Path) -> Result<Option<(Index, i64)>, ScipProblem> {
    let bytes = match read_whole(path, MAX_INDEX_BYTES) {
        Ok(bytes) => bytes,
        Err(ReadError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(ReadError::NotRegular) => return Err(ScipProblem::NotAFile),
        Err(ReadError::TooLarge) => return Err(ScipProblem::TooLarge),
        Err(ReadError::Io(_)) => return Err(ScipProblem::Unreadable),
    };
    let written_ns = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .ok_or(ScipProblem::Unreadable)?;
    let index = Index::parse_from_bytes(&bytes).map_err(|_| ScipProblem::NotAnIndex)?;
    Ok(Some((index, written_ns)))
}

/// The store as the index is matched against it, each file's rows read once.
struct Store<'q> {
    /// The read queries.
    queries: Queries<'q>,
    /// Each stored file's definitions, once read.
    symbols: BTreeMap<FileId, Vec<SymbolRow>>,
    /// Each stored file's references, once read.
    refs: BTreeMap<FileId, Vec<RefRow>>,
    /// The name each symbol of the index is known by, once parsed: its last descriptor's.
    names: BTreeMap<String, Option<Identifier>>,
}

impl<'q> Store<'q> {
    /// The store read through `queries`.
    fn new(queries: Queries<'q>) -> Self {
        Self {
            queries,
            symbols: BTreeMap::new(),
            refs: BTreeMap::new(),
            names: BTreeMap::new(),
        }
    }

    /// The stored file at `relative_path`, unless it changed after `written_ns`, when the index
    /// was written.
    fn current_file(
        &self,
        relative_path: &str,
        written_ns: i64,
    ) -> Result<Option<FileId>, StoreError> {
        let Ok(path) = RepoPath::new(relative_path) else {
            return Ok(None);
        };
        let row = self.queries.file(&path)?;
        Ok(row
            .filter(|row| row.file.mtime_ns <= written_ns)
            .map(|row| row.id))
    }

    /// The stored definition of each global symbol that `all`, every document of the index, defines
    /// exactly once, in one of `current`, the documents whose files are as indexed.
    fn definitions(
        &mut self,
        all: &[Document],
        current: &[(&Document, FileId)],
    ) -> Result<BTreeMap<String, SymbolId>, StoreError> {
        let mut defined: BTreeMap<&str, u32> = BTreeMap::new();
        for occurrence in all.iter().flat_map(|document| &document.occurrences) {
            if is_definition(occurrence) && !is_local_symbol(&occurrence.symbol) {
                *defined.entry(occurrence.symbol.as_str()).or_insert(0) += 1;
            }
        }
        let mut definitions = BTreeMap::new();
        for (document, file) in current {
            for occurrence in &document.occurrences {
                let symbol = occurrence.symbol.as_str();
                if !is_definition(occurrence) || defined.get(symbol) != Some(&1) {
                    continue;
                }
                let (Some(line), Some(name)) = (line_of(occurrence), self.name_of(symbol)) else {
                    continue;
                };
                if let Some(id) = self.definition_at(*file, &name, line)? {
                    definitions.insert(symbol.to_owned(), id);
                }
            }
        }
        Ok(definitions)
    }

    /// The stored references the reference occurrences of `documents` tie to `definitions`, each
    /// once: a reference two occurrences tie to two definitions is left out.
    fn ties(
        &mut self,
        documents: &[(&Document, FileId)],
        definitions: &BTreeMap<String, SymbolId>,
    ) -> Result<BTreeMap<RefId, SymbolId>, StoreError> {
        let mut ties: BTreeMap<RefId, Option<SymbolId>> = BTreeMap::new();
        for (document, file) in documents {
            for occurrence in &document.occurrences {
                if is_definition(occurrence) {
                    continue;
                }
                let Some(&target) = definitions.get(&occurrence.symbol) else {
                    continue;
                };
                let (Some(line), Some(name)) =
                    (line_of(occurrence), self.name_of(&occurrence.symbol))
                else {
                    continue;
                };
                for reference in self.references_at(*file, &name, line)? {
                    let tie = ties.entry(reference).or_insert(Some(target));
                    if *tie != Some(target) {
                        *tie = None;
                    }
                }
            }
        }
        Ok(ties
            .into_iter()
            .filter_map(|(reference, target)| Some((reference, target?)))
            .collect())
    }

    /// The name the index's `symbol` is known by: its last descriptor's, when that is a name.
    fn name_of(&mut self, symbol: &str) -> Option<Identifier> {
        self.names
            .entry(symbol.to_owned())
            .or_insert_with(|| {
                let parsed = parse_symbol(symbol).ok()?;
                let last = parsed.descriptors.last()?;
                Identifier::new(&last.name).ok()
            })
            .clone()
    }

    /// The innermost stored definition named `name` in `file` whose lines hold `line`, when there
    /// is exactly one innermost.
    fn definition_at(
        &mut self,
        file: FileId,
        name: &Identifier,
        line: u32,
    ) -> Result<Option<SymbolId>, StoreError> {
        let symbols = match self.symbols.entry(file) {
            std::collections::btree_map::Entry::Occupied(read) => read.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(self.queries.symbols_in_file(file)?)
            }
        };
        let holding: Vec<&SymbolRow> = symbols
            .iter()
            .filter(|symbol| symbol.name == *name)
            .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
            .collect();
        let innermost = holding.iter().map(|symbol| symbol.start_line).max();
        let mut at_innermost = holding
            .iter()
            .filter(|symbol| Some(symbol.start_line) == innermost);
        match (at_innermost.next(), at_innermost.next()) {
            (Some(only), None) => Ok(Some(only.id)),
            _ => Ok(None),
        }
    }

    /// The stored references in `file` that use `name` on `line`.
    fn references_at(
        &mut self,
        file: FileId,
        name: &Identifier,
        line: u32,
    ) -> Result<Vec<RefId>, StoreError> {
        let refs = match self.refs.entry(file) {
            std::collections::btree_map::Entry::Occupied(read) => read.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(self.queries.refs_in_file(file)?)
            }
        };
        Ok(refs
            .iter()
            .filter(|reference| reference.dst_name == *name && reference.line == line)
            .map(|reference| reference.id)
            .collect())
    }
}

/// Whether `occurrence` defines its symbol.
fn is_definition(occurrence: &Occurrence) -> bool {
    occurrence.symbol_roles & DEFINITION_ROLE != 0
}

/// The 1-based line `occurrence` starts on, from whichever form its range takes: a single line, a
/// span of lines, or the packed numbers whose first is the line. `None` for a range of no form or
/// a line past what a line number holds.
fn line_of(occurrence: &Occurrence) -> Option<u32> {
    let zero_based = if occurrence.has_single_line_range() {
        occurrence.single_line_range().line
    } else if occurrence.has_multi_line_range() {
        occurrence.multi_line_range().start_line
    } else if matches!(occurrence.range.len(), 3 | 4) {
        *occurrence.range.first()?
    } else {
        return None;
    };
    u32::try_from(zero_based).ok()?.checked_add(1)
}
