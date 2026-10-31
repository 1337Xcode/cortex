//! Ties from a SCIP index: `index.scip` at the repository's root, written by the user's or CI's own
//! indexer (Cortex runs none). A symbol the index defines exactly once, as a namespace, type, term
//! or method, maps to the stored definition with its name around the definition's line. On each
//! line of a document, the stored references of one name are tied to that definition, at full
//! confidence, when the index has as many reference occurrences of that name there and every one
//! of them is that symbol: a line using one name for two things, one of them unknown to the index
//! or not a definition Cortex stores (a parameter, a local), ties nothing.
//!
//! Lines are all this reads of a range, so a document's position encoding does not matter. A
//! document is left out when its path is not a stored file's or its file changed after the index
//! was written, and with it every symbol it defines.
//!
//! The index is untrusted input. It is read through the safe reader, one document and one
//! occurrence at a time into small records, with caps on its size, its occurrences, its distinct
//! symbols and a symbol's length; past a cap it is refused. Symbol names are read by a scan of
//! their own, linear in the symbol's length.

mod symbols;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use cortex_core::{Identifier, RepoPath};
use cortex_store::{FileId, Queries, RefId, StoreError, SymbolId, SymbolRow};
use protobuf::CodedInputStream;
use protobuf::rt::skip_field_for_tag;
use scip::types::Occurrence;

use crate::safe_read::{ReadError, read_whole};

/// The index file, at the repository's root.
pub(crate) const INDEX_FILE: &str = "index.scip";

/// The largest index read. An index of a large repository runs to tens of megabytes.
const MAX_INDEX_BYTES: u64 = 256 * 1024 * 1024;

/// The most occurrences read, about 60 MB of records: a repository of a million lines holds a few
/// million (an estimate, not measured).
const MAX_OCCURRENCES: usize = 5_000_000;

/// The most distinct symbols read.
const MAX_SYMBOLS: usize = 1_000_000;

/// The longest symbol read; an occurrence of a longer one is left out. Real symbols run to a few
/// dozen bytes.
const MAX_SYMBOL_BYTES: usize = 512;

/// The field of an index that holds its documents.
const INDEX_DOCUMENTS: u32 = 2;

/// The field of a document that holds its path.
const DOCUMENT_PATH: u32 = 1;

/// The field of a document that holds its occurrences.
const DOCUMENT_OCCURRENCES: u32 = 2;

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
    /// It was not read again: an index of chosen paths keeps the ties it gave before.
    NotRead,
}

/// Why a repository's SCIP index could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScipProblem {
    /// It is a link, a folder or a special file.
    NotAFile,
    /// It is larger than read: in bytes, occurrences or distinct symbols.
    TooLarge,
    /// It could not be read.
    Unreadable,
    /// It is not a SCIP index.
    NotAnIndex,
}

/// One occurrence as matching reads it.
#[derive(Debug, Clone, Copy)]
struct Seen {
    /// The 1-based line it starts on.
    line: u32,
    /// Its symbol, by position among the index's symbols.
    symbol: usize,
    /// Whether it defines the symbol.
    is_definition: bool,
}

/// One document as matching reads it.
struct Indexed {
    /// Its path as written, relative to the repository's root.
    path: String,
    /// Its occurrences with a line and a symbol of a readable length.
    seen: Vec<Seen>,
}

/// The index as matching reads it.
#[derive(Default)]
struct Read {
    /// Its documents, in order.
    documents: Vec<Indexed>,
    /// The position of each distinct symbol.
    positions: BTreeMap<String, usize>,
    /// Each symbol's name and whether Cortex stores definitions of its kind, by position; `None`
    /// for a local symbol or one whose name cannot be read.
    symbols: Vec<Option<(Identifier, bool)>>,
    /// The occurrences read so far.
    occurrences: usize,
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
    let mut current: Vec<(&Indexed, FileId)> = Vec::new();
    for document in &index.documents {
        if let Some(file) = store.current_file(&document.path, written_ns)? {
            current.push((document, file));
        }
    }
    let definitions = store.definitions(&index, &current)?;
    let ties = store.ties(&index, &current, &definitions)?;
    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
    let outcome = ScipOutcome::Read {
        documents: count(index.documents.len()),
        left_out: count(index.documents.len() - current.len()),
        tied: count(ties.len()),
    };
    Ok((outcome, ties))
}

