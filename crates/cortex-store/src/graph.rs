//! Reading the whole graph at once, as retrieval holds it in memory: every node with its kind, for a
//! symbol the node of the file it is defined in, its interest and for a file its flags; every edge;
//! and the number of the latest task, which faded weights and interest are brought forward to.
//!
//! Each read streams its rows to the caller, so that a graph of millions of edges is never held
//! twice, once as rows and once as what the caller builds from them.

use cortex_core::{FileFlags, NodeKind};
use rusqlite::Row;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::interest::{InterestRow, checked};
use crate::queries::Queries;
use crate::raw::RawEdge;
use crate::rows::EdgeRow;
use crate::schema;
use crate::values;

/// One node as the graph reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNodeRow {
    /// The node.
    pub node: NodeId,
    /// What it is.
    pub kind: NodeKind,
    /// For a symbol, the node of the file it is defined in.
    pub file: Option<NodeId>,
    /// Its interest, when it has any.
    pub interest: Option<InterestRow>,
    /// For a file, its flags.
    pub flags: Option<FileFlags>,
}

impl Queries<'_> {
    /// Hands `each` every node, by node, with its kind, for a symbol its file's node, its interest
    /// and for a file its flags.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back, `each`'s first error included:
    /// the read stops there.
    pub fn each_graph_node(
        self,
        mut each: impl FnMut(GraphNodeRow) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::GRAPH_NODES)
            .map_err(sqlite("reading the graph's nodes"))?;
        let mut rows = statement
            .query([])
            .map_err(sqlite("reading the graph's nodes"))?;
        while let Some(row) = rows.next().map_err(sqlite("reading the graph's nodes"))? {
            each(graph_node(row)?)?;
        }
        Ok(())
    }

    /// Hands `each` every edge, by source, type and target.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back, `each`'s first error included:
    /// the read stops there.
    pub fn each_edge(
        self,
        mut each: impl FnMut(EdgeRow) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::ALL_EDGES)
            .map_err(sqlite("reading edges"))?;
        let mut rows = statement.query([]).map_err(sqlite("reading edges"))?;
        while let Some(row) = rows.next().map_err(sqlite("reading edges"))? {
            let raw = RawEdge::read(row).map_err(sqlite("reading edges"))?;
            each(raw.check()?)?;
        }
        Ok(())
    }

    /// The number of the latest task the store holds, 0 when it holds none.
    ///
    /// # Errors
    ///
    /// SQLite errors and a number that cannot be read back.
    pub fn latest_task_seq(self) -> Result<u64, StoreError> {
        let seq: i64 = self
            .connection()
            .prepare_cached(schema::LATEST_TASK_SEQ)
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(sqlite("reading the latest task's number"))?;
        values::number(seq, "tasks.seq", seq)
    }
}

/// The columns of a row of [`schema::GRAPH_NODES`]: the node, its kind and file, its interest's
/// score, task and tier, and a file's flags.
type RawGraphNode = (
    i64,
    String,
    Option<i64>,
    Option<f64>,
    Option<i64>,
    Option<u8>,
    Option<i64>,
);

/// The node `row` of [`schema::GRAPH_NODES`] holds.
fn graph_node(row: &Row<'_>) -> Result<GraphNodeRow, StoreError> {
    let read = |row: &Row<'_>| -> rusqlite::Result<RawGraphNode> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
        ))
    };
    let (node, kind, file, score, last_seq, tier, flags) =
        read(row).map_err(sqlite("reading the graph's nodes"))?;
    let kind = values::kind(&kind, NodeKind::from_name, "nodes.kind", node)?;
    let interest = match (score, last_seq, tier) {
        (Some(score), Some(last_seq), Some(tier)) => Some(checked((node, score, last_seq, tier))?),
        _ => None,
    };
    let flags = flags
        .map(|bits| values::number(bits, "files.flags", node).map(FileFlags::from_bits))
        .transpose()?;
    Ok(GraphNodeRow {
        node: NodeId(node),
        kind,
        file: file.map(NodeId),
        interest,
        flags,
    })
}
