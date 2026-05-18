//! Memory invalidation: automatically mark observations stale when linked nodes
//! are modified or deleted.
//!
//! This module provides a higher-level interface for triggering observation
//! invalidation outside of delta application. The `apply_delta` function in
//! `store::queries::delta` already handles staleness marking internally during
//! graph updates. These functions are useful for the indexing pipeline or other
//! modules that need to trigger invalidation independently.

use crate::error::MemoryError;
use crate::store::db::StoreManager;
use crate::store::queries::memory::mark_observations_stale;

/// Mark observations stale for nodes that have been modified (file hash changed).
/// Returns the total number of observations marked stale.
pub fn invalidate_for_modified_nodes(
    store: &StoreManager,
    fqns: &[String],
) -> Result<usize, MemoryError> {
    let conn = store.write_conn();
    let mut total = 0;
    for fqn in fqns {
        let count = mark_observations_stale(&conn, fqn, "node_modified")
            .map_err(|e| MemoryError::ObservationFailed {
                reason: format!("failed to invalidate for modified node '{}': {}", fqn, e),
            })?;
        total += count;
    }
    Ok(total)
}

/// Mark observations stale for nodes that have been deleted.
/// Returns the total number of observations marked stale.
pub fn invalidate_for_deleted_nodes(
    store: &StoreManager,
    fqns: &[String],
) -> Result<usize, MemoryError> {
    let conn = store.write_conn();
    let mut total = 0;
    for fqn in fqns {
        let count = mark_observations_stale(&conn, fqn, "node_deleted")
            .map_err(|e| MemoryError::ObservationFailed {
                reason: format!("failed to invalidate for deleted node '{}': {}", fqn, e),
            })?;
        total += count;
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::queries::memory::{read_observations, write_observation};

    /// Creates a StoreManager with a temporary directory and applies migrations.
    fn setup_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");

        // Apply memory tables migration
        {
            let conn = store.write_conn();
            let migration_0003 = include_str!("../../migrations/0003_memory_tables.sql");
            conn.execute_batch(migration_0003)
                .expect("failed to apply migration 0003");
        }

        (store, tmp)
    }

    #[test]
    fn test_modified_node_stales_observation() {
        let (store, _tmp) = setup_store();

        // Write an observation for node A
        {
            let conn = store.write_conn();
            write_observation(&conn, "src/a.rs::foo", "observation about foo", "agent-1", "hash_a")
                .expect("failed to write observation");
        }

        // Invalidate for modified node A
        let staled = invalidate_for_modified_nodes(
            &store,
            &["src/a.rs::foo".to_string()],
        )
        .expect("invalidation failed");

        assert_eq!(staled, 1);

        // Verify observation is stale with reason "node_modified"
        {
            let conn = store.read_conn();
            let observations = read_observations(&conn, "src/a.rs::foo", true)
                .expect("failed to read observations");
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].status, "stale");
            assert_eq!(observations[0].stale_reason.as_deref(), Some("node_modified"));
        }
    }

    #[test]
    fn test_deleted_node_stales_observation() {
        let (store, _tmp) = setup_store();

        // Write an observation for node B
        {
            let conn = store.write_conn();
            write_observation(&conn, "src/b.rs::bar", "observation about bar", "agent-2", "hash_b")
                .expect("failed to write observation");
        }

        // Invalidate for deleted node B
        let staled = invalidate_for_deleted_nodes(
            &store,
            &["src/b.rs::bar".to_string()],
        )
        .expect("invalidation failed");

        assert_eq!(staled, 1);

        // Verify observation is stale with reason "node_deleted"
        {
            let conn = store.read_conn();
            let observations = read_observations(&conn, "src/b.rs::bar", true)
                .expect("failed to read observations");
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].status, "stale");
            assert_eq!(observations[0].stale_reason.as_deref(), Some("node_deleted"));
        }
    }

    #[test]
    fn test_unchanged_node_preserves_observation() {
        let (store, _tmp) = setup_store();

        // Write an observation for node C
        {
            let conn = store.write_conn();
            write_observation(&conn, "src/c.rs::baz", "observation about baz", "agent-3", "hash_c")
                .expect("failed to write observation");
        }

        // Do NOT invalidate node C - only invalidate a different node
        let staled = invalidate_for_modified_nodes(
            &store,
            &["src/other.rs::unrelated".to_string()],
        )
        .expect("invalidation failed");

        assert_eq!(staled, 0);

        // Verify observation for node C remains active
        {
            let conn = store.read_conn();
            let observations = read_observations(&conn, "src/c.rs::baz", false)
                .expect("failed to read observations");
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].status, "active");
            assert!(observations[0].stale_reason.is_none());
        }
    }

    #[test]
    fn test_invalidate_multiple_nodes() {
        let (store, _tmp) = setup_store();

        // Write observations for multiple nodes
        {
            let conn = store.write_conn();
            write_observation(&conn, "src/x.rs::alpha", "obs alpha", "agent", "hash")
                .expect("failed to write observation");
            write_observation(&conn, "src/y.rs::beta", "obs beta", "agent", "hash")
                .expect("failed to write observation");
        }

        // Invalidate both
        let staled = invalidate_for_modified_nodes(
            &store,
            &["src/x.rs::alpha".to_string(), "src/y.rs::beta".to_string()],
        )
        .expect("invalidation failed");

        assert_eq!(staled, 2);
    }

    #[test]
    fn test_invalidate_empty_list_returns_zero() {
        let (store, _tmp) = setup_store();

        let staled = invalidate_for_modified_nodes(&store, &[])
            .expect("invalidation failed");
        assert_eq!(staled, 0);

        let staled = invalidate_for_deleted_nodes(&store, &[])
            .expect("invalidation failed");
        assert_eq!(staled, 0);
    }
}