/// The index at `path` and when it was written, in nanoseconds since the epoch; `None` when there
/// is none.
fn load(path: &Path) -> Result<Option<(Read, i64)>, ScipProblem> {
    // When it was written is read first, so an index replaced while it is read is not taken as
    // newer than its contents.
    let written_ns = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_nanos()).ok())
            .ok_or(ScipProblem::Unreadable)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ScipProblem::Unreadable),
    };
    let bytes = match read_whole(path, MAX_INDEX_BYTES) {
        Ok(bytes) => bytes,
        Err(ReadError::NotRegular) => return Err(ScipProblem::NotAFile),
        Err(ReadError::TooLarge) => return Err(ScipProblem::TooLarge),
        Err(ReadError::Io(_)) => return Err(ScipProblem::Unreadable),
    };
    Ok(Some((read(&bytes)?, written_ns)))
}

/// The index `bytes` encode, read field by field.
fn read(bytes: &[u8]) -> Result<Read, ScipProblem> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut index = Read::default();
    while let Some(tag) = input.read_raw_tag_or_eof().map_err(not_an_index)? {
        if tag >> 3 == INDEX_DOCUMENTS && tag & 7 == 2 {
            let document = index.read_document(&mut input)?;
            index.documents.push(document);
        } else {
            skip_field_for_tag(tag, &mut input).map_err(not_an_index)?;
        }
    }
    Ok(index)
}

/// What a decoding error makes of the index.
fn not_an_index(_: protobuf::Error) -> ScipProblem {
    ScipProblem::NotAnIndex
}

impl Read {
    /// The document whose length comes next in `input`, read field by field.
    fn read_document(&mut self, input: &mut CodedInputStream<'_>) -> Result<Indexed, ScipProblem> {
        let length = input.read_raw_varint64().map_err(not_an_index)?;
        let outer = input.push_limit(length).map_err(not_an_index)?;
        let mut document = Indexed {
            path: String::new(),
            seen: Vec::new(),
        };
        while let Some(tag) = input.read_raw_tag_or_eof().map_err(not_an_index)? {
            match (tag >> 3, tag & 7) {
                (DOCUMENT_PATH, 2) => document.path = input.read_string().map_err(not_an_index)?,
                (DOCUMENT_OCCURRENCES, 2) => {
                    let occurrence: Occurrence = input.read_message().map_err(not_an_index)?;
                    self.occurrences += 1;
                    if self.occurrences > MAX_OCCURRENCES {
                        return Err(ScipProblem::TooLarge);
                    }
                    if let Some(seen) = self.seen(&occurrence)? {
                        document.seen.push(seen);
                    }
                }
                _ => skip_field_for_tag(tag, input).map_err(not_an_index)?,
            }
        }
        input.pop_limit(outer);
        Ok(document)
    }

    /// `occurrence` as matching reads it; `None` when it has no line or its symbol is too long.
    fn seen(&mut self, occurrence: &Occurrence) -> Result<Option<Seen>, ScipProblem> {
        let Some(line) = line_of(occurrence) else {
            return Ok(None);
        };
        if occurrence.symbol.len() > MAX_SYMBOL_BYTES {
            return Ok(None);
        }
        let next = self.symbols.len();
        let symbol = *self
            .positions
            .entry(occurrence.symbol.clone())
            .or_insert(next);
        if symbol == next {
            if next == MAX_SYMBOLS {
                return Err(ScipProblem::TooLarge);
            }
            self.symbols.push(symbols::last_name(&occurrence.symbol));
        }
        Ok(Some(Seen {
            line,
            symbol,
            is_definition: occurrence.symbol_roles & DEFINITION_ROLE != 0,
        }))
    }

    /// The name of the symbol at `symbol`, and whether Cortex stores definitions of its kind.
    fn name_of(&self, symbol: usize) -> Option<&(Identifier, bool)> {
        self.symbols.get(symbol)?.as_ref()
    }
}

/// The store as the index is matched against it, each file's rows read once and grouped.
struct Store<'q> {
    /// The read queries.
    queries: Queries<'q>,
    /// Each stored file's definitions, by name.
    definitions: BTreeMap<FileId, BTreeMap<Identifier, Vec<SymbolRow>>>,
    /// Each stored file's references, by name and line.
    references: BTreeMap<FileId, BTreeMap<(Identifier, u32), Vec<RefId>>>,
}

