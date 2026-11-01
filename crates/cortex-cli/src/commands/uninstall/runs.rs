//! The runs of `cortex init` that uninstall undoes, newest first, read from the user's backup
//! folder alone: for a repository, the user's copies of the manifests of the runs that changed its
//! files, whose backup copies are in the repository's own `.cortex/backups/`; for the user, the
//! manifests of the runs that changed the user's own files, beside their copies. A manifest inside
//! a repository is never read: it may have come with the clone.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use crate::backup::{MANIFEST, Manifest, REPOSITORY_MANIFEST};

/// One run of init: what it changed, and where its backup copies are.
pub(super) struct Run {
    /// When it ran, in UTC milliseconds, and its process id: its folder's name.
    started: (i64, u32),
    /// The file its manifest was read from.
    pub(super) manifest_path: PathBuf,
    /// The folder its backup copies are in.
    pub(super) copies: PathBuf,
    /// The repository every file it changed is in; `None` for a run on the user's own files, which
    /// may be wherever a link led init.
    pub(super) bound: Option<PathBuf>,
    /// What it changed.
    pub(super) manifest: Manifest,
}

/// The runs that changed the files of the repository whose canonical root is `root`, newest first,
/// from the backups of the user folder `user`. A manifest this build cannot read is passed over,
/// with a note saying so added to `notes`.
///
/// # Errors
///
/// The backup folder or a manifest cannot be read.
pub(super) fn of_repository(
    user: &UserFolder,
    root: &Path,
    notes: &mut Vec<String>,
) -> anyhow::Result<Vec<Run>> {
    let mut runs = Vec::new();
    for (started, name) in run_folders(&user.path().join("backups"))? {
        let path = user
            .path()
            .join("backups")
            .join(&name)
            .join(REPOSITORY_MANIFEST);
        let Some(manifest) = read(&path, notes)? else {
            continue;
        };
        if Path::new(&manifest.repository) != root {
            continue;
        }
        runs.push(Run {
            started,
            manifest_path: path,
            copies: root.join(".cortex").join("backups").join(&name),
            bound: Some(root.to_path_buf()),
            manifest,
        });
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started));
    Ok(runs)
}

/// The runs that changed the user's own files, whatever repository each set up, newest first,
/// from the backups of the user folder `user`. A manifest this build cannot read is passed over,
/// with a note saying so added to `notes`.
///
/// # Errors
///
/// The backup folder or a manifest cannot be read.
pub(super) fn of_user(user: &UserFolder, notes: &mut Vec<String>) -> anyhow::Result<Vec<Run>> {
    let backups = user.path().join("backups");
    let mut runs = Vec::new();
    for (started, name) in run_folders(&backups)? {
        let path = backups.join(&name).join(MANIFEST);
        let Some(manifest) = read(&path, notes)? else {
            continue;
        };
        runs.push(Run {
            started,
            manifest_path: path,
            copies: backups.join(&name),
            bound: None,
            manifest,
        });
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started));
    Ok(runs)
}

/// The run folders in `backups`, each with its name and the time and process the name gives: the
/// folders named `<UTC milliseconds>-<process id>`, as init names them. None when there is no
/// such folder.
///
/// # Errors
///
/// The folder cannot be read.
fn run_folders(backups: &Path) -> anyhow::Result<Vec<((i64, u32), String)>> {
    let entries = match fs::read_dir(backups) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", backups.display()));
        }
    };
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", backups.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if let Some(started) = started(&name) {
            folders.push((started, name));
        }
    }
    Ok(folders)
}

/// The time and process id a run folder's name `<UTC milliseconds>-<process id>` gives.
fn started(name: &str) -> Option<(i64, u32)> {
    let (time, process) = name.split_once('-')?;
    Some((time.parse().ok()?, process.parse().ok()?))
}

/// The manifest at `path`, or `None` when there is none, or when this build cannot read it, which
/// a note added to `notes` says.
///
/// # Errors
///
/// The file cannot be read.
fn read(path: &Path, notes: &mut Vec<String>) -> anyhow::Result<Option<Manifest>> {
    let Some(text) = read_text_file(path, MAX_SETTINGS_BYTES)? else {
        return Ok(None);
    };
    match serde_json::from_str(&text) {
        Ok(manifest) => Ok(Some(manifest)),
        Err(error) => {
            notes.push(format!(
                "passed over {}: it is not a manifest this build reads (line {}, column {})",
                path.display(),
                error.line(),
                error.column()
            ));
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only names of a time and a process id, as init gives its run folders, are runs.
    #[test]
    fn a_run_folder_is_named_by_time_and_process() {
        assert_eq!(
            started("1761955200000-4242"),
            Some((1_761_955_200_000, 4242))
        );
        assert_eq!(started("cortex-v4.db"), None);
        assert_eq!(started("1761955200000"), None);
        assert_eq!(started("12-x"), None);
    }
}
