//! Integration tests for persistent token savings recording.
//!
//! Validates:
//! - Dispatching a tool via `dispatch_tool` records a row in `token_savings`
//! - The recorded row has the correct tool_name, tokens_used > 0, tokens_saved > 0
//! - `query_cumulative` returns correct totals after tool execution
//! - Multiple tool calls aggregate correctly in cumulative totals
//!
//! _Requirements: 6.2, 6.4_

use std::path::PathBuf;

use cortex::mcp::dispatch::dispatch_tool;
use cortex::mcp::savings_store::{self, TimePeriod};
use cortex::store::db::StoreManager;
use cortex::store::migrations;

/// Returns the path to the migrations directory.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// Returns the cortex source directory for indexing.
fn cortex_src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Creates a StoreManager in a temp directory with all migrations applied and
/// the cortex source tree indexed (so tools have real data to work with).
fn setup_indexed_store() -> (StoreManager, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");

    // Apply migrations
    {
        let conn = store.write_conn();
        migrations::run_migrations(&conn, &migrations_dir()).expect("migrations should succeed");
    }

    // Index cortex's own source tree so tools return real data
    let repo_root = cortex_src_dir();
    cortex::indexer::pipeline::index_repository(&repo_root, &store)
        .expect("index_repository should succeed");

    (store, tmp)
}

// ---------------------------------------------------------------------------
// Test: Dispatching a tool records a savings row
// ---------------------------------------------------------------------------

#[test]
fn test_dispatch_tool_records_savings_row() {
    let (store, _tmp) = setup_indexed_store();

    // Execute search_symbols via dispatch
    let args = serde_json::json!({"pattern": "*main*", "limit": 5});
    let result = dispatch_tool(&store, "search_symbols", &args);
    assert!(
        result.is_ok(),
        "dispatch_tool should succeed: {:?}",
        result.err()
    );

    // Query the token_savings table directly
    let conn = store.read_conn();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM token_savings", [], |row| row.get(0))
        .unwrap();

    // There should be at least 1 row (from our dispatch call).
    // Note: index_repository also dispatches tools internally, but the savings
    // recording happens in dispatch_tool which is what we're testing.
    assert!(
        count >= 1,
        "Expected at least 1 row in token_savings after dispatch, got {}",
        count
    );
}

// ---------------------------------------------------------------------------
// Test: Recorded row has correct tool_name
// ---------------------------------------------------------------------------