impl<'q> Store<'q> {
    /// The store read through `queries`.
    fn new(queries: Queries<'q>) -> Self {
        Self {
            queries,
            definitions: BTreeMap::new(),
            references: BTreeMap::new(),
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

    /// The stored definition of each symbol of a kind Cortex stores that `index` defines exactly
    /// once, in one of `current`, the documents whose files are as indexed.
    fn definitions(
        &mut self,
        index: &Read,
        current: &[(&Indexed, FileId)],
    ) -> Result<BTreeMap<usize, SymbolId>, StoreError> {
        let mut defined: BTreeMap<usize, u32> = BTreeMap::new();
        for seen in index.documents.iter().flat_map(|document| &document.seen) {
            if seen.is_definition {
                *defined.entry(seen.symbol).or_insert(0) += 1;
            }
        }
        let mut definitions = BTreeMap::new();
        for (document, file) in current {
            for seen in &document.seen {
                let Some((name, is_stored_kind)) = index.name_of(seen.symbol) else {
                    continue;
                };
                if !seen.is_definition || !is_stored_kind || defined.get(&seen.symbol) != Some(&1) {
                    continue;
                }
                if let Some(id) = self.definition_at(*file, name, seen.line)? {
                    definitions.insert(seen.symbol, id);
                }
            }
        }
        Ok(definitions)
    }

    /// The stored references the reference occurrences of `current` tie to `definitions`: on each
    /// line, those of a name as many as the occurrences of that name there, all of one defined
    /// symbol. A reference two documents tie to two definitions is left out.
    fn ties(
        &mut self,
        index: &Read,
        current: &[(&Indexed, FileId)],
        definitions: &BTreeMap<usize, SymbolId>,
    ) -> Result<BTreeMap<RefId, SymbolId>, StoreError> {
        let mut ties: BTreeMap<RefId, Option<SymbolId>> = BTreeMap::new();
        for (document, file) in current {
            let mut lines: BTreeMap<(&Identifier, u32), Vec<usize>> = BTreeMap::new();
            for seen in document.seen.iter().filter(|seen| !seen.is_definition) {
                if let Some((name, _)) = index.name_of(seen.symbol) {
                    lines
                        .entry((name, seen.line))
                        .or_default()
                        .push(seen.symbol);
                }
            }
            for ((name, line), symbols) in lines {
                let targets: Vec<Option<&SymbolId>> = symbols
                    .iter()
                    .map(|symbol| definitions.get(symbol))
                    .collect();
                let Some(&target) = targets.first().copied().flatten() else {
                    continue;
                };
                if targets.iter().any(|other| *other != Some(&target)) {
                    continue;
                }
                let references = self.references_at(*file, name, line)?;
                if references.len() != symbols.len() {
                    continue;
                }
                for reference in references {
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

    /// The innermost stored definition named `name` in `file` whose lines hold `line`, when there
    /// is exactly one innermost.
    fn definition_at(
        &mut self,
        file: FileId,
        name: &Identifier,
        line: u32,
    ) -> Result<Option<SymbolId>, StoreError> {
        if !self.definitions.contains_key(&file) {
            let mut by_name: BTreeMap<Identifier, Vec<SymbolRow>> = BTreeMap::new();
            for symbol in self.queries.symbols_in_file(file)? {
                by_name.entry(symbol.name.clone()).or_default().push(symbol);
            }
            self.definitions.insert(file, by_name);
        }
        let named = self
            .definitions
            .get(&file)
            .and_then(|by_name| by_name.get(name))
            .map_or(&[][..], Vec::as_slice);
        let holding: Vec<&SymbolRow> = named
            .iter()
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
        if !self.references.contains_key(&file) {
            let mut by_place: BTreeMap<(Identifier, u32), Vec<RefId>> = BTreeMap::new();
            for reference in self.queries.refs_in_file(file)? {
                let place = (reference.dst_name.clone(), reference.line);
                by_place.entry(place).or_default().push(reference.id);
            }
            self.references.insert(file, by_place);
        }
        Ok(self
            .references
            .get(&file)
            .and_then(|by_place| by_place.get(&(name.clone(), line)))
            .cloned()
            .unwrap_or_default())
    }
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
