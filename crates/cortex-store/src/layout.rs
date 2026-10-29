//! Where the store keeps its files inside a repository's `.cortex` folder.

use std::path::{Path, PathBuf};

/// The database file inside `folder`.
pub(crate) fn database(folder: &Path) -> PathBuf {
    folder.join("cortex.db")
}

/// The writer's lock file inside `folder`.
pub(crate) fn writer_lock(folder: &Path) -> PathBuf {
    folder.join("writer.lock")
}

/// The folder for the copies made before a migration, inside `folder`.
pub(crate) fn backups(folder: &Path) -> PathBuf {
    folder.join("backups")
}
