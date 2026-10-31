//! Which repository a command works on: the folder `--root` names, or the current folder.

use std::path::{Path, PathBuf};

use anyhow::Context;
use cortex_core::canonical_root;

/// The repository root `given` names, or the current folder, in its canonical form, so that every
/// spelling of one folder names one repository.
///
/// # Errors
///
/// The current folder cannot be read, or the root is missing, unreachable or not a folder.
pub(crate) fn root(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    let given = match given {
        Some(root) => root.to_owned(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    canonical_root(&given).with_context(|| format!("finding the folder {}", given.display()))
}
