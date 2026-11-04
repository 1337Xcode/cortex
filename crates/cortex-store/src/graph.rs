//! Reading the whole graph at once, as retrieval holds it in memory: every node with its kind and,
//! for a symbol, the node of the file it is defined in; every edge; and the number of the latest
//! task, which faded weights are brought forward to.
//!
//! Each read streams its rows to the caller, so that a graph of millions of edges is never held
//! twice, once as rows and once as what the caller builds from them.

use cortex_core::NodeKind;
use rusqlite::Row;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::raw::RawEdge;
use crate::rows::EdgeRow;
use crate::schema;
use crate::values;

/// One node as the graph reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphNodeRow {
    /// The node.
    pub node: NodeId,
    /// What it is.
    pub kind: NodeKind,
    /// For a symbol, the node of the file it is defined in.
    pub file: Option<NodeId>,
}

impl Queries<'_> {
    /// Hands `each` every node, by node, with its kind and, for a symbol, its file's node.
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

/// The node `row` of [`schema::GRAPH_NODES`] holds.
fn graph_node(row: &Row<'_>) -> Result<GraphNodeRow, StoreError> {
    let read = |row: &Row<'_>| -> rusqlite::Result<(i64, String, Option<i64>)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    };
    let (node, kind, file) = read(row).map_err(sqlite("reading the graph's nodes"))?;
    let kind = values::kind(&kind, NodeKind::from_name, "nodes.kind", node)?;
    Ok(GraphNodeRow {
        node: NodeId(node),
        kind,
        file: file.map(NodeId),
    })
}
