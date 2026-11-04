//! Stores the daemon's unit tests build by hand: the writer of a temporary repository's store, and
//! files stored as the index stores them.

use std::path::Path;

use cortex_core::{FileFlags, RepoId, RepoPath};
use cortex_store::{FileIndex, FileRecord, NodeId, Writer, WriterOptions};

/// Any failure fails the test.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The store of the repository at `root`.
pub(crate) fn writer(root: &Path) -> TestResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 0,
        now_ms: 1,
    })?)
}

/// Stores the file at `path`, with no symbol, as last modified at `mtime_ns`, and returns its node.
pub(crate) fn add_file(writer: &mut Writer, path: &str, mtime_ns: i64) -> TestResult<NodeId> {
    add_flagged_file(writer, path, mtime_ns, FileFlags::empty())
}

/// Stores the file at `path` as [`add_file`] does, with `flags`, and returns its node.
pub(crate) fn add_flagged_file(
    writer: &mut Writer,
    path: &str,
    mtime_ns: i64,
    flags: FileFlags,
) -> TestResult<NodeId> {
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: RepoPath::new(path)?,
            language: None,
            size_bytes: 10,
            line_count: Some(1),
            content_hash: None,
            mtime_ns,
            flags,
            indexed_at_ms: 1,
        },
        symbols: Vec::new(),
        refs: Vec::new(),
        passes_on: Vec::new(),
    })?;
    let node = transaction
        .queries()
        .file_node_at(&RepoPath::new(path)?)?
        .ok_or("the file's node")?;
    transaction.commit()?;
    Ok(node)
}
