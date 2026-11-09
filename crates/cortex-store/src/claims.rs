//! Advisory claims as the store keeps them. A claim is a session's position of mode claim, kept
//! as any position is (`WriteTx::keep_position`); here claims are released, whoever holds them,
//! since a claim only ever informs and the user may clear any.

use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::schema;

/// Removes the claims on `nodes`, or every claim when `nodes` is none: how many were removed.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn release(
    connection: &Connection,
    nodes: Option<&[NodeId]>,
) -> Result<u64, StoreError> {
    let listed = nodes
        .map(|nodes| {
            let ids: Vec<i64> = nodes.iter().map(|node| node.get()).collect();
            serde_json::to_string(&ids).map_err(|_| StoreError::Corrupt {
                column: "positions.node_id",
                row: 0,
            })
        })
        .transpose()?;
    let removed = connection
        .prepare_cached(schema::CLAIMS_RELEASE)
        .and_then(|mut statement| statement.execute([listed]))
        .map_err(sqlite("releasing claims"))?;
    Ok(u64::try_from(removed).unwrap_or(u64::MAX))
}
