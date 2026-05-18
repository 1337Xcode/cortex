//! Serve command: runs watcher + indexer + MCP server concurrently.
//!
//! Uses Tokio to spawn three concurrent tasks:
//! 1. File watcher: watches repo_root, sends FileEvents to channel
//! 2. Indexer: receives FileEvents, runs incremental index pipeline
//! 3. MCP server: reads stdin, dispatches tools, writes stdout
//!
//! All three share an `Arc<StoreManager>`. The serve command runs until
//! stdin is closed (MCP server exits).

use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::indexer::pipeline::index_repository;
use crate::mcp::server::McpServer;
use crate::store::db::StoreManager;
use crate::watcher::{FileEvent, FileEventKind, FileWatcher, WatchFilter};

/// Run the serve command: concurrent watcher + indexer + MCP server.
///
/// This function creates a Tokio runtime and runs until the MCP server
/// exits (stdin closed).
pub fn run(config: &Config, store: Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(config, store, false))
}

/// Run the serve command with smart-tools option.
pub fn run_with_options(config: &Config, store: Arc<StoreManager>, smart_tools: bool) -> Result<(), anyhow::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(config, store, smart_tools))
}

/// Async implementation of the serve command.
async fn run_async(config: &Config, store: Arc<StoreManager>, smart_tools: bool) -> Result<(), anyhow::Error> {
    // Step 1: Initial index if auto_index is enabled
    if config.auto_index {
        info!("Running initial index on startup");
        match index_repository(&config.repo_root, &store) {
            Ok(stats) => {
                info!(
                    "Initial index complete: {} files scanned, {} indexed, {} skipped",
                    stats.files_scanned, stats.files_indexed, stats.files_skipped
                );
            }
            Err(e) => {
                warn!("Initial index failed (continuing anyway): {}", e);
            }
        }
    }

    // Step 2: Set up watcher -> indexer channel
    let (tx, rx) = mpsc::channel::<FileEvent>(256);

    // Step 3: Spawn watcher task
    let watcher_store = Arc::clone(&store);
    let repo_root = config.repo_root.clone();
    let watcher_handle = tokio::spawn(async move {
        let filter = WatchFilter::new(&repo_root);
        match FileWatcher::new(&repo_root, filter) {
            Ok(watcher) => {
                if let Err(e) = watcher.watch(tx).await {
                    error!("File watcher error: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to initialize file watcher: {}", e);
            }
        }
        drop(watcher_store); // keep the Arc alive for the watcher's lifetime
    });

    // Step 4: Spawn indexer task
    let indexer_store = Arc::clone(&store);
    let indexer_repo_root = config.repo_root.clone();
    let indexer_handle = tokio::spawn(async move {
        run_indexer(rx, &indexer_repo_root, &indexer_store).await;
    });

    // Step 4.5: Spawn visualizer HTTP server if ui_enabled
    let viz_handle = if config.ui_enabled {
        let viz_store = Arc::clone(&store);
        Some(tokio::spawn(async move {
            if let Err(e) = super::visualizer::serve(viz_store, 9749).await {
                error!("Visualizer server error: {}", e);
            }
        }))
    } else {
        info!("Visualizer UI disabled (set ui_enabled = true in config to enable)");
        None
    };

    // Step 5: Run MCP server (blocks until stdin closes)
    let mcp_store = Arc::clone(&store);
    let mcp_server = McpServer::with_smart_tools(mcp_store, smart_tools);
    let mcp_result = mcp_server.run().await;

    // MCP server exited, clean up
    info!("MCP server exited, shutting down");
    watcher_handle.abort();
    indexer_handle.abort();
    if let Some(handle) = viz_handle {
        handle.abort();
    }

    match mcp_result {
        Ok(()) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("MCP server error: {}", e)),
    }
}

/// Indexer task: receives FileEvents and runs incremental indexing.
async fn run_indexer(mut rx: mpsc::Receiver<FileEvent>, repo_root: &Path, store: &StoreManager) {
    while let Some(event) = rx.recv().await {
        match event.kind {
            FileEventKind::Created | FileEventKind::Modified | FileEventKind::Deleted => {
                info!("File change detected: {:?} {:?}", event.kind, event.path);
                // Re-run the full pipeline which will skip unchanged files via SHA-256 delta
                match index_repository(repo_root, store) {
                    Ok(stats) => {
                        if stats.files_indexed > 0 || stats.files_deleted > 0 {
                            info!(
                                "Incremental index: {} indexed, {} deleted",
                                stats.files_indexed, stats.files_deleted
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Incremental index failed: {}", e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations::run_migrations;
    use std::path::Path;
    use tempfile::TempDir;

    /// Helper to create a store with migrations applied.
    fn setup_store(data_dir: &Path) -> Arc<StoreManager> {
        let store = StoreManager::new(data_dir).expect("failed to create store");
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).expect("failed to run migrations");
        drop(conn);
        Arc::new(store)
    }

    #[tokio::test]
    async fn test_mcp_server_responds_to_initialize() {
        let tmp = TempDir::new().unwrap();
        let store = setup_store(tmp.path());
        let server = McpServer::new(Arc::clone(&store));

        // Send an initialize request
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let response = server.handle_line(request).await;

        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "cortex");
    }

    #[tokio::test]
    async fn test_indexer_processes_file_event() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        // Create a Python file
        std::fs::write(
            repo_dir.join("hello.py"),
            "def hello():\n    pass\n",
        )
        .unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        // Create a channel and send a file event
        let (tx, rx) = mpsc::channel(10);
        let event = FileEvent {
            path: repo_dir.join("hello.py"),
            kind: FileEventKind::Created,
        };
        tx.send(event).await.unwrap();
        drop(tx); // Close channel so indexer exits

        // Run the indexer
        run_indexer(rx, &repo_dir, &store).await;

        // Verify the file was indexed
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap();
        assert!(count > 0, "expected nodes to be indexed, got {}", count);
    }
}
