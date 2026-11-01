//! Which repository a command works on: the folder `--root` names; else the repository the current
//! folder is in, found as a hook finds it (the nearest folder at or above it that holds `.cortex`,
//! below the home folder); else the current folder, which Cortex has not set up yet.

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
    if let Some(given) = given {
        return canonical(given);
    }
    let current = std::env::current_dir().context("reading the current folder")?;
    match cortex_hook::locate(&current, std::env::home_dir().as_deref()) {
        Some(located) => Ok(located.root),
        None => canonical(&current),
    }
}

/// The canonical form of `folder`.
///
/// # Errors
///
/// The folder is missing, unreachable or not a folder.
fn canonical(folder: &Path) -> anyhow::Result<PathBuf> {
    canonical_root(folder).with_context(|| format!("finding the folder {}", folder.display()))
}
