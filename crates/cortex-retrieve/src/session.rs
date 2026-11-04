//! What the session the prompt comes in has been doing: the code it edited, which a follow-up
//! prompt is most likely about; the spans it read last; and every span it read since its context
//! was last compacted, which the agent still holds and need not be shown again.
//!
//! Edits are those of the session's tasks, its subagents' included, since they changed the code the
//! session works on; reads are the session's own, since a subagent's reads are in the subagent's
//! context, not the session's. An edited or read stretch of lines counts for the symbols it touches,
//! or for its file when it touches none or names no lines. A node edited and read carries both
//! weights; edited or read again, it carries one. A read or an edit that failed counts for nothing:
//! it read or changed nothing.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EventKind, Identifier, LineRange, RepoPath, ShortHash};
use cortex_store::{EventRow, FileId, NodeId, Queries, StoreError, SymbolSpan};

use crate::graph::Graph;

/// A seed's weight for code edited in the session.
pub const EDITED_WEIGHT: f64 = 1.0;

/// A seed's weight for code among the session's latest reads.
pub const READ_WEIGHT: f64 = 0.4;

/// How many of the session's latest reads are seeds: read rows, so a read of several stretches of a
/// file, a row for each, counts once for each stretch.
pub const SEEDED_READS: usize = 10;

/// Most edit rows read: the session's latest, its subagents' included.
const MAX_EDITS: usize = 200;

/// Most read and compaction rows read: the session's latest, so a session that has read more since
/// its last compaction is taken to hold only the latest of them.
const MAX_READS: usize = 500;

/// A stretch of a file the session read and still holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadSpan {
    /// The file.
    pub path: RepoPath,
    /// The lines read; none for the whole file.
    pub range: Option<LineRange>,
    /// The short hash of what the index held for the file when it was read, when known.
    pub content: Option<ShortHash>,
}

/// What the session shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionMatch {
    /// Each node it seeds, with its weight, by node.
    pub seeds: BTreeMap<u32, f64>,
    /// The files it edited, by node.
    pub edited_files: BTreeSet<u32>,
    /// The spans it read since its last compaction, newest first.
    pub in_context: Vec<ReadSpan>,
}

/// What session `session` has been doing, as the store `queries` reads holds it, as nodes of
/// `graph`.
///
/// # Errors
///
/// The store's errors.
pub fn session(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    session: &Identifier,
) -> Result<SessionMatch, StoreError> {
    let mut places = Places::new(queries);
    let edits = queries.edits_of_session(session, MAX_EDITS)?;
    let (mut seeds, edited_files) = edited(&mut places, graph, &edits)?;
    let reads = queries.reads_of_session(session, MAX_READS)?;
    // A node edited and read is seeded for both; edited or read twice, once.
    for (node, weight) in read(&mut places, graph, &reads)? {
        *seeds.entry(node).or_insert(0.0) += weight;
    }
    Ok(SessionMatch {
        seeds,
        edited_files,
        in_context: in_context(&reads),
    })
}

/// The nodes `edits` changed, each weighed [`EDITED_WEIGHT`], and the files they are in.
fn edited(
    places: &mut Places<'_>,
    graph: &Graph<NodeId>,
    edits: &[EventRow],
) -> Result<(BTreeMap<u32, f64>, BTreeSet<u32>), StoreError> {
    let mut seeds: BTreeMap<u32, f64> = BTreeMap::new();
    let mut files: BTreeSet<u32> = BTreeSet::new();
    for row in edits.iter().filter(|row| is_done(row)) {
        for node in places.nodes(row)? {
            let Some(index) = graph.index_of(node) else {
                continue;
            };
            raise(&mut seeds, index, EDITED_WEIGHT);
            if let Some(file) = graph.file_of(index) {
                files.insert(file);
            }
        }
    }
    Ok((seeds, files))
}

/// The nodes the latest [`SEEDED_READS`] reads of `reads`, newest first, touched, each weighed
/// [`READ_WEIGHT`]; a compaction or a failed read takes none of the ten.
fn read(
    places: &mut Places<'_>,
    graph: &Graph<NodeId>,
    reads: &[EventRow],
) -> Result<BTreeMap<u32, f64>, StoreError> {
    let mut seeds: BTreeMap<u32, f64> = BTreeMap::new();
    let latest = reads
        .iter()
        .filter(|row| row.record.kind == EventKind::Read && is_done(row))
        .take(SEEDED_READS);
    for row in latest {
        for node in places.nodes(row)? {
            if let Some(index) = graph.index_of(node) {
                raise(&mut seeds, index, READ_WEIGHT);
            }
        }
    }
    Ok(seeds)
}

/// The spans `reads`, newest first, read since the latest compaction among them, newest first.
fn in_context(reads: &[EventRow]) -> Vec<ReadSpan> {
    reads
        .iter()
        .take_while(|row| row.record.kind == EventKind::Read)
        .filter(|row| is_done(row))
        .filter_map(|row| {
            Some(ReadSpan {
                path: row.record.path.clone()?,
                range: row.record.range,
                content: row.record.meta.content,
            })
        })
        .collect()
}

/// Whether `row` records work that was done: not a read or an edit that failed.
fn is_done(row: &EventRow) -> bool {
    row.record.meta.is_success != Some(false)
}

/// The symbols a stretch of lines touches, each file read once.
struct Places<'a> {
    /// The store.
    queries: Queries<'a>,
    /// Each file asked about: its node and its symbols' spans, or none when the store holds no
    /// such file.
    files: BTreeMap<RepoPath, Option<(NodeId, Vec<SymbolSpan>)>>,
}

impl<'a> Places<'a> {
    /// Places read from `queries`.
    fn new(queries: Queries<'a>) -> Self {
        Self {
            queries,
            files: BTreeMap::new(),
        }
    }

    /// The nodes `row`'s lines touch: the symbols of its file whose lines meet them, or the file
    /// when they meet none or the row names no lines; none for a file the store does not hold.
    fn nodes(&mut self, row: &EventRow) -> Result<Vec<NodeId>, StoreError> {
        let Some(path) = &row.record.path else {
            return Ok(Vec::new());
        };
        if !self.files.contains_key(path) {
            let held = match self.queries.file(path)? {
                Some(file) => self.file_node(path, file.id)?,
                None => None,
            };
            self.files.insert(path.clone(), held);
        }
        let Some(Some((file, spans))) = self.files.get(path) else {
            return Ok(Vec::new());
        };
        let Some(range) = row.record.range else {
            return Ok(vec![*file]);
        };
        let touched: Vec<NodeId> = spans
            .iter()
            .filter(|span| {
                span.start_line <= range.end_line() && range.start_line() <= span.end_line
            })
            .map(|span| span.node)
            .collect();
        Ok(if touched.is_empty() {
            vec![*file]
        } else {
            touched
        })
    }

    /// The node and symbol spans of the file at `path`, stored as `file`.
    fn file_node(
        &self,
        path: &RepoPath,
        file: FileId,
    ) -> Result<Option<(NodeId, Vec<SymbolSpan>)>, StoreError> {
        let Some(node) = self.queries.file_node_at(path)? else {
            return Ok(None);
        };
        Ok(Some((node, self.queries.symbol_spans_in_file(file)?)))
    }
}

/// Raises `node`'s weight in `seeds` to `weight` when that is more than it has.
fn raise(seeds: &mut BTreeMap<u32, f64>, node: u32, weight: f64) {
    let held = seeds.entry(node).or_insert(0.0);
    *held = held.max(weight);
}
