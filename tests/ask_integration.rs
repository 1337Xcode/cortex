//! Integration tests for MCP ask round-trip.
//!
//! Validates:
//! - Sending a JSON-RPC `ask` request through the full dispatch pipeline
//! - Response matches `AskResponse` schema (valid JSON, flat array, sorted by relevance)
//! - Empty question, unicode question, and normal question all produce valid JSON
//! - Results are sorted by relevance_score descending
//!
//! _Requirements: 11.1, 11.2, 11.3_

use std::path::PathBuf;

use cortex::mcp::ask::AskResponse;
use cortex::mcp::dispatch::dispatch_tool;
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
/// the cortex source tree indexed (so the ask engine has real data to work with).
fn setup_indexed_store() -> (StoreManager, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");

    // Apply migrations
    {
        let conn = store.write_conn();
        migrations::run_migrations(&conn, &migrations_dir()).expect("migrations should succeed");
    }

    // Index cortex's own source tree so the ask engine has real data
    let repo_root = cortex_src_dir();
    cortex::indexer::pipeline::index_repository(&repo_root, &store)
        .expect("index_repository should succeed");

    (store, tmp)
}

/// Helper: dispatch an `ask` tool call and return the raw JSON response value.
fn dispatch_ask(store: &StoreManager, question: &str) -> serde_json::Value {
    let args = serde_json::json!({"question": question});
    dispatch_tool(store, "ask", &args).expect("dispatch_tool for 'ask' should not error")
}

/// Helper: extract the ask response text from the dispatch result and parse it
/// as an `AskResponse`.
fn parse_ask_response(dispatch_result: &serde_json::Value) -> AskResponse {
    // The dispatch result wraps the tool output in a ToolCallResult structure:
    // { "content": [{"type": "text", "text": "<json>"}], "_meta": {...} }
    let content_array = dispatch_result
        .get("content")
        .expect("response should have 'content'")
        .as_array()
        .expect("'content' should be an array");

    assert!(
        !content_array.is_empty(),
        "content array should not be empty"
    );

    let text = content_array[0]
        .get("text")
        .expect("content item should have 'text'")
        .as_str()
        .expect("'text' should be a string");

    // The text should be valid JSON
    let parsed: AskResponse =
        serde_json::from_str(text).expect("ask response text should deserialize to AskResponse");

    parsed
}

// ---------------------------------------------------------------------------
// Test: Normal question returns valid AskResponse with results
// ---------------------------------------------------------------------------

