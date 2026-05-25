//! Reindex command: delete the graph database and perform a clean full index.
//!
//! Deletes the existing graph.db (and WAL/SHM files), recreates the store
//! with a fresh schema, and runs a full repository index.

use std::fs;
use std::path::Path;
use std::process;

use crate::indexer::pipeline::{index_repository, IndexStats};
use crate::store::db::StoreManager;
use crate::store::migrations::run_embedded_migrations;

/// Run the reindex command: delete DB, recreate store, and perform a full index.
///
/// # Arguments
/// * `repo_root` - Path to the repository root
/// * `data_dir` - Path to the Cortex data directory (contains graph.db)
pub fn run_reindex(repo_root: &Path, data_dir: &Path) {
    // Delete existing database files (graph.db, graph.db-wal, graph.db-shm)
    for suffix in &["", "-wal", "-shm"] {
        let db_file = data_dir.join(format!("graph.db{}", suffix));
        if db_file.exists() {
            if let Err(e) = fs::remove_file(&db_file) {
                eprintln!(
                    "error: cannot delete '{}': {}. Close any running cortex processes.",
                    db_file.display(),
                    e
                );
                process::exit(1);
            }
        }
    }

    // Recreate StoreManager (triggers schema creation on fresh DB)
    let store = match StoreManager::new(data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to create store: {e}");
            process::exit(1);
        }
    };

    // Run embedded migrations on the fresh database
    {
        let conn = store.write_conn();
        if let Err(e) = run_embedded_migrations(&conn) {
            eprintln!("error: failed to run migrations: {e}");
            process::exit(1);
        }
    }

    // Run full repository index
    match index_repository(repo_root, &store) {
        Ok(stats) => {
            print_index_stats(&stats);
        }
        Err(e) => {
            eprintln!("error: indexing failed: {e}");
            process::exit(1);
        }
    }
}

/// Print a summary of the index statistics.
fn print_index_stats(stats: &IndexStats) {
    println!(
        "Reindex complete: {} scanned, {} indexed, {} skipped, {} deleted, {} nodes, {} edges ({} ms)",
        stats.files_scanned,
        stats.files_indexed,
        stats.files_skipped,
        stats.files_deleted,
        stats.nodes_added,
        stats.edges_added,
        stats.duration_ms,
    );
}
