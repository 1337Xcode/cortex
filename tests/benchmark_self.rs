//! Self-benchmark for Cortex.
//!
//! Indexes Cortex's own source tree and measures performance metrics.
//! Run with: `cargo test --test benchmark_self -- --ignored --nocapture`

use std::path::PathBuf;
use std::time::Instant;

/// Get the cortex source directory (the workspace root for this crate).
fn cortex_src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Count source files and lines in the cortex src/ directory.
fn count_source_stats() -> (usize, usize) {
    let src_dir = cortex_src_dir().join("src");
    let mut file_count = 0;
    let mut line_count = 0;

    fn walk_dir(dir: &std::path::Path, file_count: &mut usize, line_count: &mut usize) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk_dir(&path, file_count, line_count);
                } else if let Some(ext) = path.extension() {
                    if ext == "rs" {
                        *file_count += 1;
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            *line_count += content.lines().count();
                        }
                    }
                }
            }
        }
    }

    walk_dir(&src_dir, &mut file_count, &mut line_count);
    (file_count, line_count)
}

/// Set up a store with migrations applied.
fn setup_store() -> (cortex::store::db::StoreManager, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let store = cortex::store::db::StoreManager::new(tmp.path()).expect("failed to create store");

    {
        let conn = store.write_conn();
        let migrations_dir = cortex_src_dir().join("migrations");
        cortex::store::migrations::run_migrations(&conn, &migrations_dir)
            .expect("failed to run migrations");
    }

    (store, tmp)
}

