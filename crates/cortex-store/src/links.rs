//! Writes that tie rows together: the targets of references and the edges of the graph.

use cortex_core::EdgeType;
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::records::{EdgeRecord, Resolved};
use crate::schema;

/// Sets each reference's target, resolution and confidence, or marks it unresolved.
pub(crate) fn set_resolutions(
    connection: &Connection,
    resolutions: &[Resolved],
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::REF_RESOLVE)
        .map_err(sqlite("resolving references"))?;
    for resolved in resolutions {
        let id = resolved.reference.get();
        let changed = match resolved.target {
            Some(target) => statement.execute((
                id,
                Some(target.symbol.get()),
                Some(target.resolution.name()),
                target.resolution.confidence(),
            )),
            None => statement.execute((id, None::<i64>, None::<&str>, 0.0)),
        }
        .map_err(sqlite("resolving references"))?;
        if changed != 1 {
            return Err(StoreError::UnknownRow { table: "refs", id });
        }
    }
    Ok(())
}

/// Replaces the edges of `edge_type` that start at `src` with `edges`.
pub(crate) fn replace_edges(
    connection: &Connection,
    src: NodeId,
    edge_type: EdgeType,
    edges: &[EdgeRecord],
) -> Result<(), StoreError> {
    connection
        .prepare_cached(schema::EDGES_DELETE_FROM)
        .and_then(|mut statement| statement.execute((src.get(), edge_type.number())))
        .map_err(sqlite("deleting edges"))?;
    let mut statement = connection
        .prepare_cached(schema::EDGE_INSERT)
        .map_err(sqlite("adding edges"))?;
    for edge in edges {
        statement
            .execute((
                src.get(),
                edge.dst.get(),
                edge_type.number(),
                edge.weight,
                edge.count,
                edge.last_seq,
                edge.is_directed,
            ))
            .map_err(sqlite("adding edges"))?;
    }
    Ok(())
}
