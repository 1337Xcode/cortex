//! Integration tests for Phase 1 migration files (0001-0003).
//!
//! Verifies that running all migrations creates the expected tables, indexes,
//! and constraints in the SQLite database.

use std::path::PathBuf;

use rusqlite::Connection;

/// Returns the path to the migrations directory relative to the crate root.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// Helper: create an in-memory connection with PRAGMAs applied and run all migrations.
fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .expect("failed to apply PRAGMAs");

    // Use the run_migrations function from the crate.
    cortex::store::migrations::run_migrations(&conn, &migrations_dir())
        .expect("migrations should succeed");

    conn
}

/// Returns all table names from sqlite_master.
fn get_tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

/// Returns all index names from sqlite_master.
fn get_indexes(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

#[test]
fn test_all_expected_tables_exist() {
    let conn = setup_db();
    let tables = get_tables(&conn);

    let expected_tables = [
        "architectural_decisions",
        "bundle_metadata",
        "change_notes",
        "edges",
        "file_snapshots",
        "nodes",
        "observations",
        "sbom_entries",
        "schema_versions",
        "security_findings",
        "taint_paths",
    ];

    for table in &expected_tables {
        assert!(
            tables.contains(&table.to_string()),
            "Expected table '{}' not found. Tables: {:?}",
            table,
            tables
        );
    }
}

#[test]
fn test_all_expected_indexes_exist() {
    let conn = setup_db();
    let indexes = get_indexes(&conn);

    let expected_indexes = [
        "idx_nodes_file",
        "idx_nodes_kind",
        "idx_edges_source",
        "idx_edges_target",
        "idx_edges_kind",
        "idx_edges_source_kind",
        "idx_findings_node",
        "idx_findings_kind",
        "idx_taint_source",
        "idx_taint_sink",
        "idx_obs_node",
        "idx_obs_status",
    ];

    for index in &expected_indexes {
        assert!(
            indexes.contains(&index.to_string()),
            "Expected index '{}' not found. Indexes: {:?}",
            index,
            indexes
        );
    }
}

#[test]
fn test_nodes_table_columns() {
    let conn = setup_db();

    // Insert a valid node.
    conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at)
         VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'abc123', 1000)",
        [],
    )
    .expect("valid node insert should succeed");

    // Verify attributes defaults to '{}'.
    let attrs: String = conn
        .query_row(
            "SELECT attributes FROM nodes WHERE fqn = 'src/main.rs::main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attrs, "{}");
}

#[test]
fn test_nodes_kind_constraint() {
    let conn = setup_db();

    // Insert with invalid kind should fail.
    let result = conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at)
         VALUES ('test::foo', 'InvalidKind', 'test.rs', 1, 5, 'hash', 1000)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting node with invalid kind should fail"
    );
}

#[test]
fn test_edges_confidence_constraint() {
    let conn = setup_db();

    // Insert a valid node first (needed for foreign key).
    conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at)
         VALUES ('src/a.rs::foo', 'Function', 'src/a.rs', 1, 5, 'hash1', 1000)",
        [],
    )
    .unwrap();

    // Insert edge with confidence > 1.0 should fail.
    let result = conn.execute(
        "INSERT INTO edges (source_fqn, target_fqn, kind, confidence)
         VALUES ('src/a.rs::foo', 'src/b.rs::bar', 'Calls', 1.5)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting edge with confidence > 1.0 should fail"
    );

    // Insert edge with confidence < 0.0 should fail.
    let result = conn.execute(
        "INSERT INTO edges (source_fqn, target_fqn, kind, confidence)
         VALUES ('src/a.rs::foo', 'src/b.rs::bar', 'Calls', -0.1)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting edge with confidence < 0.0 should fail"
    );
}

#[test]
fn test_edges_kind_constraint() {
    let conn = setup_db();

    // Insert a valid node first.
    conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at)
         VALUES ('src/a.rs::foo', 'Function', 'src/a.rs', 1, 5, 'hash1', 1000)",
        [],
    )
    .unwrap();

    // Insert edge with invalid kind should fail.
    let result = conn.execute(
        "INSERT INTO edges (source_fqn, target_fqn, kind, confidence)
         VALUES ('src/a.rs::foo', 'src/b.rs::bar', 'InvalidEdge', 0.9)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting edge with invalid kind should fail"
    );
}