#[test]
fn test_ask_normal_question_returns_valid_response() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "What does dispatch_tool do?");
    let response = parse_ask_response(&result);

    // Summary should be present and well-formed
    // total_results is usize so always >= 0; just verify it's reasonable
    assert!(
        response.summary.total_results <= 1000,
        "total_results should be reasonable, got {}",
        response.summary.total_results
    );
    assert!(
        response.summary.budget_used_percent >= 0.0
            && response.summary.budget_used_percent <= 100.0,
        "budget_used_percent should be in [0, 100], got {}",
        response.summary.budget_used_percent
    );
    assert!(
        !response.summary.intent_detected.is_empty(),
        "intent_detected should not be empty"
    );
    assert!(
        !response.summary.query_terms_extracted.is_empty(),
        "query_terms_extracted should not be empty for a normal question"
    );

    // If there are results, verify they have all required fields
    if !response.results.is_empty() {
        for item in &response.results {
            assert!(!item.fqn.is_empty(), "fqn should not be empty");
            assert!(!item.kind.is_empty(), "kind should not be empty");
            assert!(!item.file.is_empty(), "file should not be empty");
            assert!(!item.why.is_empty(), "why should not be empty");
            assert!(
                item.relevance_score >= 0.0,
                "relevance_score should be non-negative"
            );
            assert!(item.token_cost > 0, "token_cost should be positive");
            assert!(
                item.naive_cost_estimate >= 0.0,
                "naive_cost_estimate should be non-negative"
            );
            assert!(
                item.coverage >= 0.0 && item.coverage <= 1.0,
                "coverage should be in [0.0, 1.0], got {}",
                item.coverage
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test: Results are sorted by relevance_score descending
// ---------------------------------------------------------------------------

#[test]
fn test_ask_results_sorted_by_relevance_descending() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "What does dispatch_tool do?");
    let response = parse_ask_response(&result);

    // Verify results are sorted by relevance_score descending
    for window in response.results.windows(2) {
        assert!(
            window[0].relevance_score >= window[1].relevance_score,
            "Results should be sorted descending by relevance_score: {} >= {} failed (fqn: {} vs {})",
            window[0].relevance_score,
            window[1].relevance_score,
            window[0].fqn,
            window[1].fqn,
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Empty question returns valid JSON with empty results
// ---------------------------------------------------------------------------

#[test]
fn test_ask_empty_question_returns_valid_json() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "");
    let response = parse_ask_response(&result);

    // Empty question should produce zero results (no query terms to search for)
    assert_eq!(
        response.summary.total_results, 0,
        "Empty question should return 0 results, got {}",
        response.summary.total_results
    );
    assert!(
        response.results.is_empty(),
        "Empty question should return empty results array"
    );
    assert_eq!(
        response.summary.budget_used_percent, 0.0,
        "Empty question should use 0% budget"
    );
}

// ---------------------------------------------------------------------------
// Test: Unicode question returns valid JSON
// ---------------------------------------------------------------------------

#[test]
fn test_ask_unicode_question_returns_valid_json() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "什么是 StoreManager？");

    // The response should always be valid JSON regardless of input language
    let response = parse_ask_response(&result);

    // Summary should be well-formed
    assert!(
        response.summary.budget_used_percent >= 0.0
            && response.summary.budget_used_percent <= 100.0,
        "budget_used_percent should be in [0, 100]"
    );

    // If StoreManager is found in the index, we should get results
    // (the term "StoreManager" is CamelCase and should be extracted)
    // Either way, the response must be valid JSON - which we already verified by parsing
    assert!(
        response.summary.total_results <= 1000,
        "total_results should be reasonable, got {}",
        response.summary.total_results
    );

    // Results (if any) should still be sorted
    for window in response.results.windows(2) {
        assert!(
            window[0].relevance_score >= window[1].relevance_score,
            "Unicode question results should still be sorted descending"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Question with backtick terms extracts the term correctly
// ---------------------------------------------------------------------------

#[test]
fn test_ask_backtick_terms_extracted() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "How does `search_symbols` work?");
    let response = parse_ask_response(&result);

    // The backtick-quoted term "search_symbols" should be extracted
    assert!(
        response
            .summary
            .query_terms_extracted
            .iter()
            .any(|t| t == "search_symbols"),
        "Expected 'search_symbols' in query_terms_extracted, got: {:?}",
        response.summary.query_terms_extracted
    );

    // Response should be valid and sorted
    for window in response.results.windows(2) {
        assert!(
            window[0].relevance_score >= window[1].relevance_score,
            "Results should be sorted descending by relevance_score"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: Response is always valid JSON (round-trip serialization)
// ---------------------------------------------------------------------------

#[test]
fn test_ask_response_round_trip_serialization() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "What does dispatch_tool do?");
    let response = parse_ask_response(&result);

    // Serialize back to JSON and deserialize again - should produce equal object
    let serialized = serde_json::to_string(&response).expect("should serialize to JSON");
    let deserialized: AskResponse =
        serde_json::from_str(&serialized).expect("should deserialize back from JSON");

    assert_eq!(
        response, deserialized,
        "Round-trip serialization should produce equal AskResponse"
    );
}

// ---------------------------------------------------------------------------
// Test: Token budget is respected in results
// ---------------------------------------------------------------------------

#[test]
fn test_ask_respects_token_budget() {
    let (store, _tmp) = setup_indexed_store();

    // Use a small token budget to force truncation
    let args = serde_json::json!({"question": "What does dispatch_tool do?", "token_budget": 500});
    let result =
        dispatch_tool(&store, "ask", &args).expect("dispatch_tool for 'ask' should not error");
    let response = parse_ask_response(&result);

    // The total token cost should not exceed the budget
    // (except for the guaranteed top-1 result which is always included)
    if response.results.len() > 1 {
        let total_cost: usize = response.results.iter().map(|r| r.token_cost).sum();
        assert!(
            total_cost <= 500 + response.results[0].token_cost,
            "Total token cost {} should not greatly exceed budget 500 (top-1 exception: {})",
            total_cost,
            response.results[0].token_cost
        );
    }

    assert_eq!(
        response.summary.total_token_cost,
        response.results.iter().map(|r| r.token_cost).sum::<usize>(),
        "summary.total_token_cost should equal sum of result token_costs"
    );
}

// ---------------------------------------------------------------------------
// Test: dispatch_tool wraps ask response with _meta
// ---------------------------------------------------------------------------

#[test]
fn test_ask_dispatch_includes_meta() {
    let (store, _tmp) = setup_indexed_store();

    let result = dispatch_ask(&store, "What is StoreManager?");

    // Verify the dispatch wrapper includes _meta
    assert!(
        result.get("_meta").is_some(),
        "dispatch response should include '_meta'"
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
