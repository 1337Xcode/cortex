//! Where a task's work lands: the symbols the lines of a read or an edit touch, and the folder
//! holding a file.
//!
//! One task touches the same file again and again, so each file is asked about once and kept.

use std::collections::BTreeMap;

use cortex_core::{LineRange, RepoPath};
use cortex_store::{NodeId, Queries, StoreError, SymbolSpan};

/// The places a task's work lands, each file asked about once.
pub(crate) struct Places<'a> {
    /// The store.
    queries: Queries<'a>,
    /// Each file's symbols, by the file's path; empty for a file with none and for one the store
    /// does not hold.
    symbols: BTreeMap<RepoPath, Vec<SymbolSpan>>,
    /// The node of the folder holding each file, by the file's path.
    folders: BTreeMap<RepoPath, Option<NodeId>>,
}

impl<'a> Places<'a> {
    /// Nothing asked yet.
    pub(crate) fn new(queries: Queries<'a>) -> Self {
        Self {
            queries,
            symbols: BTreeMap::new(),
            folders: BTreeMap::new(),
        }
    }

    /// The nodes of the symbols of the file at `path` that the lines `range` touch, in the order
    /// they appear in the file.
    pub(crate) fn covering(
        &mut self,
        path: &RepoPath,
        range: LineRange,
    ) -> Result<Vec<NodeId>, StoreError> {
        Ok(self
            .covering_spans(path, range)?
            .into_iter()
            .map(|span| span.node)
            .collect())
    }

    /// Those symbols with the lines they cover, for a caller that needs to know where each one
    /// starts.
    pub(crate) fn covering_spans(
        &mut self,
        path: &RepoPath,
        range: LineRange,
    ) -> Result<Vec<SymbolSpan>, StoreError> {
        if !self.symbols.contains_key(path) {
            let file = self.queries.file(path)?.map(|row| row.id);
            let spans = match file {
                Some(file) => self.queries.symbol_spans_in_file(file)?,
                None => Vec::new(),
            };
            self.symbols.insert(path.clone(), spans);
        }
        let spans = self.symbols.get(path).map_or(&[][..], Vec::as_slice);
        Ok(spans
            .iter()
            .filter(|span| {
                LineRange::new(span.start_line, span.end_line)
                    .is_ok_and(|lines| lines.overlaps(range))
            })
            .cloned()
            .collect())
    }

    /// The node of the folder holding the file at `path`, when that folder has one.
    pub(crate) fn folder_of(&mut self, path: &RepoPath) -> Result<Option<NodeId>, StoreError> {
        if let Some(folder) = self.folders.get(path) {
            return Ok(*folder);
        }
        let folder = match path.parent() {
            Some(folder) => self.queries.folder_node_at(&folder)?,
            None => None,
        };
        self.folders.insert(path.clone(), folder);
        Ok(folder)
    }
}
