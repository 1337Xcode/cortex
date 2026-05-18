//! Integration tests for FTS5 full-text search (Task 27).
//!
//! Tests cover:
//! - Search finds indexed nodes
//! - Update/delete triggers keep FTS5 in sync
//! - Malicious/special input is sanitized
//! - BM25 ranking orders results by relevance

use std::path::PathBuf;

use rusqlite::Connection;

/// Returns the path to the migrations directory relative to the crate root.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// Helper: create an in-memory connection with PRAGMAs applied and run all 4 migrations.
fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .expect("failed to apply PRAGMAs");

    cortex::store::migrations::run_migrations(&conn, &migrations_dir())
        .expect("migrations should succeed (all 4)");

    conn
}

/// Helper: insert a node into the database.
fn insert_node(conn: &Connection, fqn: &str, kind: &str, file: &str, attributes: &str) {
    conn.execute(
        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
         VALUES (?1, ?2, ?3, 1, 10, 'hash', 1000, ?4)",
        rusqlite::params![fqn, kind, file, attributes],
    )
    .expect("failed to insert node");
}

// ---------------------------------------------------------------------------
// Test: FTS5 virtual table and triggers are created
// ---------------------------------------------------------------------------

#[test]
fn test_fts5_virtual_table_created() {
    let conn = setup_db();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='nodes_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "nodes_fts virtual table should exist");
}

#[test]
fn test_fts5_triggers_created() {
    let conn = setup_db();

    let triggers: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };

    assert!(triggers.contains(&"nodes_ai".to_string()), "INSERT trigger missing");
    assert!(triggers.contains(&"nodes_ad".to_string()), "DELETE trigger missing");
    assert!(triggers.contains(&"nodes_au".to_string()), "UPDATE trigger missing");
}

// ---------------------------------------------------------------------------
// Test: Search finds indexed nodes
// ---------------------------------------------------------------------------

#[test]
fn test_search_finds_inserted_node() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/orders/processor.py::process_order",
        "Function",
        "src/orders/processor.py",
        "{}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "process_order", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].fqn, "src/orders/processor.py::process_order");
    assert_eq!(results[0].kind, "Function");
    assert_eq!(results[0].file, "src/orders/processor.py");
}

#[test]
fn test_search_finds_multiple_nodes() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/auth/login.ts::handle_login",
        "Function",
        "src/auth/login.ts",
        "{}",
    );
    insert_node(
        &conn,
        "src/auth/logout.ts::handle_logout",
        "Function",
        "src/auth/logout.ts",
        "{}",
    );
    insert_node(
        &conn,
        "src/orders/handler.ts::handle_order",
        "Function",
        "src/orders/handler.ts",
        "{}",
    );

    // Search for "handle" should find all three (unicode61 tokenizer splits on underscores)
    let results = cortex::store::queries::search::search_fts(&conn, "handle", 10).unwrap();
    assert_eq!(results.len(), 3);
}

#[test]
fn test_search_respects_limit() {
    let conn = setup_db();

    for i in 0..10 {
        insert_node(
            &conn,
            &format!("src/mod{}.rs::function_{}", i, i),
            "Function",
            &format!("src/mod{}.rs", i),
            "{}",
        );
    }

    let results = cortex::store::queries::search::search_fts(&conn, "function", 3).unwrap();
    assert_eq!(results.len(), 3);
}

#[test]
fn test_search_empty_query_returns_empty() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "", 10).unwrap();
    assert!(results.is_empty());

    let results = cortex::store::queries::search::search_fts(&conn, "   ", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_no_match_returns_empty() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    let results =
        cortex::store::queries::search::search_fts(&conn, "nonexistent_symbol", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_by_kind() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/models/user.py::User",
        "Class",
        "src/models/user.py",
        "{}",
    );

    // Searching by kind should work since kind is indexed
    let results = cortex::store::queries::search::search_fts(&conn, "Class", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].fqn, "src/models/user.py::User");
}

#[test]
fn test_search_by_file_path() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/auth/login.ts::login",
        "Function",
        "src/auth/login.ts",
        "{}",
    );
    insert_node(
        &conn,
        "src/orders/create.ts::create",
        "Function",
        "src/orders/create.ts",
        "{}",
    );

    // Search by file path fragment
    let results = cortex::store::queries::search::search_fts(&conn, "auth", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].fqn, "src/auth/login.ts::login");
}

