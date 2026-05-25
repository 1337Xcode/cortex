//! Memory store operations: CRUD for observations, ADRs, and change notes.
//!
//! All functions operate on a single `rusqlite::Connection` and use the types
//! defined in `crate::store::types`. UUID v4 is used for generating IDs and
//! Unix timestamps (seconds since epoch) for time fields.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use uuid::Uuid;

use crate::error::StoreError;
use crate::store::types::{Adr, ChangeNote, Observation};

/// Write a new observation linked to a node FQN.
/// Generates UUID v4 id, inserts with status='active', returns the id.
pub fn write_observation(
    conn: &Connection,
    node_fqn: &str,
    text: &str,
    agent_id: &str,
    node_hash: &str,
) -> Result<String, StoreError> {
    let id = Uuid::new_v4().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
        rusqlite::params![id, node_fqn, text, agent_id, node_hash, now],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to write observation: {}", e),
    })?;

    Ok(id)
}

/// Read observations for a node FQN.
/// If include_stale is false, only returns active observations.
pub fn read_observations(
    conn: &Connection,
    node_fqn: &str,
    include_stale: bool,
) -> Result<Vec<Observation>, StoreError> {
    let sql = if include_stale {
        "SELECT id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason \
         FROM observations WHERE node_fqn = ?1 ORDER BY written_at DESC"
    } else {
        "SELECT id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason \
         FROM observations WHERE node_fqn = ?1 AND status = 'active' ORDER BY written_at DESC"
    };

    let mut stmt = conn.prepare(sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare read_observations query: {}", e),
    })?;

    let rows = stmt
        .query_map(rusqlite::params![node_fqn], |row| {
            Ok(Observation {
                id: row.get(0)?,
                node_fqn: row.get(1)?,
                observation_text: row.get(2)?,
                agent_id: row.get(3)?,
                node_hash_at_write: row.get(4)?,
                written_at: row.get(5)?,
                status: row.get(6)?,
                stale_reason: row.get(7)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute read_observations query: {}", e),
        })?;

    let mut observations = Vec::new();
    for row in rows {
        observations.push(row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read observation row: {}", e),
        })?);
    }

    Ok(observations)
}

/// Mark all active observations for a node FQN as stale with the given reason.
/// Returns the number of observations marked stale.
pub fn mark_observations_stale(
    conn: &Connection,
    node_fqn: &str,
    reason: &str,
) -> Result<usize, StoreError> {
    let count = conn
        .execute(
            "UPDATE observations SET status = 'stale', stale_reason = ?1 \
             WHERE node_fqn = ?2 AND status = 'active'",
            rusqlite::params![reason, node_fqn],
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to mark observations stale: {}", e),
        })?;

    Ok(count)
}

/// Write a new ADR. Generates UUID v4 id, returns the id.
pub fn write_adr(
    conn: &Connection,
    title: &str,
    body: &str,
    status: &str,
    linked_fqn: Option<&str>,
) -> Result<String, StoreError> {
    let id = Uuid::new_v4().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO architectural_decisions (id, title, body, status, linked_fqn, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, title, body, status, linked_fqn, now, now],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to write ADR: {}", e),
    })?;

    Ok(id)
}

/// Read ADRs with optional filters.
/// If linked_fqn is Some, filters by linked_fqn.
/// If status is Some, filters by status.
pub fn read_adrs(
    conn: &Connection,
    linked_fqn: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Adr>, StoreError> {
    let mut sql = String::from(
        "SELECT id, title, body, status, linked_fqn, created_at, updated_at \
         FROM architectural_decisions WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(fqn) = linked_fqn {
        sql.push_str(&format!(" AND linked_fqn = ?{}", params.len() + 1));
        params.push(Box::new(fqn.to_string()));
    }

    if let Some(s) = status {
        sql.push_str(&format!(" AND status = ?{}", params.len() + 1));
        params.push(Box::new(s.to_string()));
    }

    sql.push_str(" ORDER BY created_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare read_adrs query: {}", e),
    })?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(Adr {
                id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
                status: row.get(3)?,
                linked_fqn: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute read_adrs query: {}", e),
        })?;

    let mut adrs = Vec::new();
    for row in rows {
        adrs.push(row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read ADR row: {}", e),
        })?);
    }

    Ok(adrs)
}