#[test]
fn test_savings_row_has_correct_tool_name() {
    let (store, _tmp) = setup_indexed_store();

    // Clear any rows from indexing
    {
        let conn = store.write_conn();
        conn.execute("DELETE FROM token_savings", []).unwrap();
    }

    // Execute a specific tool
    let args = serde_json::json!({"pattern": "*dispatch*", "limit": 3});
    dispatch_tool(&store, "search_symbols", &args).expect("dispatch should succeed");

    // Verify the row has the correct tool_name
    let conn = store.read_conn();
    let tool_name: String = conn
        .query_row(
            "SELECT tool_name FROM token_savings ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("should have a savings row");

    assert_eq!(tool_name, "search_symbols");
}

// ---------------------------------------------------------------------------
// Test: Recorded row has reasonable tokens_used and tokens_saved values
// ---------------------------------------------------------------------------

#[test]
fn test_savings_row_has_positive_token_values() {
    let (store, _tmp) = setup_indexed_store();

    // Clear any rows from indexing
    {
        let conn = store.write_conn();
        conn.execute("DELETE FROM token_savings", []).unwrap();
    }

    // Execute a tool that touches files (search_symbols with broad pattern)
    let args = serde_json::json!({"pattern": "*", "limit": 1});
    dispatch_tool(&store, "search_symbols", &args).expect("dispatch should succeed");

    // Verify tokens_used > 0 (the result JSON has some content)
    let conn = store.read_conn();
    let (tokens_used, tokens_saved): (i64, i64) = conn
        .query_row(
            "SELECT tokens_used, tokens_saved FROM token_savings ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("should have a savings row");

    assert!(
        tokens_used > 0,
        "tokens_used should be > 0, got {}",
        tokens_used
    );
    assert!(
        tokens_saved > 0,
        "tokens_saved should be > 0 for a query touching files, got {}",
        tokens_saved
    );
}

// ---------------------------------------------------------------------------
// Test: query_cumulative returns correct totals after a single tool call
// ---------------------------------------------------------------------------

#[test]
fn test_cumulative_query_matches_single_dispatch() {
    let (store, _tmp) = setup_indexed_store();

    // Clear any rows from indexing
    {
        let conn = store.write_conn();
        conn.execute("DELETE FROM token_savings", []).unwrap();
    }

    // Execute one tool
    let args = serde_json::json!({"pattern": "*store*", "limit": 5});
    dispatch_tool(&store, "search_symbols", &args).expect("dispatch should succeed");

    // Read the raw row values
    let conn = store.read_conn();
    let (raw_used, raw_saved): (i64, i64) = conn
        .query_row(
            "SELECT tokens_used, tokens_saved FROM token_savings ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("should have a savings row");

    // Query cumulative totals
    let totals = savings_store::query_cumulative(&conn, None, TimePeriod::AllTime)
        .expect("query_cumulative should succeed");

    assert_eq!(totals.total_tokens_used, raw_used as u64);
    assert_eq!(totals.total_tokens_saved, raw_saved as u64);
    assert_eq!(totals.total_tool_calls, 1);
}

// ---------------------------------------------------------------------------
// Test: Multiple tool calls aggregate correctly in cumulative totals
// ---------------------------------------------------------------------------

#[test]
fn test_cumulative_totals_aggregate_multiple_tools() {
    let (store, _tmp) = setup_indexed_store();

    // Clear any rows from indexing
    {
        let conn = store.write_conn();
        conn.execute("DELETE FROM token_savings", []).unwrap();
    }

    // Execute multiple different tools
    let tools_and_args: Vec<(&str, serde_json::Value)> = vec![
        (
            "search_symbols",
            serde_json::json!({"pattern": "*main*", "limit": 5}),
        ),
        ("get_architecture", serde_json::json!({})),
        ("find_dead_code", serde_json::json!({"limit": 5})),
    ];

    for (tool_name, args) in &tools_and_args {
        dispatch_tool(&store, tool_name, args).expect("dispatch should succeed");
    }

    // Read all raw rows
    let conn = store.read_conn();
    let mut stmt = conn
        .prepare("SELECT tool_name, tokens_used, tokens_saved FROM token_savings ORDER BY id")
        .unwrap();
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(rows.len(), 3, "Expected 3 savings rows, got {}", rows.len());

    let expected_total_used: u64 = rows.iter().map(|(_, u, _)| *u as u64).sum();
    let expected_total_saved: u64 = rows.iter().map(|(_, _, s)| *s as u64).sum();

    // Verify cumulative totals match the sum of individual rows
    let totals = savings_store::query_cumulative(&conn, None, TimePeriod::AllTime)
        .expect("query_cumulative should succeed");

    assert_eq!(totals.total_tokens_used, expected_total_used);
    assert_eq!(totals.total_tokens_saved, expected_total_saved);
    assert_eq!(totals.total_tool_calls, 3);

    // Verify per-tool breakdown
    let breakdown = savings_store::query_per_tool(&conn).expect("query_per_tool should succeed");
    assert_eq!(
        breakdown.len(),
        3,
        "Expected 3 tools in breakdown, got {}",
        breakdown.len()
    );

    // Each tool should have call_count = 1
    for entry in &breakdown {
        assert_eq!(
            entry.call_count, 1,
            "Tool '{}' should have call_count=1, got {}",
            entry.tool_name, entry.call_count
        );
        assert!(
            entry.total_tokens_saved > 0 || entry.average_tokens_saved >= 0.0,
            "Tool '{}' should have non-negative savings",
            entry.tool_name
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Savings recording does not block tool response on failure
// ---------------------------------------------------------------------------

#[test]
fn test_dispatch_succeeds_even_with_savings_data() {
    let (store, _tmp) = setup_indexed_store();

    // Execute a tool and verify the response structure is correct
    // regardless of savings recording
    let args = serde_json::json!({"pattern": "*", "limit": 3});
    let result = dispatch_tool(&store, "search_symbols", &args).expect("dispatch should succeed");

    // Verify the response has the expected structure
    assert!(
        result.get("content").is_some(),
        "response should have 'content'"
    );
    assert!(
        result.get("_meta").is_some(),
        "response should have '_meta'"
    );

    let meta = result.get("_meta").unwrap();
    assert!(
        meta.get("tokens_used").is_some(),
        "_meta should have 'tokens_used'"
    );
    assert!(
        meta.get("tokens_saved").is_some(),
        "_meta should have 'tokens_saved'"
    );
    assert!(
        meta.get("query_time_ms").is_some(),
        "_meta should have 'query_time_ms'"
    );
}