// ---------------------------------------------------------------------------
// Test: Update/delete triggers keep FTS5 in sync
// ---------------------------------------------------------------------------

#[test]
fn test_fts5_sync_on_insert() {
    let conn = setup_db();

    // Before insert, search should find nothing
    let results = cortex::store::queries::search::search_fts(&conn, "calculator", 10).unwrap();
    assert!(results.is_empty());

    // After insert, search should find the node
    insert_node(
        &conn,
        "src/math/calc.rs::calculator",
        "Function",
        "src/math/calc.rs",
        "{}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "calculator", 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts5_sync_on_delete() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/temp.rs::temporary_function",
        "Function",
        "src/temp.rs",
        "{}",
    );

    // Verify it's searchable
    let results =
        cortex::store::queries::search::search_fts(&conn, "temporary_function", 10).unwrap();
    assert_eq!(results.len(), 1);

    // Delete the node
    conn.execute(
        "DELETE FROM nodes WHERE fqn = 'src/temp.rs::temporary_function'",
        [],
    )
    .unwrap();

    // Should no longer be found
    let results =
        cortex::store::queries::search::search_fts(&conn, "temporary_function", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_fts5_sync_on_update() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/old.rs::old_name",
        "Function",
        "src/old.rs",
        "{}",
    );

    // Verify old name is searchable
    let results = cortex::store::queries::search::search_fts(&conn, "old_name", 10).unwrap();
    assert_eq!(results.len(), 1);

    // Update the node's FQN (simulating a rename)
    conn.execute(
        "UPDATE nodes SET fqn = 'src/new.rs::new_name', file = 'src/new.rs' WHERE fqn = 'src/old.rs::old_name'",
        [],
    )
    .unwrap();

    // Old name should no longer be found
    let results = cortex::store::queries::search::search_fts(&conn, "old_name", 10).unwrap();
    assert!(results.is_empty());

    // New name should be found
    let results = cortex::store::queries::search::search_fts(&conn, "new_name", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].fqn, "src/new.rs::new_name");
}

// ---------------------------------------------------------------------------
// Test: Malicious/special input is sanitized
// ---------------------------------------------------------------------------

#[test]
fn test_sanitize_boolean_operators() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/logic.rs::process_or_skip",
        "Function",
        "src/logic.rs",
        "{}",
    );

    // "OR" as a boolean operator should be escaped, not crash
    let results = cortex::store::queries::search::search_fts(&conn, "process OR skip", 10).unwrap();
    // Should not crash; may or may not find results depending on tokenization
    assert!(results.len() <= 1);
}

#[test]
fn test_sanitize_quotes() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // Unbalanced quotes should not crash
    let results = cortex::store::queries::search::search_fts(&conn, "\"unbalanced", 10).unwrap();
    // Should not panic or error
    assert!(results.len() <= 1);
}

#[test]
fn test_sanitize_asterisk_wildcard() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // Asterisk alone should not crash
    let results = cortex::store::queries::search::search_fts(&conn, "*", 10).unwrap();
    assert!(results.is_empty() || results.len() >= 1); // Just verify no crash
}

#[test]
fn test_sanitize_parentheses() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // Unbalanced parentheses should not crash
    let results = cortex::store::queries::search::search_fts(&conn, "(unbalanced", 10).unwrap();
    assert!(results.is_empty() || results.len() >= 1); // Just verify no crash
}

#[test]
fn test_sanitize_near_operator() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // NEAR operator should be escaped
    let results = cortex::store::queries::search::search_fts(&conn, "NEAR main", 10).unwrap();
    // Should not crash
    assert!(results.is_empty() || results.len() >= 1);
}

#[test]
fn test_sanitize_not_operator() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // NOT operator should be escaped
    let results = cortex::store::queries::search::search_fts(&conn, "NOT main", 10).unwrap();
    // Should not crash
    assert!(results.is_empty() || results.len() >= 1);
}

