//! Which repository a command works on: the folder `--root` names, or the current folder.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// The repository root `given` names, or the current folder, as an absolute path.
///
/// # Errors
///
/// The current folder cannot be read, or the root is not a folder.
pub(crate) fn root(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    let given = match given {
        Some(root) => root.to_owned(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let absolute = std::path::absolute(&given)
        .with_context(|| format!("making {} absolute", given.display()))?;
    if !absolute.is_dir() {
        bail!("{} is not a folder", absolute.display());
    }
    Ok(absolute)
}
