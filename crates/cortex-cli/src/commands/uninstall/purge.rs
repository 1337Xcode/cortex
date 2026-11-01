//! `cortex uninstall --purge`: once init's changes are undone and the daemon has stopped, deleting
//! what Cortex keeps of a repository: its `.cortex/` but `.cortex/shared/`, which the team commits
//! through git, then, in the user's folder, init's manifests for this repository and the daemon's
//! log. Nothing is deleted through a link: a `.cortex` that is one is refused, and a link inside it
//! goes as a link. The home folder's `.cortex` is the user's own, for every repository, and is
//! never purged, nor is anything when the root holds the home folder. The run folder's files stay,
//! so that no lock another process may be taking is deleted under it.

use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context as _, bail};
use cortex_core::{RepoId, UserFolder, canonical_root};

use super::remove_if_empty;
use super::runs::Run;

/// The folder inside `.cortex/` the team shares through git, which purge keeps.
const SHARED: &str = "shared";

/// Deletes what Cortex keeps of the repository at `root`, whose init `runs` were read from the
/// user folder `user`, adding to `said` what it deleted, one line each, as far as it got.
///
/// # Errors
///
/// `root` is the home folder or holds it, or that cannot be found; `.cortex` is a link or not a
/// folder; or a file or folder cannot be read or deleted.
pub(super) fn purged(
    root: &Path,
    user: &UserFolder,
    runs: &[Run],
    said: &mut Vec<String>,
) -> anyhow::Result<()> {
    refuse_home(root, user)?;
    said.push(cortex_folder(root)?);
    for run in runs {
        remove_file(&run.manifest_path)?;
        if let Some(folder) = run.manifest_path.parent() {
            remove_if_empty(folder)?;
        }
    }
    if !runs.is_empty() {
        said.push(format!(
            "purged {} of init's manifests for this repository from {}",
            runs.len(),
            user.path().join("backups").display()
        ));
    }
    if daemon_logs(user, &RepoId::of(root))? > 0 {
        said.push(format!(
            "purged the daemon's log of this repository from {}",
            user.logs_folder().display()
        ));
    }
    Ok(())
}

/// Refuses `root` when it is the home folder of the user folder `user`, or a folder holding it:
/// the home folder's `.cortex` is the user's own, for every repository.
///
/// # Errors
///
/// As said, or the home folder cannot be found.
fn refuse_home(root: &Path, user: &UserFolder) -> anyhow::Result<()> {
    let home = user
        .path()
        .parent()
        .with_context(|| format!("{} is in no folder", user.path().display()))?;
    let home = canonical_root(home)
        .with_context(|| format!("finding the home folder {}", home.display()))?;
    if home.starts_with(root) {
        bail!(
            "{} is the home folder or holds it, whose `.cortex` is the user's own, for every \
             repository; purge never deletes it",
            root.display()
        );
    }
    Ok(())
}

/// Deletes the repository's `.cortex/` at `root` but `.cortex/shared/`, and says what it did.
///
/// # Errors
///
/// `.cortex` is a link or not a folder, or something in it cannot be read or deleted.
fn cortex_folder(root: &Path) -> anyhow::Result<String> {
    let folder = root.join(".cortex");
    let metadata = match fs::symlink_metadata(&folder) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(format!("nothing to purge in {}", root.display()));
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    if !metadata.is_dir() {
        bail!(
            "{} is not a folder of the repository's own (a link or a file); purge deletes nothing \
             through it",
            folder.display()
        );
    }
    let mut is_shared_kept = false;
    for entry in fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))? {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        if entry.file_name() == SHARED {
            is_shared_kept = true;
            continue;
        }
        let path = entry.path();
        // The entry's own type, not its target's: a link is deleted as a link.
        let is_folder = entry
            .file_type()
            .with_context(|| format!("reading {}", path.display()))?
            .is_dir();
        let deleted = if is_folder {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        deleted.with_context(|| format!("deleting {}", path.display()))?;
    }
    if is_shared_kept {
        return Ok(format!(
            "purged {} but {}, which the team shares through git",
            folder.display(),
            folder.join(SHARED).display()
        ));
    }
    fs::remove_dir(&folder).with_context(|| format!("deleting {}", folder.display()))?;
    Ok(format!("purged {}", folder.display()))
}

/// Deletes the log of the daemon of the repository `repo_id` in the user folder `user`, and the
/// older files it rotated to (`.1`, `.2`, ...), and returns how many files it deleted.
///
/// # Errors
///
/// The log folder cannot be read, or a log cannot be deleted.
fn daemon_logs(user: &UserFolder, repo_id: &RepoId) -> anyhow::Result<usize> {
    let folder = user.logs_folder();
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    let log = format!("daemon-{repo_id}.log");
    let mut deleted = 0;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_rotated = name
            .strip_prefix(&log)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
        if name == log || is_rotated {
            remove_file(&entry.path())?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Deletes the file at `path`; one already gone needs nothing.
///
/// # Errors
///
/// The file cannot be deleted.
fn remove_file(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("deleting {}", path.display()))
        }
        _ => Ok(()),
    }
}