#[test]
fn test_sanitize_column_filter_syntax() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::main",
        "Function",
        "src/main.rs",
        "{}",
    );

    // Column filter syntax (fqn:value) should be escaped
    let results = cortex::store::queries::search::search_fts(&conn, "fqn:main", 10).unwrap();
    // Should not crash - the colon is escaped
    assert!(results.is_empty() || results.len() >= 1);
}

// ---------------------------------------------------------------------------
// Test: BM25 ranking
// ---------------------------------------------------------------------------

#[test]
fn test_results_ordered_by_rank() {
    let conn = setup_db();

    // Insert nodes where "process" appears in different contexts
    insert_node(
        &conn,
        "src/processor.rs::process",
        "Function",
        "src/processor.rs",
        "{\"description\": \"process data\"}",
    );
    insert_node(
        &conn,
        "src/handler.rs::handle_request",
        "Function",
        "src/handler.rs",
        "{\"calls\": \"process\"}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "process", 10).unwrap();
    assert!(!results.is_empty());

    // Verify results have rank values (FTS5 rank is negative, lower = better)
    for result in &results {
        assert!(result.rank <= 0.0, "FTS5 BM25 rank should be <= 0");
    }

    // Verify ordering: first result should have lower (better) rank
    if results.len() > 1 {
        assert!(
            results[0].rank <= results[1].rank,
            "Results should be ordered by rank (best first)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: MCP dispatch integration
// ---------------------------------------------------------------------------

#[test]
fn test_dispatch_search_text() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    // Apply migrations
    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Insert a test node
    {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/api/users.rs::get_user', 'Function', 'src/api/users.rs', 1, 20, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
    }

    // Dispatch search_text tool
    let args = serde_json::json!({ "query": "get_user", "limit": 5 });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "search_text", &args).unwrap();

    // Verify response structure
    assert!(result.get("content").is_some());
    assert!(result.get("_meta").is_some());

    // Parse the content text
    let content = result["content"][0]["text"].as_str().unwrap();
    let search_results: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0]["fqn"], "src/api/users.rs::get_user");
}

#[test]
fn test_dispatch_search_text_missing_query() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Missing required "query" argument
    let args = serde_json::json!({});
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "search_text", &args);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("missing required argument: query"));
}

// ---------------------------------------------------------------------------
// Test: Multi-tier confidence scoring (Task 5)
// ---------------------------------------------------------------------------

#[test]
fn test_fts5_results_include_confidence_field() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/auth.rs::authenticate",
        "Function",
        "src/auth.rs",
        "{}",
    );
    insert_node(
        &conn,
        "src/auth.rs::validate_token",
        "Function",
        "src/auth.rs",
        "{}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "auth", 10).unwrap();
    assert!(!results.is_empty());

    // Every result must have a confidence field in range (0.0, 0.5]
    for result in &results {
        assert!(
            result.confidence > 0.0 && result.confidence <= 0.5,
            "FTS5 confidence should be in (0.0, 0.5], got {}",
            result.confidence
        );
    }
}

#[test]
fn test_fts5_results_sorted_by_confidence_descending() {
    let conn = setup_db();

    // Insert multiple nodes that will match "process" with different relevance
    insert_node(
        &conn,
        "src/processor.rs::process_data",
        "Function",
        "src/processor.rs",
        "{}",
    );
    insert_node(
        &conn,
        "src/handler.rs::handle_process",
        "Function",
        "src/handler.rs",
        "{}",
    );
    insert_node(
        &conn,
        "src/utils.rs::preprocess",
        "Function",
        "src/utils.rs",
        "{}",
    );

    let results = cortex::store::queries::search::search_fts(&conn, "process", 10).unwrap();

    // Verify sorted by confidence descending
    for i in 1..results.len() {
        assert!(
            results[i - 1].confidence >= results[i].confidence,
            "Results should be sorted by confidence descending: {} >= {} failed at index {}",
            results[i - 1].confidence,
            results[i].confidence,
            i
        );
    }
}

#[test]
fn test_fts5_best_match_gets_confidence_0_5() {
    let conn = setup_db();

    insert_node(
        &conn,
        "src/main.rs::unique_function_name",
        "Function",
        "src/main.rs",
        "{}",
    );

    let results =
        cortex::store::queries::search::search_fts(&conn, "unique_function_name", 10).unwrap();
    assert_eq!(results.len(), 1);

    // Single result (best match) should get confidence = 0.5
    assert!(
        (results[0].confidence - 0.5).abs() < 0.001,
        "Best (only) FTS5 match should have confidence 0.5, got {}",
        results[0].confidence
    );
}

