//! Which repository a command works on: the folder `--root` names; else the repository the current
//! folder is in, the nearest folder at or above it that holds `.cortex`, below the home folder, set
//! up here or a fresh clone holding only a team's shared folder, or a linked working tree's main
//! working tree that holds one; else the current folder, which Cortex has not set up yet.

use std::path::{Path, PathBuf};

use anyhow::Context;
use cortex_core::canonical_root;

/// The repository root `given` names, or the one the current folder is in, in its canonical form,
/// so that every spelling of one folder names one repository.
///
/// # Errors
///
/// The current folder cannot be read, or the root is missing, unreachable or not a folder.
pub(crate) fn root(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    Ok(root_and_worktree(given)?.0)
}

/// The repository root, as [`root`] finds it, and the top of the working tree the paths a person
/// types are read from, canonical: the linked working tree the current folder is in, when it is in
/// one, and the root otherwise.
///
/// # Errors
///
/// The current folder cannot be read, or the root is missing, unreachable or not a folder.
pub(crate) fn root_and_worktree(given: Option<&Path>) -> anyhow::Result<(PathBuf, PathBuf)> {
    let alone = |folder: &Path| canonical(folder).map(|root| (root.clone(), root));
    if let Some(given) = given {
        return alone(given);
    }
    let current = std::env::current_dir().context("reading the current folder")?;
    match cortex_hook::locate_marked(&current, std::env::home_dir().as_deref()) {
        Some(located) => Ok((located.repository_root().to_path_buf(), located.root)),
        None => alone(&current),
    }
}

/// `worktree`, the working tree a command was run in, when it is a linked working tree of the
/// repository at `root`: the top a request names for the daemon to read its files from; none for
/// the repository's own.
pub(crate) fn linked(root: &Path, worktree: PathBuf) -> Option<PathBuf> {
    (worktree != root).then_some(worktree)
}

/// The canonical form of `folder`.
///
/// # Errors
///
/// The folder is missing, unreachable or not a folder.
fn canonical(folder: &Path) -> anyhow::Result<PathBuf> {
    canonical_root(folder).with_context(|| format!("finding the folder {}", folder.display()))
}
