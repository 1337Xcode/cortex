//! Folder nodes: the graph's nodes for folders, which have no row of their own and are known by
//! their path.

use cortex_core::RepoPath;
use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::schema;

/// The node of the folder at `path`, added when it has none.
pub(crate) fn folder_node(connection: &Connection, path: &RepoPath) -> Result<NodeId, StoreError> {
    if let Some(node) = find(connection, path)? {
        return Ok(node);
    }
    connection
        .prepare_cached(schema::FOLDER_NODE_INSERT)
        .and_then(|mut statement| statement.query_row([path.as_str()], |row| row.get(0)))
        .map(NodeId)
        .map_err(sqlite("adding a folder node"))
}

/// Deletes the node of the folder at `path`; the schema deletes every edge at it with it. `false`
/// when the folder had no node.
pub(crate) fn remove_folder_node(
    connection: &Connection,
    path: &RepoPath,
) -> Result<bool, StoreError> {
    let Some(node) = find(connection, path)? else {
        return Ok(false);
    };
    connection
        .prepare_cached(schema::NODE_DELETE)
        .and_then(|mut statement| statement.execute([node.get()]))
        .map_err(sqlite("deleting a folder node"))?;
    Ok(true)
}

/// The node of the folder at `path`, if it has one.
fn find(connection: &Connection, path: &RepoPath) -> Result<Option<NodeId>, StoreError> {
    connection
        .prepare_cached(schema::FOLDER_NODE)
        .and_then(|mut statement| {
            statement
                .query_row([path.as_str()], |row| row.get(0))
                .optional()
        })
        .map(|node| node.map(NodeId))
        .map_err(sqlite("finding a folder node"))
}