/// Prune stale observations older than the given number of days.
/// Sets status to 'archived'. Returns the number archived.
/// If older_than_days is None, archives all stale observations.
pub fn prune_stale_observations(
    conn: &Connection,
    older_than_days: Option<u32>,
) -> Result<usize, StoreError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let count = if let Some(days) = older_than_days {
        let cutoff = now - (days as i64 * 86400);
        conn.execute(
            "UPDATE observations SET status = 'archived' \
             WHERE status = 'stale' AND written_at < ?1",
            rusqlite::params![cutoff],
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prune stale observations: {}", e),
        })?
    } else {
        conn.execute(
            "UPDATE observations SET status = 'archived' WHERE status = 'stale'",
            [],
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prune stale observations: {}", e),
        })?
    };

    Ok(count)
}

/// Write a new change note. Generates UUID v4 id, returns the id.
pub fn write_change_note(conn: &Connection, text: &str) -> Result<String, StoreError> {
    let id = Uuid::new_v4().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO change_notes (id, text, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, text, now],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to write change note: {}", e),
    })?;

    Ok(id)
}

/// Read all change notes, ordered by created_at DESC.
pub fn read_change_notes(conn: &Connection) -> Result<Vec<ChangeNote>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT id, text, created_at FROM change_notes ORDER BY created_at DESC")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare read_change_notes query: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ChangeNote {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute read_change_notes query: {}", e),
        })?;

    let mut notes = Vec::new();
    for row in rows {
        notes.push(row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read change note row: {}", e),
        })?);
    }

    Ok(notes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates an in-memory SQLite connection with the memory tables migration applied.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("failed to enable foreign keys");

        let migration_0003 = include_str!("../../../migrations/0003_memory_tables.sql");
        conn.execute_batch(migration_0003)
            .expect("failed to apply migration 0003");

        conn
    }

    // -----------------------------------------------------------------------
    // Observation CRUD tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_and_read_observation() {
        let conn = setup_db();

        let id = write_observation(
            &conn,
            "src/main.rs::main",
            "This function handles startup",
            "claude",
            "hash_abc",
        )
        .unwrap();

        assert!(!id.is_empty());

        let observations = read_observations(&conn, "src/main.rs::main", false).unwrap();
        assert_eq!(observations.len(), 1);

        let obs = &observations[0];
        assert_eq!(obs.id, id);
        assert_eq!(obs.node_fqn, "src/main.rs::main");
        assert_eq!(obs.observation_text, "This function handles startup");
        assert_eq!(obs.agent_id, "claude");
        assert_eq!(obs.node_hash_at_write, "hash_abc");
        assert_eq!(obs.status, "active");
        assert!(obs.stale_reason.is_none());
        assert!(obs.written_at > 0);
    }

    #[test]
    fn test_mark_observations_stale_and_filter() {
        let conn = setup_db();

        // Write two observations for the same node
        let id1 = write_observation(
            &conn,
            "src/lib.rs::run",
            "First observation",
            "agent-1",
            "hash_1",
        )
        .unwrap();

        let _id2 = write_observation(
            &conn,
            "src/lib.rs::run",
            "Second observation",
            "agent-2",
            "hash_1",
        )
        .unwrap();

        // Mark all stale
        let staled = mark_observations_stale(&conn, "src/lib.rs::run", "node_modified").unwrap();
        assert_eq!(staled, 2);

        // Read without stale - should be empty
        let active = read_observations(&conn, "src/lib.rs::run", false).unwrap();
        assert_eq!(active.len(), 0);

        // Read with stale - should have both
        let all = read_observations(&conn, "src/lib.rs::run", true).unwrap();
        assert_eq!(all.len(), 2);

        // Verify stale_reason is set
        let obs = all.iter().find(|o| o.id == id1).unwrap();
        assert_eq!(obs.status, "stale");
        assert_eq!(obs.stale_reason.as_deref(), Some("node_modified"));
    }

    #[test]
    fn test_mark_stale_only_affects_active() {
        let conn = setup_db();

        // Write an observation and mark it stale
        write_observation(&conn, "src/a.rs::foo", "obs1", "agent", "hash").unwrap();
        mark_observations_stale(&conn, "src/a.rs::foo", "reason1").unwrap();

        // Write another observation (active)
        write_observation(&conn, "src/a.rs::foo", "obs2", "agent", "hash").unwrap();

        // Mark stale again - should only affect the new active one
        let staled = mark_observations_stale(&conn, "src/a.rs::foo", "reason2").unwrap();
        assert_eq!(staled, 1);

        // All should be stale now
        let all = read_observations(&conn, "src/a.rs::foo", true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|o| o.status == "stale"));
    }

    // -----------------------------------------------------------------------
    // Pruning tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_prune_stale_observations() {
        let conn = setup_db();

        // Insert a stale observation with an old timestamp directly
        conn.execute(
            "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason) \
             VALUES ('old-obs', 'src/a.rs::foo', 'old observation', 'agent', 'hash', 1000, 'stale', 'node_deleted')",
            [],
        )
        .unwrap();

        // Insert an active observation (should not be pruned)
        write_observation(&conn, "src/a.rs::foo", "active obs", "agent", "hash").unwrap();

        // Prune stale older than 0 days (all stale should be archived)
        let pruned = prune_stale_observations(&conn, Some(0)).unwrap();
        assert_eq!(pruned, 1);

        // Verify the old observation is now archived
        let status: String = conn
            .query_row(
                "SELECT status FROM observations WHERE id = 'old-obs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "archived");

        // Active observation should still be active
        let active = read_observations(&conn, "src/a.rs::foo", false).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, "active");
    }

    #[test]
    fn test_prune_without_days_archives_all_stale() {
        let conn = setup_db();

        // Insert two stale observations
        conn.execute(
            "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason) \
             VALUES ('stale-1', 'src/a.rs::foo', 'obs1', 'agent', 'hash', 1000, 'stale', 'reason')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason) \
             VALUES ('stale-2', 'src/b.rs::bar', 'obs2', 'agent', 'hash', 2000, 'stale', 'reason')",
            [],
        )
        .unwrap();

        let pruned = prune_stale_observations(&conn, None).unwrap();
        assert_eq!(pruned, 2);
    }

    // -----------------------------------------------------------------------
    // ADR CRUD tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_and_read_adr() {
        let conn = setup_db();

        let id = write_adr(
            &conn,
            "Use SQLite for storage",
            "We chose SQLite because it is embedded and requires no server.",
            "accepted",
            Some("src/store/mod.rs::StoreManager"),
        )
        .unwrap();

        assert!(!id.is_empty());

        let adrs = read_adrs(&conn, None, None).unwrap();
        assert_eq!(adrs.len(), 1);

        let adr = &adrs[0];
        assert_eq!(adr.id, id);
        assert_eq!(adr.title, "Use SQLite for storage");
        assert_eq!(
            adr.body,
            "We chose SQLite because it is embedded and requires no server."
        );
        assert_eq!(adr.status, "accepted");
        assert_eq!(
            adr.linked_fqn.as_deref(),
            Some("src/store/mod.rs::StoreManager")
        );
        assert!(adr.created_at > 0);
        assert_eq!(adr.created_at, adr.updated_at);
    }

    #[test]
    fn test_read_adrs_filter_by_status() {
        let conn = setup_db();

        write_adr(&conn, "ADR 1", "body1", "proposed", None).unwrap();
        write_adr(&conn, "ADR 2", "body2", "accepted", None).unwrap();
        write_adr(&conn, "ADR 3", "body3", "proposed", None).unwrap();

        let proposed = read_adrs(&conn, None, Some("proposed")).unwrap();
        assert_eq!(proposed.len(), 2);
        assert!(proposed.iter().all(|a| a.status == "proposed"));

        let accepted = read_adrs(&conn, None, Some("accepted")).unwrap();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].title, "ADR 2");
    }

    #[test]
    fn test_read_adrs_filter_by_linked_fqn() {
        let conn = setup_db();

        write_adr(&conn, "ADR A", "body", "accepted", Some("src/a.rs::foo")).unwrap();
        write_adr(&conn, "ADR B", "body", "accepted", Some("src/b.rs::bar")).unwrap();
        write_adr(&conn, "ADR C", "body", "accepted", None).unwrap();

        let linked = read_adrs(&conn, Some("src/a.rs::foo"), None).unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].title, "ADR A");
    }

    // -----------------------------------------------------------------------
    // Change note tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_and_read_change_notes() {
        let conn = setup_db();

        let id1 = write_change_note(&conn, "Refactored auth module").unwrap();
        assert!(!id1.is_empty());

        // Insert a second note with a slight delay to ensure ordering
        let id2 = write_change_note(&conn, "Added pagination support").unwrap();
        assert!(!id2.is_empty());
        assert_ne!(id1, id2);

        let notes = read_change_notes(&conn).unwrap();
        assert_eq!(notes.len(), 2);

        // Should be ordered by created_at DESC (most recent first)
        // Both were created in the same second, so order may vary,
        // but both should be present
        let texts: Vec<&str> = notes.iter().map(|n| n.text.as_str()).collect();
        assert!(texts.contains(&"Refactored auth module"));
        assert!(texts.contains(&"Added pagination support"));
    }

    #[test]
    fn test_change_notes_ordering() {
        let conn = setup_db();

        // Insert notes with explicit timestamps to test ordering
        conn.execute(
            "INSERT INTO change_notes (id, text, created_at) VALUES ('cn-1', 'First note', 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO change_notes (id, text, created_at) VALUES ('cn-2', 'Second note', 2000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO change_notes (id, text, created_at) VALUES ('cn-3', 'Third note', 3000)",
            [],
        )
        .unwrap();

        let notes = read_change_notes(&conn).unwrap();
        assert_eq!(notes.len(), 3);

        // DESC order: most recent first
        assert_eq!(notes[0].text, "Third note");
        assert_eq!(notes[1].text, "Second note");
        assert_eq!(notes[2].text, "First note");
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_observations_empty() {
        let conn = setup_db();
        let observations = read_observations(&conn, "nonexistent::fqn", false).unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn test_read_adrs_empty() {
        let conn = setup_db();
        let adrs = read_adrs(&conn, None, None).unwrap();
        assert!(adrs.is_empty());
    }

    #[test]
    fn test_read_change_notes_empty() {
        let conn = setup_db();
        let notes = read_change_notes(&conn).unwrap();
        assert!(notes.is_empty());
    }

    #[test]
    fn test_mark_stale_nonexistent_node_returns_zero() {
        let conn = setup_db();
        let staled = mark_observations_stale(&conn, "nonexistent::fqn", "reason").unwrap();
        assert_eq!(staled, 0);
    }

    #[test]
    fn test_prune_with_no_stale_returns_zero() {
        let conn = setup_db();
        write_observation(&conn, "src/a.rs::foo", "active obs", "agent", "hash").unwrap();
        let pruned = prune_stale_observations(&conn, Some(0)).unwrap();
        assert_eq!(pruned, 0);
    }
}
