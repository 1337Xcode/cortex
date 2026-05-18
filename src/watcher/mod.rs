//! File watcher module.
//!
//! Watches a repository directory for file system changes using native OS events
//! (via the `notify` crate) and emits `FileEvent` structs through an async channel.
//!
//! The watcher knows nothing about parsing, the graph, or MCP. It only produces
//! file events that downstream consumers (like the indexer) can act on.

pub mod filter;
pub mod watcher;

pub use filter::WatchFilter;
pub use watcher::{FileEvent, FileEventKind, FileWatcher};
