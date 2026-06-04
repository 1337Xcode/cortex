//! `cortex status` command implementation.
//!
//! Displays index health metrics including files indexed, node/edge counts,
//! SCIP coverage, active framework adapters, and per-language breakdown.
//! When `--savings` is specified, displays the token savings dashboard.
//!
//! Satisfies Requirements 13.5, 14.4, 17.3, 24.3.

use crate::indexer::context_budget::check_embeddings_available;
use crate::mcp::health::check_health;
use crate::mcp::savings_store::get_savings_dashboard;
use crate::store::db::StoreManager;

/// Per-language file breakdown entry.
#[derive(Debug)]
struct LangBreakdown {
    ext: String,
    files: usize,
}

/// Query the nodes table for a per-language (file extension) breakdown.
///
/// Uses the query specified in the task:
/// ```sql
/// SELECT substr(file, instr(file, '.')+1) as ext,
///        COUNT(DISTINCT file) as files
/// FROM nodes
/// GROUP BY ext
/// ORDER BY files DESC
/// LIMIT 10
/// ```
fn query_language_breakdown(conn: &rusqlite::Connection) -> Vec<LangBreakdown> {
    let mut stmt = match conn.prepare(
        "SELECT substr(file, instr(file, '.')+1) as ext, \
         COUNT(DISTINCT file) as files \
         FROM nodes \
         GROUP BY ext \
         ORDER BY files DESC \
         LIMIT 10",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map([], |row| {
        Ok(LangBreakdown {
            ext: row.get::<_, String>(0).unwrap_or_default(),
            files: row.get::<_, i64>(1).unwrap_or(0) as usize,
        })
    });

    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Run the `cortex status` command.
///
/// Prints a formatted health report to stdout covering:
/// - Overall health status (healthy / unhealthy)
/// - files_indexed, node_count, edge_count
/// - SCIP coverage percentage
/// - Active framework adapters
/// - Per-language file breakdown (top 10 extensions by file count)
///
/// When `savings` is true, additionally displays the token savings dashboard:
/// - Cumulative net tokens saved
/// - Average savings per query
/// - Number of net-negative queries
/// - Baseline total and actual total
pub fn run(store: &StoreManager, savings: bool) {
    let health = check_health(store);
    let conn = store.read_conn();

    // -----------------------------------------------------------------------
    // Header
    // -----------------------------------------------------------------------
    println!("Cortex Index Status");
    println!("{}", "─".repeat(40));

    // -----------------------------------------------------------------------
    // Overall health
    // -----------------------------------------------------------------------
    let health_label = if health.healthy {
        "✓ healthy"
    } else {
        "✗ unhealthy"
    };
    println!("  Health:         {}", health_label);

    if let Some(ref reason) = health.failure_reason {
        println!("  Failure:        {}", reason);
    }
    if let Some(ref action) = health.suggested_action {
        println!("  Action:         {}", action);
    }

    println!();

    // -----------------------------------------------------------------------
    // Core metrics
    // -----------------------------------------------------------------------
    println!("  Files indexed:  {}", health.files_indexed);
    println!("  Nodes:          {}", health.node_count);
    println!("  Edges:          {}", health.edge_count);

    // SCIP coverage — stored as a fraction (0.0–1.0) in the DB
    let scip_pct = health.scip_coverage_percent * 100.0;
    println!("  SCIP coverage:  {:.1}%", scip_pct);

    // -----------------------------------------------------------------------
    // Framework adapters
    // -----------------------------------------------------------------------
    println!();
    if health.framework_coverage.is_empty() {
        println!("  Framework adapters: none detected");
    } else {
        println!(
            "  Framework adapters: {}",
            health.framework_coverage.join(", ")
        );
    }

    // -----------------------------------------------------------------------
    // Semantic search / embeddings status (Requirement 25.4)
    // -----------------------------------------------------------------------
    let (embeddings_available, embedding_count) = check_embeddings_available(&conn);
    println!();
    if embeddings_available {
        println!(
            "  Semantic search:  enabled ({} embeddings)",
            embedding_count
        );
    } else {
        println!("  Semantic search:  disabled (no embeddings)");
        println!("    ⚠ Degraded mode: using BM25-only ranking for get_task_context");
        println!("    Run `cortex semantic enable` to build embeddings");
    }

    // -----------------------------------------------------------------------
    // Per-language breakdown (monorepo support — Requirement 17.3)
    // -----------------------------------------------------------------------
    let breakdown = query_language_breakdown(&conn);

    println!();
    if breakdown.is_empty() {
        println!("  Language breakdown: no data (run `cortex index` first)");
    } else {
        println!("  Language breakdown (by file extension):");
        // Find the widest extension name for alignment
        let max_ext_len = breakdown
            .iter()
            .map(|b| b.ext.len())
            .max()
            .unwrap_or(4)
            .max(4);

        for entry in &breakdown {
            println!(
                "    {:<width$}  {} files",
                entry.ext,
                entry.files,
                width = max_ext_len
            );
        }
    }

    // -----------------------------------------------------------------------
    // Token savings dashboard (--savings flag)
    // -----------------------------------------------------------------------
    if savings {
        let dashboard = get_savings_dashboard(&conn);

        println!();
        println!("Token Savings Dashboard");
        println!("{}", "─".repeat(40));
        println!("  Cumulative net saved:   {} tokens", dashboard.cumulative_net_saved);
        println!("  Average per query:      {:.1} tokens", dashboard.average_savings_per_query);
        println!("  Net-negative queries:   {}", dashboard.queries_net_negative);
        println!("  Total queries:          {}", dashboard.total_queries);
        println!("  Baseline total:         {} tokens", dashboard.baseline_total);
        println!("  Actual total:           {} tokens", dashboard.actual_total);
    }

    println!();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::store::db::StoreManager;

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    fn setup_test_store() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();
        let conn = store.write_conn();
        crate::store::migrations::run_migrations(&conn, &migrations_dir()).unwrap();
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_run_empty_index_does_not_panic() {
        let (store, _tmp) = setup_test_store();
        // Should not panic on an empty index
        run(&store, false);
    }

    #[test]
    fn test_run_populated_index_does_not_panic() {
        let (store, _tmp) = setup_test_store();

        // Populate index_health
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health \
                 SET files_indexed = 42, node_count = 300, edge_count = 500, \
                     scip_coverage_percent = 0.75, \
                     frameworks_detected = '[\"FastAPI\",\"React\"]' \
                 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        run(&store, false);
    }

    #[test]
    fn test_run_with_savings_flag_does_not_panic() {
        let (store, _tmp) = setup_test_store();
        // Should not panic even with no savings data
        run(&store, true);
    }

    #[test]
    fn test_language_breakdown_empty_nodes() {
        let (store, _tmp) = setup_test_store();
        let conn = store.read_conn();
        let breakdown = query_language_breakdown(&conn);
        assert!(breakdown.is_empty());
    }

    #[test]
    fn test_language_breakdown_with_nodes() {
        let (store, _tmp) = setup_test_store();

        // Insert some nodes with different file extensions
        {
            let conn = store.write_conn();
            for (file, fqn) in &[
                ("src/main.rs", "src/main.rs::main"),
                ("src/lib.rs", "src/lib.rs::helper"),
                ("src/utils.rs", "src/utils.rs::util"),
                ("app/index.ts", "app/index.ts::App"),
                ("app/utils.ts", "app/utils.ts::format"),
                ("scripts/build.py", "scripts/build.py::build"),
            ] {
                conn.execute(
                    "INSERT OR IGNORE INTO nodes (fqn, name, kind, file, start_line, end_line) \
                     VALUES (?1, ?2, 'Function', ?3, 1, 10)",
                    rusqlite::params![fqn, fqn, file],
                )
                .unwrap();
            }
        }

        let conn = store.read_conn();
        let breakdown = query_language_breakdown(&conn);

        // Should have rs (3 files), ts (2 files), py (1 file)
        assert!(!breakdown.is_empty());
        let rs_entry = breakdown.iter().find(|b| b.ext == "rs");
        assert!(rs_entry.is_some());
        assert_eq!(rs_entry.unwrap().files, 3);

        let ts_entry = breakdown.iter().find(|b| b.ext == "ts");
        assert!(ts_entry.is_some());
        assert_eq!(ts_entry.unwrap().files, 2);

        let py_entry = breakdown.iter().find(|b| b.ext == "py");
        assert!(py_entry.is_some());
        assert_eq!(py_entry.unwrap().files, 1);

        // Results should be ordered by file count descending
        assert!(breakdown[0].files >= breakdown[1].files);
    }

    #[test]
    fn test_language_breakdown_respects_limit() {
        let (store, _tmp) = setup_test_store();

        // Insert nodes with 12 different extensions (limit is 10)
        {
            let conn = store.write_conn();
            let exts = [
                "rs", "ts", "py", "go", "java", "kt", "rb", "php", "cs", "cpp", "c", "swift",
            ];
            for (i, ext) in exts.iter().enumerate() {
                let file = format!("src/file{}.{}", i, ext);
                let fqn = format!("src/file{}::func{}", i, i);
                conn.execute(
                    "INSERT OR IGNORE INTO nodes (fqn, name, kind, file, start_line, end_line) \
                     VALUES (?1, ?2, 'Function', ?3, 1, 10)",
                    rusqlite::params![fqn, fqn, file],
                )
                .unwrap();
            }
        }

        let conn = store.read_conn();
        let breakdown = query_language_breakdown(&conn);
        assert!(breakdown.len() <= 10);
    }
}