#[test]
#[ignore]
fn benchmark_self_index() {
    println!("\n{}", "=".repeat(70));
    println!("  CORTEX SELF-BENCHMARK");
    println!("{}\n", "=".repeat(70));

    // --- Source stats ---
    let (file_count, line_count) = count_source_stats();
    println!("Source: {} Rust files, {} lines\n", file_count, line_count);

    // --- Full index ---
    let (store, _tmp) = setup_store();
    let repo_root = cortex_src_dir();

    let start = Instant::now();
    let stats = cortex::indexer::pipeline::index_repository(&repo_root, &store)
        .expect("index_repository should succeed");
    let full_index_ms = start.elapsed().as_millis();

    let conn = store.read_conn();
    let node_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
        .unwrap();
    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
        .unwrap();
    drop(conn);

    println!("--- Indexing ---");
    println!("  Full index time:        {}ms ({} files indexed)", full_index_ms, stats.files_indexed);
    println!("  Nodes produced:         {}", node_count);
    println!("  Edges produced:         {}", edge_count);

    // --- Incremental re-index (no changes) ---
    let start = Instant::now();
    let incr_stats = cortex::indexer::pipeline::index_repository(&repo_root, &store)
        .expect("incremental re-index should succeed");
    let incr_index_ms = start.elapsed().as_millis();

    println!("  Incremental re-index:   {}ms ({} files re-indexed)", incr_index_ms, incr_stats.files_indexed);
    println!();

    // --- Query latency ---
    println!("--- Query Latency ---");

    // Get a real FQN
    let real_fqn: String = {
        let conn = store.read_conn();
        conn.query_row(
            "SELECT fqn FROM nodes WHERE kind = 'Function' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "src/main.rs::main".to_string())
    };

    // search_symbols
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "search_symbols",
        &serde_json::json!({"pattern": "*main*", "limit": 20}),
    );
    let search_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let search_ok = result.is_ok();
    println!("  search_symbols:         {:.2}ms {}", search_ms, if search_ok { "✓" } else { "✗" });

    // trace_callees
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "trace_callees",
        &serde_json::json!({"fqn": &real_fqn, "depth": 3}),
    );
    let callees_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let callees_ok = result.is_ok();
    println!("  trace_callees:          {:.2}ms {}", callees_ms, if callees_ok { "✓" } else { "✗" });

    // trace_callers
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "trace_callers",
        &serde_json::json!({"fqn": &real_fqn, "depth": 3}),
    );
    let callers_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let callers_ok = result.is_ok();
    println!("  trace_callers:          {:.2}ms {}", callers_ms, if callers_ok { "✓" } else { "✗" });

    // get_architecture
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "get_architecture",
        &serde_json::json!({}),
    );
    let arch_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let arch_ok = result.is_ok();
    println!("  get_architecture:       {:.2}ms {}", arch_ms, if arch_ok { "✓" } else { "✗" });

    // get_code_snippet
    unsafe {
        std::env::set_var("CORTEX_REPO_ROOT", cortex_src_dir().to_str().unwrap());
    }
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "get_code_snippet",
        &serde_json::json!({"fqn": &real_fqn}),
    );
    let snippet_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let snippet_ok = result.is_ok();
    println!("  get_code_snippet:       {:.2}ms {}", snippet_ms, if snippet_ok { "✓" } else { "✗" });

    // find_dead_code
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "find_dead_code",
        &serde_json::json!({"limit": 20}),
    );
    let dead_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let dead_ok = result.is_ok();
    println!("  find_dead_code:         {:.2}ms {}", dead_ms, if dead_ok { "✓" } else { "✗" });

    // search_text (FTS5)
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "search_text",
        &serde_json::json!({"query": "extract regex function", "limit": 10}),
    );
    let fts_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let fts_ok = result.is_ok();
    println!("  search_text (FTS5):     {:.2}ms {}", fts_ms, if fts_ok { "✓" } else { "✗" });

    // query_graph
    let start = Instant::now();
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "query_graph",
        &serde_json::json!({"query": "MATCH (n:Function) WHERE n.fqn LIKE '%main%' RETURN n LIMIT 10"}),
    );
    let graph_ms = start.elapsed().as_micros() as f64 / 1000.0;
    let graph_ok = result.is_ok();
    println!("  query_graph:            {:.2}ms {}", graph_ms, if graph_ok { "✓" } else { "✗" });

    println!();

    // --- Token savings ---
    println!("--- Token Savings ---");

    // search_symbols with broad pattern
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "search_symbols",
        &serde_json::json!({"pattern": "*", "limit": 50}),
    ).unwrap();
    let meta = result.get("_meta").unwrap();
    let tokens_used = meta["tokens_used"].as_u64().unwrap_or(0);
    let tokens_saved = meta["tokens_saved"].as_u64().unwrap_or(0);
    if tokens_used > 0 {
        let ratio = (tokens_saved as f64 + tokens_used as f64) / tokens_used as f64;
        println!("  search_symbols:         {:.1}x savings (used={}, saved={})", ratio, tokens_used, tokens_saved);
    } else {
        println!("  search_symbols:         used={}, saved={}", tokens_used, tokens_saved);
    }

    // get_architecture
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "get_architecture",
        &serde_json::json!({}),
    ).unwrap();
    let meta = result.get("_meta").unwrap();
    let tokens_used = meta["tokens_used"].as_u64().unwrap_or(0);
    let tokens_saved = meta["tokens_saved"].as_u64().unwrap_or(0);
    if tokens_used > 0 {
        let ratio = (tokens_saved as f64 + tokens_used as f64) / tokens_used as f64;
        println!("  get_architecture:       {:.1}x savings (used={}, saved={})", ratio, tokens_used, tokens_saved);
    } else {
        println!("  get_architecture:       used={}, saved={}", tokens_used, tokens_saved);
    }

    // trace_callees
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "trace_callees",
        &serde_json::json!({"fqn": &real_fqn, "depth": 3}),
    ).unwrap();
    let meta = result.get("_meta").unwrap();
    let tokens_used = meta["tokens_used"].as_u64().unwrap_or(0);
    let tokens_saved = meta["tokens_saved"].as_u64().unwrap_or(0);
    if tokens_used > 0 {
        let ratio = (tokens_saved as f64 + tokens_used as f64) / tokens_used as f64;
        println!("  trace_callees:          {:.1}x savings (used={}, saved={})", ratio, tokens_used, tokens_saved);
    } else {
        println!("  trace_callees:          used={}, saved={}", tokens_used, tokens_saved);
    }

    // find_dead_code
    let result = cortex::mcp::dispatch::dispatch_tool(
        &store,
        "find_dead_code",
        &serde_json::json!({"limit": 50}),
    ).unwrap();
    let meta = result.get("_meta").unwrap();
    let tokens_used = meta["tokens_used"].as_u64().unwrap_or(0);
    let tokens_saved = meta["tokens_saved"].as_u64().unwrap_or(0);
    if tokens_used > 0 {
        let ratio = (tokens_saved as f64 + tokens_used as f64) / tokens_used as f64;
        println!("  find_dead_code:         {:.1}x savings (used={}, saved={})", ratio, tokens_used, tokens_saved);
    } else {
        println!("  find_dead_code:         used={}, saved={}", tokens_used, tokens_saved);
    }

    println!();

    // --- Summary table (README-friendly) ---
    println!("--- README Benchmarks Table ---");
    println!("| Metric | Value |");
    println!("|--------|-------|");
    println!("| Source files | {} Rust files, {} lines |", file_count, line_count);
    println!("| Full index time | {}ms ({} files) |", full_index_ms, stats.files_indexed);
    println!("| Incremental re-index (no changes) | {}ms |", incr_index_ms);
    println!("| Nodes produced | {} |", node_count);
    println!("| Edges produced | {} |", edge_count);
    println!("| search_symbols latency | {:.2}ms |", search_ms);
    println!("| trace_callees latency | {:.2}ms |", callees_ms);
    println!("| get_architecture latency | {:.2}ms |", arch_ms);
    println!("| get_code_snippet latency | {:.2}ms |", snippet_ms);
    println!("| FTS5 search latency | {:.2}ms |", fts_ms);
    println!("| query_graph latency | {:.2}ms |", graph_ms);

    println!("\n{}", "=".repeat(70));
    println!("  BENCHMARK COMPLETE");
    println!("{}\n", "=".repeat(70));

    // Clean up
    unsafe {
        std::env::remove_var("CORTEX_REPO_ROOT");
    }
}