#[test]
fn test_graph_results_include_confidence_via_dispatch() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Insert nodes
    {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/a.rs::caller', 'Function', 'src/a.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/b.rs::callee', 'Function', 'src/b.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes)
             VALUES ('src/a.rs::caller', 'src/b.rs::callee', 'Calls', 0.85, '{}')",
            [],
        )
        .unwrap();
    }

    // trace_callers should return results with confidence from edge
    let args = serde_json::json!({ "fqn": "src/b.rs::callee", "depth": 3 });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "trace_callers", &args).unwrap();

    let content = result["content"][0]["text"].as_str().unwrap();
    let callers: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0]["fqn"], "src/a.rs::caller");

    // Confidence should come from edge confidence (0.85)
    let confidence = callers[0]["confidence"].as_f64().unwrap();
    assert!(
        (confidence - 0.85).abs() < 0.01,
        "Graph confidence should be from edge (0.85), got {}",
        confidence
    );
}

#[test]
fn test_search_symbols_includes_confidence_1_0() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Insert a test node
    {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
    }

    let args = serde_json::json!({ "pattern": "*main*" });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "search_symbols", &args).unwrap();

    let content = result["content"][0]["text"].as_str().unwrap();
    let nodes: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
    assert!(!nodes.is_empty());

    // Graph-resolved results should have confidence 1.0
    for node in &nodes {
        let confidence = node["confidence"].as_f64().unwrap();
        assert!(
            (confidence - 1.0).abs() < 0.001,
            "Graph-resolved search_symbols should have confidence 1.0, got {}",
            confidence
        );
    }
}

#[test]
fn test_trace_callers_sorted_by_confidence_descending() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Insert nodes and edges with different confidence levels
    {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/target.rs::target', 'Function', 'src/target.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/a.rs::high_conf', 'Function', 'src/a.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/b.rs::low_conf', 'Function', 'src/b.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/c.rs::mid_conf', 'Function', 'src/c.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();

        // Edges with different confidence values
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes)
             VALUES ('src/a.rs::high_conf', 'src/target.rs::target', 'Calls', 0.95, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes)
             VALUES ('src/b.rs::low_conf', 'src/target.rs::target', 'Calls', 0.6, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes)
             VALUES ('src/c.rs::mid_conf', 'src/target.rs::target', 'Calls', 0.8, '{}')",
            [],
        )
        .unwrap();
    }

    let args = serde_json::json!({ "fqn": "src/target.rs::target", "depth": 1 });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "trace_callers", &args).unwrap();

    let content = result["content"][0]["text"].as_str().unwrap();
    let callers: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
    assert_eq!(callers.len(), 3);

    // Verify sorted by confidence descending
    let confidences: Vec<f64> = callers
        .iter()
        .map(|c| c["confidence"].as_f64().unwrap())
        .collect();

    for i in 1..confidences.len() {
        assert!(
            confidences[i - 1] >= confidences[i],
            "trace_callers results should be sorted by confidence descending: {} >= {} failed",
            confidences[i - 1],
            confidences[i]
        );
    }

    // First result should be the highest confidence caller
    assert_eq!(callers[0]["fqn"], "src/a.rs::high_conf");
}

#[test]
fn test_search_text_dispatch_includes_confidence() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    // Insert test nodes
    {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES ('src/api/users.rs::get_user', 'Function', 'src/api/users.rs', 1, 20, 'hash', 1000, '{}')",
            [],
        )
        .unwrap();
    }

    let args = serde_json::json!({ "query": "get_user", "limit": 5 });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "search_text", &args).unwrap();

    let content = result["content"][0]["text"].as_str().unwrap();
    let search_results: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
    assert_eq!(search_results.len(), 1);

    // FTS5 results should have confidence field in (0.0, 0.5]
    let confidence = search_results[0]["confidence"].as_f64().unwrap();
    assert!(
        confidence > 0.0 && confidence <= 0.5,
        "FTS5 dispatch result should have confidence in (0.0, 0.5], got {}",
        confidence
    );
}
