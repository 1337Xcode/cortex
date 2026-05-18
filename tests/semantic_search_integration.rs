//! Integration test for semantic search functionality.
//!
//! Tests that semantic search returns relevant results when embeddings are stored.
//! Uses mock embeddings (not the real ONNX model) to test the search logic.

use std::path::Path;

/// Helper to create a StoreManager with migrations applied.
fn setup_store() -> (cortex::store::db::StoreManager, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store =
        cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");
    let conn = store.write_conn();
    cortex::store::migrations::run_migrations(&conn, Path::new("migrations"))
        .expect("failed to run migrations");
    drop(conn);
    (store, tmp)
}

#[test]
fn test_semantic_search_validates_user_input_returns_validation_functions() {
    let (store, _tmp) = setup_store();
    let conn = store.write_conn();

    // Insert test nodes representing various functions
    let nodes = vec![
        ("src/auth.rs::validate_user_input", "Function", "src/auth.rs"),
        ("src/auth.rs::check_password_strength", "Function", "src/auth.rs"),
        ("src/validation.rs::validate_email", "Function", "src/validation.rs"),
        ("src/validation.rs::sanitize_input", "Function", "src/validation.rs"),
        ("src/db.rs::connect_database", "Function", "src/db.rs"),
        ("src/db.rs::execute_query", "Function", "src/db.rs"),
        ("src/http.rs::handle_request", "Function", "src/http.rs"),
        ("src/http.rs::parse_headers", "Function", "src/http.rs"),
    ];

    for (fqn, kind, file) in &nodes {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, ?3, 1, 10, 'hash', 1000, '{}')",
            rusqlite::params![fqn, kind, file],
        ).unwrap();
    }

    // Create embeddings that simulate semantic similarity.
    // "validate" and "input" related functions get embeddings in a similar direction.
    // Database and HTTP functions get embeddings in a different direction.
    let dim = cortex::indexer::embedder::EMBEDDING_DIM;

    // Validation-related embeddings: strong signal in first few dimensions
    let mut emb_validate_input = vec![0.0f32; dim];
    emb_validate_input[0] = 0.9;
    emb_validate_input[1] = 0.85;
    emb_validate_input[2] = 0.8;
    emb_validate_input[3] = 0.7;

    let mut emb_check_password = vec![0.0f32; dim];
    emb_check_password[0] = 0.7;
    emb_check_password[1] = 0.6;
    emb_check_password[2] = 0.5;
    emb_check_password[4] = 0.8; // password-specific dimension

    let mut emb_validate_email = vec![0.0f32; dim];
    emb_validate_email[0] = 0.85;
    emb_validate_email[1] = 0.8;
    emb_validate_email[2] = 0.75;
    emb_validate_email[5] = 0.6; // email-specific dimension

    let mut emb_sanitize_input = vec![0.0f32; dim];
    emb_sanitize_input[0] = 0.8;
    emb_sanitize_input[1] = 0.7;
    emb_sanitize_input[2] = 0.65;
    emb_sanitize_input[6] = 0.5;

    // Database-related embeddings: signal in different dimensions
    let mut emb_connect_db = vec![0.0f32; dim];
    emb_connect_db[100] = 0.9;
    emb_connect_db[101] = 0.85;
    emb_connect_db[102] = 0.8;

    let mut emb_execute_query = vec![0.0f32; dim];
    emb_execute_query[100] = 0.8;
    emb_execute_query[101] = 0.75;
    emb_execute_query[103] = 0.7;

    // HTTP-related embeddings: signal in yet another direction
    let mut emb_handle_request = vec![0.0f32; dim];
    emb_handle_request[200] = 0.9;
    emb_handle_request[201] = 0.85;

    let mut emb_parse_headers = vec![0.0f32; dim];
    emb_parse_headers[200] = 0.7;
    emb_parse_headers[202] = 0.8;

    // Store all embeddings
    let entries: Vec<(&str, &[f32])> = vec![
        ("src/auth.rs::validate_user_input", &emb_validate_input),
        ("src/auth.rs::check_password_strength", &emb_check_password),
        ("src/validation.rs::validate_email", &emb_validate_email),
        ("src/validation.rs::sanitize_input", &emb_sanitize_input),
        ("src/db.rs::connect_database", &emb_connect_db),
        ("src/db.rs::execute_query", &emb_execute_query),
        ("src/http.rs::handle_request", &emb_handle_request),
        ("src/http.rs::parse_headers", &emb_parse_headers),
    ];

    cortex::store::queries::embeddings::store_embeddings_batch(&conn, &entries).unwrap();

    // Now search with a query embedding similar to "function that validates user input"
    // This should be close to the validation-related embeddings
    let mut query_emb = vec![0.0f32; dim];
    query_emb[0] = 0.88;
    query_emb[1] = 0.82;
    query_emb[2] = 0.78;
    query_emb[3] = 0.6;

    let results =
        cortex::store::queries::embeddings::semantic_search(&conn, &query_emb, 10).unwrap();

    // Verify we get results
    assert!(!results.is_empty(), "semantic search should return results");

    // The top results should be validation-related functions
    let top_3_fqns: Vec<&str> = results.iter().take(3).map(|r| r.fqn.as_str()).collect();

    // validate_user_input should be the top result (most similar)
    assert_eq!(
        results[0].fqn, "src/auth.rs::validate_user_input",
        "validate_user_input should be the most similar result"
    );

    // The top 3 should all be validation/input related
    assert!(
        top_3_fqns.contains(&"src/auth.rs::validate_user_input"),
        "top 3 should contain validate_user_input"
    );
    assert!(
        top_3_fqns.contains(&"src/validation.rs::validate_email"),
        "top 3 should contain validate_email"
    );

    // Database functions should NOT be in the top 3
    assert!(
        !top_3_fqns.contains(&"src/db.rs::connect_database"),
        "database functions should not be in top 3 for validation query"
    );
    assert!(
        !top_3_fqns.contains(&"src/db.rs::execute_query"),
        "database functions should not be in top 3 for validation query"
    );

    // Verify similarity scores are in expected range
    assert!(
        results[0].similarity > 0.9,
        "top result should have high similarity, got {}",
        results[0].similarity
    );

    // Verify metadata is populated
    assert_eq!(results[0].kind, Some("Function".to_string()));
    assert_eq!(results[0].file, Some("src/auth.rs".to_string()));
}

