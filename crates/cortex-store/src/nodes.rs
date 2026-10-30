//! Graph nodes and their search rows. A node is written with the identifier tokens of what names
//! it and removed with them, so the search index never holds a row for a node that is gone.

use cortex_core::{NodeKind, RepoPath, identifier_tokens};
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::schema;

/// Adds the node for row `id` of the table `kind` names, or updates its path, and returns it.
pub(crate) fn upsert_node(
    connection: &Connection,
    kind: NodeKind,
    id: i64,
    path: &RepoPath,
) -> Result<NodeId, StoreError> {
    connection
        .prepare_cached(schema::NODE_UPSERT)
        .and_then(|mut statement| {
            statement.query_row((kind.name(), id, path.as_str()), |row| row.get(0))
        })
        .map(NodeId)
        .map_err(sqlite("writing a node"))
}

/// Deletes a node and its search row; the schema deletes every edge at it with it.
pub(crate) fn remove_node(connection: &Connection, node: NodeId) -> Result<(), StoreError> {
    delete_tokens(connection, node)?;
    connection
        .prepare_cached(schema::NODE_DELETE)
        .and_then(|mut statement| statement.execute([node.get()]))
        .map(drop)
        .map_err(sqlite("deleting a node"))
}

/// Sets the search row of `node` to the tokens of `texts`, its names or its path; a node whose
/// texts hold no token has no row.
pub(crate) fn write_tokens(
    connection: &Connection,
    node: NodeId,
    texts: &[&str],
) -> Result<(), StoreError> {
    delete_tokens(connection, node)?;
    // INVARIANT: no-content-stored. A search row holds the words of a node's names or path only.
    let tokens: Vec<String> = texts
        .iter()
        .flat_map(|text| identifier_tokens(text))
        .collect();
    if tokens.is_empty() {
        return Ok(());
    }
    connection
        .prepare_cached(schema::SEARCH_INSERT)
        .and_then(|mut statement| statement.execute((node.get(), tokens.join(" "))))
        .map(drop)
        .map_err(sqlite("writing a search row"))
}

/// Deletes the search row of `node`, if it has one.
fn delete_tokens(connection: &Connection, node: NodeId) -> Result<(), StoreError> {
    connection
        .prepare_cached(schema::SEARCH_DELETE)
        .and_then(|mut statement| statement.execute([node.get()]))
        .map(drop)
        .map_err(sqlite("deleting a search row"))
}
