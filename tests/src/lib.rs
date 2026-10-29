//! Helpers shared by the cross-crate tests: where the fixtures live and how to load them.

pub mod scrub;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folder holding `fixtures/<agent>/<version>/`.
#[must_use]
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// The folder of one agent version's hook payloads, such as `fixtures/claude/2.1.312`.
#[must_use]
pub fn payload_dir(agent: &str, version: &str) -> PathBuf {
    fixtures_dir().join(agent).join(version)
}

/// Every `.json` file in `dir`, sorted by name.
///
/// # Errors
///
/// Fails when the folder cannot be listed.
pub fn json_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Every file under `dir` at any depth, sorted. Symbolic links are listed as files and never
/// followed.
///
/// # Errors
///
/// Fails when a folder cannot be listed.
pub fn files_under(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}