#[test]
fn test_semantic_search_with_no_embeddings_returns_empty() {
    let (store, _tmp) = setup_store();
    let conn = store.read_conn();

    let dim = cortex::indexer::embedder::EMBEDDING_DIM;
    let query_emb = vec![0.1f32; dim];

    let results =
        cortex::store::queries::embeddings::semantic_search(&conn, &query_emb, 10).unwrap();

    assert!(results.is_empty(), "should return empty when no embeddings stored");
}

#[test]
fn test_semantic_search_top_k_limits_results() {
    let (store, _tmp) = setup_store();
    let conn = store.write_conn();

    let dim = cortex::indexer::embedder::EMBEDDING_DIM;

    // Insert 10 nodes with embeddings
    for i in 0..10 {
        let fqn = format!("src/mod{i}.rs::func_{i}");
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, 'Function', ?2, 1, 10, 'hash', 1000, '{}')",
            rusqlite::params![fqn, format!("src/mod{i}.rs")],
        ).unwrap();

        let mut emb = vec![0.0f32; dim];
        emb[i] = 1.0;
        cortex::store::queries::embeddings::store_embedding(&conn, &fqn, &emb).unwrap();
    }

    // Search with top_k = 3
    let query_emb = vec![0.1f32; dim];
    let results =
        cortex::store::queries::embeddings::semantic_search(&conn, &query_emb, 3).unwrap();

    assert_eq!(results.len(), 3, "should return exactly top_k results");
}

#[test]
fn test_cosine_similarity_correctness() {
    // Test that our cosine similarity implementation is correct
    let a = vec![1.0f32, 0.0, 0.0];
    let b = vec![1.0f32, 0.0, 0.0];
    let sim = cortex::indexer::embedder::cosine_similarity(&a, &b);
    assert!((sim - 1.0).abs() < 1e-6, "identical vectors should have similarity 1.0");

    let a = vec![1.0f32, 0.0, 0.0];
    let b = vec![0.0f32, 1.0, 0.0];
    let sim = cortex::indexer::embedder::cosine_similarity(&a, &b);
    assert!(sim.abs() < 1e-6, "orthogonal vectors should have similarity 0.0");

    let a = vec![1.0f32, 1.0, 0.0];
    let b = vec![1.0f32, 0.0, 0.0];
    let sim = cortex::indexer::embedder::cosine_similarity(&a, &b);
    // cos(45°) ≈ 0.707
    assert!((sim - 0.7071).abs() < 0.01, "45-degree angle should be ~0.707, got {sim}");
}

#[test]
fn test_dispatch_semantic_search_with_stored_embeddings_uses_fallback() {
    // When embeddings exist but the model is not available (no ONNX model downloaded),
    // the dispatch should fall back to FTS5 search
    let (store, _tmp) = setup_store();

    {
        let conn = store.write_conn();

        // Insert a node and its embedding
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES ('src/auth.rs::validate', 'Function', 'src/auth.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        ).unwrap();

        let dim = cortex::indexer::embedder::EMBEDDING_DIM;
        let emb = vec![0.5f32; dim];
        cortex::store::queries::embeddings::store_embedding(&conn, "src/auth.rs::validate", &emb)
            .unwrap();
    }

    // Dispatch semantic_search - model won't be available so it should use FTS5 fallback
    let args = serde_json::json!({ "query": "validate", "top_k": 5 });
    let result = cortex::mcp::dispatch::dispatch_tool(&store, "semantic_search", &args).unwrap();

    assert!(result.get("content").is_some());
    assert!(result.get("_meta").is_some());

    // Parse the content
    let content = result["content"][0]["text"].as_str().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(content).unwrap();

    // Should fall back to FTS5 since model is not available
    assert_eq!(parsed["status"], "fallback_fts5");
    assert!(parsed["message"].as_str().unwrap().contains("fallback"));
}
