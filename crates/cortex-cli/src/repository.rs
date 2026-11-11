//! Which repository a command works on: the folder `--root` names; else the repository the current
//! folder is in, the nearest folder at or above it that holds `.cortex`, below the home folder, set
//! up here or a fresh clone holding only a team's shared folder, or a linked working tree's main
//! working tree that holds one; else the current folder, which Cortex has not set up yet. A command
//! that asks the repository's daemon, starting it when none runs, never takes the home folder, a
//! folder holding it, or a file system's root (see [`root_and_worktree`]), whose daemon would
//! refuse to start; one that only reads or writes files does. `cortex mcp` serves only a repository
//! set up here (see [`served`]).

mod served;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use cortex_core::{canonical_root, is_never_a_root, working_tree_top};

pub(crate) use self::served::served;

/// The repository root `given` names, or the one the current folder is in, in its canonical form,
/// so that every spelling of one folder names one repository: for a command that reads or writes
/// files and asks no daemon.
///
/// # Errors
///
/// The current folder cannot be read, or the root is missing, unreachable or not a folder.
pub(crate) fn root(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    Ok(found(given)?.0)
}

/// The repository root, as [`root`] finds it, and the top of the working tree the paths a person
/// types are read from, canonical: the linked working tree the current folder is in, when it is in
/// one, and the root otherwise; for a command that asks the repository's daemon, which refuses a
/// root that is the home folder, a folder holding it, or a file system's root.
///
/// # Errors
///
/// The current folder cannot be read, the root is missing, unreachable or not a folder, or it is
/// one the daemon refuses.
pub(crate) fn root_and_worktree(given: Option<&Path>) -> anyhow::Result<(PathBuf, PathBuf)> {
    let (root, worktree) = found(given)?;
    refuse_never_a_root(&root)?;
    Ok((root, worktree))
}

/// The repository root and the working tree's top, as [`root_and_worktree`] finds them, whatever
/// folder they are.
///
/// # Errors
///
/// As [`root`].
fn found(given: Option<&Path>) -> anyhow::Result<(PathBuf, PathBuf)> {
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

/// Refuses `root`, canonical, when it is the user's home folder, a folder holding it, or a file
/// system's root, which Cortex never serves as a repository.
///
/// # Errors
///
/// The root is one of those, or the home folder cannot be found.
pub(crate) fn refuse_never_a_root(root: &Path) -> anyhow::Result<()> {
    let home = std::env::home_dir().context("finding the home folder")?;
    let home = canonical(&home)?;
    if is_never_a_root(root, &home) {
        bail!(
            "{} is the home folder, a folder holding it, or a file system's root, which Cortex \
             never serves as a repository: run the command in a repository, or name one with \
             --root",
            root.display()
        );
    }
    Ok(())
}

/// The repository `cortex off` and `cortex on` work on: the folder `--root` names; else the
/// repository the current folder is in, as [`root`] finds it; else the top of the git working tree
/// the current folder is in, below the home folder, which Cortex would set up on its own; else the
/// current folder. Never the home folder or a folder holding it, whose `.cortex` is the user's
/// own.
///
/// # Errors
///
/// The current folder cannot be read, the folder is missing or not a folder, or it is the home
/// folder or holds it.
pub(crate) fn switched_root(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    let home = std::env::home_dir().context("finding the home folder")?;
    let home = canonical(&home)?;
    let root = match given {
        Some(given) => canonical(given)?,
        None => {
            let current = std::env::current_dir().context("reading the current folder")?;
            match cortex_hook::locate_marked(&current, Some(&home)) {
                Some(located) => located.repository_root().to_path_buf(),
                None => {
                    let current = canonical(&current)?;
                    let top = current
                        .starts_with(&home)
                        .then(|| working_tree_top(&current, &home).map(Path::to_path_buf))
                        .flatten();
                    top.unwrap_or(current)
                }
            }
        }
    };
    refuse_never_a_root(&root)?;
    Ok(root)
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
