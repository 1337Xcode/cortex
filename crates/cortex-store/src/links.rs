//! Writes that tie rows together: the targets of references and the edges of the graph.

use cortex_core::{EdgeType, Resolution};
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
            None if resolved.goes_through_import => {
                statement.execute((id, None::<i64>, Some(Resolution::ImportPath.name()), 0.0))
            }
            None => statement.execute((id, None::<i64>, None::<&str>, 0.0)),
        }
        .map_err(sqlite("resolving references"))?;
        if changed != 1 {
            return Err(StoreError::UnknownRow { table: "refs", id });
        }
    }
    Ok(())
}

/// Sets each edge of `edge_type`: its weight, how many observations support it, the task it was
/// last seen at and whether it points one way, adding the edge when it is not there and leaving
/// every other edge of the type alone.
pub(crate) fn set_edges(
    connection: &Connection,
    edge_type: EdgeType,
    edges: &[(NodeId, EdgeRecord)],
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::EDGE_SET)
        .map_err(sqlite("setting edges"))?;
    for (src, edge) in edges {
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
            .map_err(sqlite("setting edges"))?;
    }
    Ok(())
}

/// Deletes every edge of `edge_type`.
pub(crate) fn remove_edges_of_type(
    connection: &Connection,
    edge_type: EdgeType,
) -> Result<(), StoreError> {
    connection
        .execute(schema::EDGES_DELETE_OF_TYPE, [edge_type.number()])
        .map(drop)
        .map_err(sqlite("deleting edges"))
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
