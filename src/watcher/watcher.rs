//! File system watcher using the notify crate with native OS events.
//!
//! Watches a repository directory for file changes and emits FileEvent structs
//! via a tokio mpsc channel. The watcher knows nothing about parsing, the graph,
//! or MCP - it only produces file events.

use std::path::{Path, PathBuf};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::error::WatchError;
use crate::watcher::filter::WatchFilter;

/// The kind of file system event observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEventKind {
    /// A new file was created.
    Created,
    /// An existing file was modified.
    Modified,
    /// A file was deleted.
    Deleted,
}

/// A file system event emitted by the watcher.
#[derive(Debug, Clone)]
pub struct FileEvent {
    /// The absolute path to the affected file.
    pub path: PathBuf,
    /// The kind of event that occurred.
    pub kind: FileEventKind,
}

/// Watches a repository directory for file system changes using native OS events.
///
/// Uses the `notify` crate's `RecommendedWatcher` for platform-native event delivery
/// (inotify on Linux, FSEvents on macOS, ReadDirectoryChanges on Windows).
///
/// Events are filtered through a `WatchFilter` before being emitted to the channel.
pub struct FileWatcher {
    repo_root: PathBuf,
    filter: WatchFilter,
}

impl FileWatcher {
    /// Create a new FileWatcher for the given repository root.
    ///
    /// The filter determines which paths produce events and which are excluded.
    pub fn new(repo_root: &Path, filter: WatchFilter) -> Result<Self, WatchError> {
        if !repo_root.exists() {
            return Err(WatchError::InitFailed {
                reason: format!("repository root does not exist: {}", repo_root.display()),
            });
        }

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            filter,
        })
    }

    /// Start watching the repository and emit events to the provided channel.
    ///
    /// This method runs until the channel is closed or an unrecoverable error occurs.
    /// It blocks the current async task, so it should be spawned in its own tokio task.
    pub async fn watch(&self, tx: mpsc::Sender<FileEvent>) -> Result<(), WatchError> {
        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<Result<Event, notify::Error>>(256);

        // Create the watcher with a channel-based event handler
        let mut watcher = {
            let notify_tx = notify_tx.clone();
            RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    // Send event to async channel; ignore send errors (receiver dropped)
                    let _ = notify_tx.blocking_send(res);
                },
                Config::default(),
            )
            .map_err(|e| WatchError::InitFailed {
                reason: e.to_string(),
            })?
        };

        // Start watching the repository root recursively
        watcher
            .watch(&self.repo_root, RecursiveMode::Recursive)
            .map_err(|e| WatchError::InitFailed {
                reason: format!(
                    "failed to watch '{}': {}",
                    self.repo_root.display(),
                    e
                ),
            })?;

        // Drop the sender clone so the channel closes when the watcher is dropped
        drop(notify_tx);

        // Process events from the notify watcher
        while let Some(event_result) = notify_rx.recv().await {
            match event_result {
                Ok(event) => {
                    self.process_event(&event, &tx).await;
                }
                Err(e) => {
                    tracing::warn!("file watcher error: {}", e);
                }
            }

            // If the output channel is closed, stop watching
            if tx.is_closed() {
                break;
            }
        }

        Ok(())
    }

    /// Convert a notify event into FileEvent(s) and send them through the channel.
    async fn process_event(&self, event: &Event, tx: &mpsc::Sender<FileEvent>) {
        let kind = match event.kind {
            EventKind::Create(_) => Some(FileEventKind::Created),
            EventKind::Modify(_) => Some(FileEventKind::Modified),
            EventKind::Remove(_) => Some(FileEventKind::Deleted),
            _ => None,
        };

        let Some(file_event_kind) = kind else {
            return;
        };

        for path in &event.paths {
            // Skip directories - we only care about file events
            if path.is_dir() {
                continue;
            }

            // Apply filter
            if !self.filter.should_include(path, &self.repo_root) {
                continue;
            }

            let file_event = FileEvent {
                path: path.clone(),
                kind: file_event_kind.clone(),
            };

            // Send event; if channel is full or closed, skip
            if tx.send(file_event).await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};

    /// Helper to create a watcher and channel for testing.
    fn setup_watcher(repo_root: &Path) -> (FileWatcher, mpsc::Sender<FileEvent>, mpsc::Receiver<FileEvent>) {
        let filter = WatchFilter::new(repo_root);
        let watcher = FileWatcher::new(repo_root, filter).unwrap();
        let (tx, rx) = mpsc::channel(100);
        (watcher, tx, rx)
    }

    #[tokio::test]
    async fn test_create_event_received() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().to_path_buf();

        let (watcher, tx, mut rx) = setup_watcher(&repo_root);

        // Start watcher in background
        let watch_handle = tokio::spawn(async move {
            let _ = watcher.watch(tx).await;
        });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Create a file
        fs::write(repo_root.join("new_file.txt"), "hello").unwrap();

        // Wait for event with timeout
        let event = timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(event.is_ok(), "should receive event within timeout");
        let event = event.unwrap().unwrap();
        assert!(event.path.ends_with("new_file.txt"));
        assert!(
            event.kind == FileEventKind::Created || event.kind == FileEventKind::Modified,
            "expected Created or Modified event, got {:?}",
            event.kind
        );

        // Clean up
        watch_handle.abort();
    }

    #[tokio::test]
    async fn test_modify_event_received() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().to_path_buf();

        // Create file before starting watcher
        let file_path = repo_root.join("existing.txt");
        fs::write(&file_path, "initial content").unwrap();

        let (watcher, tx, mut rx) = setup_watcher(&repo_root);

        // Start watcher in background
        let watch_handle = tokio::spawn(async move {
            let _ = watcher.watch(tx).await;
        });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Modify the file
        fs::write(&file_path, "modified content").unwrap();

        // Wait for event with timeout
        let event = timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(event.is_ok(), "should receive event within timeout");
        let event = event.unwrap().unwrap();
        assert!(event.path.ends_with("existing.txt"));
        assert!(
            event.kind == FileEventKind::Modified || event.kind == FileEventKind::Created,
            "expected Modified event, got {:?}",
            event.kind
        );

        // Clean up
        watch_handle.abort();
    }

    #[tokio::test]
    async fn test_delete_event_received() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().to_path_buf();

        // Create file before starting watcher
        let file_path = repo_root.join("to_delete.txt");
        fs::write(&file_path, "will be deleted").unwrap();

        let (watcher, tx, mut rx) = setup_watcher(&repo_root);

        // Start watcher in background
        let watch_handle = tokio::spawn(async move {
            let _ = watcher.watch(tx).await;
        });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Delete the file
        fs::remove_file(&file_path).unwrap();

        // Wait for event with timeout
        let event = timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(event.is_ok(), "should receive event within timeout");
        let event = event.unwrap().unwrap();
        assert!(event.path.ends_with("to_delete.txt"));
        assert_eq!(event.kind, FileEventKind::Deleted);

        // Clean up
        watch_handle.abort();
    }

    #[tokio::test]
    async fn test_excluded_pattern_not_emitted() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().to_path_buf();

        // Create .gitignore that excludes *.log files
        fs::write(repo_root.join(".gitignore"), "*.log\n").unwrap();

        let filter = WatchFilter::new(&repo_root);
        let watcher = FileWatcher::new(&repo_root, filter).unwrap();
        let (tx, mut rx) = mpsc::channel(100);

        // Start watcher in background
        let watch_handle = tokio::spawn(async move {
            let _ = watcher.watch(tx).await;
        });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Create a .log file (should be filtered)
        fs::write(repo_root.join("debug.log"), "log content").unwrap();

        // Also create a normal file (should NOT be filtered)
        tokio::time::sleep(Duration::from_millis(100)).await;
        fs::write(repo_root.join("main.rs"), "fn main() {}").unwrap();

        // We should receive the main.rs event but NOT the debug.log event
        let mut received_events = Vec::new();
        let deadline = Duration::from_secs(3);
        let start = tokio::time::Instant::now();

        loop {
            let remaining = deadline.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => received_events.push(event),
                _ => break,
            }
        }

        // Verify no .log events were received
        let log_events: Vec<_> = received_events
            .iter()
            .filter(|e| e.path.to_string_lossy().contains("debug.log"))
            .collect();
        assert!(
            log_events.is_empty(),
            "should not receive events for excluded .log files, got: {:?}",
            log_events
        );

        // Verify we did receive the main.rs event
        let rs_events: Vec<_> = received_events
            .iter()
            .filter(|e| e.path.to_string_lossy().contains("main.rs"))
            .collect();
        assert!(
            !rs_events.is_empty(),
            "should receive events for non-excluded files"
        );

        // Clean up
        watch_handle.abort();
    }

    #[tokio::test]
    async fn test_excluded_directory_not_emitted() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().to_path_buf();

        // Create node_modules directory
        fs::create_dir_all(repo_root.join("node_modules")).unwrap();

        let (watcher, tx, mut rx) = setup_watcher(&repo_root);

        // Start watcher in background
        let watch_handle = tokio::spawn(async move {
            let _ = watcher.watch(tx).await;
        });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Create a file in node_modules (should be filtered)
        fs::write(
            repo_root.join("node_modules").join("package.json"),
            "{}",
        )
        .unwrap();

        // Create a normal file (should NOT be filtered)
        tokio::time::sleep(Duration::from_millis(100)).await;
        fs::write(repo_root.join("app.js"), "console.log('hi')").unwrap();

        // Collect events
        let mut received_events = Vec::new();
        let deadline = Duration::from_secs(3);
        let start = tokio::time::Instant::now();

        loop {
            let remaining = deadline.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => received_events.push(event),
                _ => break,
            }
        }

        // Verify no node_modules events were received
        let nm_events: Vec<_> = received_events
            .iter()
            .filter(|e| e.path.to_string_lossy().contains("node_modules"))
            .collect();
        assert!(
            nm_events.is_empty(),
            "should not receive events for node_modules, got: {:?}",
            nm_events
        );

        // Clean up
        watch_handle.abort();
    }

    #[test]
    fn test_watcher_init_fails_for_nonexistent_path() {
        let filter = WatchFilter::new(Path::new("/tmp"));
        let result = FileWatcher::new(Path::new("/nonexistent/path/that/does/not/exist"), filter);
        assert!(result.is_err());
    }
}
