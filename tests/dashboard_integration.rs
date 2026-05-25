//! Integration tests for the Token Savings Dashboard API.
//!
//! Tests the visualizer axum server's dashboard endpoints:
//! - `/api/savings/summary` - Cumulative totals JSON
//! - `/api/savings/timeseries` - Daily savings time-series JSON array
//! - `/api/savings/per-tool` - Per-tool breakdown JSON array
//! - `/dashboard` - HTML dashboard page
//!
//! Requirements: 7.1, 7.6

#![cfg(feature = "visualizer")]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cortex::mcp::savings_store;
use cortex::store::db::StoreManager;

/// Returns the path to the migrations directory.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// Creates a StoreManager with migrations applied in a temp directory.
fn setup_store() -> (Arc<StoreManager>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");

    // Apply migrations
    {
        let conn = store.write_conn();
        cortex::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");
    }

    (Arc::new(store), tmp)
}

/// Finds an available port by binding to port 0.
fn find_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind to port 0");
    listener.local_addr().unwrap().port()
}

/// Starts the visualizer server on the given port and returns when it's ready.
async fn start_server(store: Arc<StoreManager>, port: u16) {
    tokio::spawn(async move {
        cortex::cli::commands::visualizer::serve(store, port)
            .await
            .expect("server should start");
    });

    // Wait for the server to be ready
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    for _ in 0..50 {
        if client
            .get(format!("{}/health", base_url))
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Server did not become ready within 2.5 seconds");
}

// ---------------------------------------------------------------------------
// Tests with empty database
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_savings_summary_empty_db() {
    let (store, _tmp) = setup_store();
    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/summary", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    // Verify expected fields exist
    assert!(
        body.get("total_tokens_used").is_some(),
        "missing total_tokens_used"
    );
    assert!(
        body.get("total_tokens_saved").is_some(),
        "missing total_tokens_saved"
    );
    assert!(
        body.get("total_tool_calls").is_some(),
        "missing total_tool_calls"
    );
    assert!(
        body.get("naive_cost_estimate").is_some(),
        "missing naive_cost_estimate"
    );

    // Empty DB should return zeros
    assert_eq!(body["total_tokens_used"], 0);
    assert_eq!(body["total_tokens_saved"], 0);
    assert_eq!(body["total_tool_calls"], 0);
    assert_eq!(body["naive_cost_estimate"], 0.0);
}

#[tokio::test]
async fn test_savings_timeseries_empty_db() {
    let (store, _tmp) = setup_store();
    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/timeseries", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    // Should be an empty array
    assert!(body.is_array(), "timeseries should be a JSON array");
    assert_eq!(
        body.as_array().unwrap().len(),
        0,
        "empty DB should return empty array"
    );
}

#[tokio::test]
async fn test_savings_per_tool_empty_db() {
    let (store, _tmp) = setup_store();
    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/per-tool", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    // Should be an empty array
    assert!(body.is_array(), "per-tool should be a JSON array");
    assert_eq!(
        body.as_array().unwrap().len(),
        0,
        "empty DB should return empty array"
    );
}

#[tokio::test]
async fn test_dashboard_returns_html() {
    let (store, _tmp) = setup_store();
    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/dashboard", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "Expected text/html content-type, got: {}",
        content_type
    );

    let body = resp.text().await.expect("should read body as text");
    assert!(
        body.contains("Token Savings"),
        "Dashboard HTML should contain 'Token Savings'"
    );
}

// ---------------------------------------------------------------------------
// Tests with populated database
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_savings_summary_with_data() {
    let (store, _tmp) = setup_store();

    // Insert some savings records
    {
        let conn = store.write_conn();
        savings_store::record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o")
            .expect("record should succeed");
        savings_store::record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet")
            .expect("record should succeed");
        savings_store::record_savings(&conn, "trace_callers", 50, 500, "cursor", "gpt-4o")
            .expect("record should succeed");
    }

    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/summary", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    assert_eq!(body["total_tokens_used"], 350);
    assert_eq!(body["total_tokens_saved"], 2750);
    assert_eq!(body["total_tool_calls"], 3);

    // naive_cost_estimate should be > 0
    let cost = body["naive_cost_estimate"].as_f64().unwrap();
    assert!(cost > 0.0, "cost estimate should be positive with data");
}

#[tokio::test]
async fn test_savings_timeseries_with_data() {
    let (store, _tmp) = setup_store();

    // Insert savings records (they'll all be "today")
    {
        let conn = store.write_conn();
        savings_store::record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o")
            .expect("record should succeed");
        savings_store::record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet")
            .expect("record should succeed");
    }

    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/timeseries", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    assert!(body.is_array(), "timeseries should be a JSON array");
    let arr = body.as_array().unwrap();
    assert!(!arr.is_empty(), "should have at least one day entry");

    // Each entry should have date, tokens_saved, tokens_used, tool_calls
    let entry = &arr[0];
    assert!(entry.get("date").is_some(), "entry missing 'date'");
    assert!(
        entry.get("tokens_saved").is_some(),
        "entry missing 'tokens_saved'"
    );
    assert!(
        entry.get("tokens_used").is_some(),
        "entry missing 'tokens_used'"
    );
    assert!(
        entry.get("tool_calls").is_some(),
        "entry missing 'tool_calls'"
    );

    // Verify aggregated values for today
    let total_saved: u64 = arr
        .iter()
        .map(|e| e["tokens_saved"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(total_saved, 2250, "total tokens_saved should be 2250");
}

#[tokio::test]
async fn test_savings_per_tool_with_data() {
    let (store, _tmp) = setup_store();

    // Insert savings records for multiple tools
    {
        let conn = store.write_conn();
        savings_store::record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o")
            .expect("record should succeed");
        savings_store::record_savings(&conn, "search_symbols", 120, 800, "cursor", "gpt-4o")
            .expect("record should succeed");
        savings_store::record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet")
            .expect("record should succeed");
    }

    let port = find_available_port();
    start_server(store, port).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/api/savings/per-tool", port))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("should parse as JSON");

    assert!(body.is_array(), "per-tool should be a JSON array");
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2, "should have 2 tool entries");

    // Each entry should have tool_name, call_count, total_tokens_saved, average_tokens_saved
    for entry in arr {
        assert!(
            entry.get("tool_name").is_some(),
            "entry missing 'tool_name'"
        );
        assert!(
            entry.get("call_count").is_some(),
            "entry missing 'call_count'"
        );
        assert!(
            entry.get("total_tokens_saved").is_some(),
            "entry missing 'total_tokens_saved'"
        );
        assert!(
            entry.get("average_tokens_saved").is_some(),
            "entry missing 'average_tokens_saved'"
        );
    }

    // Verify search_symbols has 2 calls (ordered by total_saved DESC: search_symbols=1550 > ask=1500)
    let search_entry = arr
        .iter()
        .find(|e| e["tool_name"] == "search_symbols")
        .unwrap();
    assert_eq!(search_entry["call_count"], 2);
    assert_eq!(search_entry["total_tokens_saved"], 1550);
}