#[test]
fn test_edges_foreign_key_cascade() {
    let conn = setup_db();

    // Insert a node and an edge referencing it.
    conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at)
         VALUES ('src/a.rs::foo', 'Function', 'src/a.rs', 1, 5, 'hash1', 1000)",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO edges (source_fqn, target_fqn, kind, confidence)
         VALUES ('src/a.rs::foo', 'src/b.rs::bar', 'Calls', 1.0)",
        [],
    )
    .unwrap();

    // Delete the node - edge should be cascade-deleted.
    conn.execute("DELETE FROM nodes WHERE fqn = 'src/a.rs::foo'", [])
        .unwrap();

    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
        .unwrap();
    assert_eq!(edge_count, 0, "Edge should be cascade-deleted with node");
}

#[test]
fn test_observations_status_constraint() {
    let conn = setup_db();

    // Insert with invalid status should fail.
    let result = conn.execute(
        "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status)
         VALUES ('obs1', 'src/a.rs::foo', 'test', 'agent1', 'hash', 1000, 'invalid_status')",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting observation with invalid status should fail"
    );

    // Insert with valid status should succeed.
    conn.execute(
        "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status)
         VALUES ('obs1', 'src/a.rs::foo', 'test observation', 'agent1', 'hash', 1000, 'active')",
        [],
    )
    .expect("valid observation insert should succeed");
}

#[test]
fn test_architectural_decisions_status_constraint() {
    let conn = setup_db();

    // Insert with invalid status should fail.
    let result = conn.execute(
        "INSERT INTO architectural_decisions (id, title, body, status, created_at, updated_at)
         VALUES ('adr1', 'Test', 'Body', 'invalid', 1000, 1000)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting ADR with invalid status should fail"
    );

    // Insert with valid status should succeed.
    conn.execute(
        "INSERT INTO architectural_decisions (id, title, body, status, created_at, updated_at)
         VALUES ('adr1', 'Test ADR', 'Decision body', 'proposed', 1000, 1000)",
        [],
    )
    .expect("valid ADR insert should succeed");
}

#[test]
fn test_bundle_metadata_singleton_constraint() {
    let conn = setup_db();

    // bundle_metadata should already have the seed row.
    let format_version: i64 = conn
        .query_row(
            "SELECT format_version FROM bundle_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("bundle_metadata seed row should exist");
    assert_eq!(format_version, 1);

    // Inserting a second row with id != 1 should fail due to CHECK(id = 1).
    let result = conn.execute(
        "INSERT INTO bundle_metadata (id, format_version) VALUES (2, 1)",
        [],
    );
    assert!(
        result.is_err(),
        "Inserting bundle_metadata with id != 1 should fail"
    );
}

#[test]
fn test_file_snapshots_columns() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO file_snapshots (file, file_hash, node_count, indexed_at)
         VALUES ('src/main.rs', 'hash123', 5, 1000)",
        [],
    )
    .expect("valid file_snapshot insert should succeed");

    // Verify node_count defaults to 0 when not specified.
    conn.execute(
        "INSERT INTO file_snapshots (file, file_hash, indexed_at)
         VALUES ('src/lib.rs', 'hash456', 2000)",
        [],
    )
    .expect("file_snapshot insert without node_count should use default");

    let node_count: i64 = conn
        .query_row(
            "SELECT node_count FROM file_snapshots WHERE file = 'src/lib.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(node_count, 0);
}

#[test]
fn test_migrations_are_idempotent() {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .expect("failed to apply PRAGMAs");

    let dir = migrations_dir();

    // First run.
    let applied = cortex::store::migrations::run_migrations(&conn, &dir)
        .expect("first migration run should succeed");
    assert_eq!(applied.len(), 5);

    // Second run - should apply nothing.
    let applied = cortex::store::migrations::run_migrations(&conn, &dir)
        .expect("second migration run should succeed");
    assert_eq!(applied.len(), 0);
}
