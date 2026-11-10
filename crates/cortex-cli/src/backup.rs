//! Changing a file `cortex init` writes, safely, with what undoing the change takes.
//!
//! First a copy of what the file held when it was read goes into a backup folder of the run's own,
//! `<backups>/<UTC milliseconds>-<process id>/`, with a manifest naming the repository the run set
//! up and, for each file, its path and kind, its copy (none when it did not exist) with the copy's
//! BLAKE3 hash, and the BLAKE3 hash of what is written in its place, so that what was there can be
//! restored exactly, or, when the user changed the file since, Cortex's entries alone taken out.
//! The manifest of a run on a repository's files is also written to the user's backup folder, in
//! a folder of the same name, outside the repository: a manifest inside a repository may have
//! come with the clone, so the user's copy is the one to trust. Each manifest file is the run's
//! own: one already there when the run first writes it is left as it is, and the run stops. Then
//! the new contents go to a new file beside the old one, made for this write alone (never one
//! already there, nor through a link), readable by the user alone until it takes the old file's
//! permissions, and are renamed over the old file, so that a crash leaves the old file or the new
//! one, never part of either. A file that changed since it was read is left as it is, and the
//! manifests forget a write that did not happen.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use cortex_core::{ContentHash, create_private_folder};
use serde::{Deserialize, Serialize};

/// The manifest's file name in a backup folder.
pub(crate) const MANIFEST: &str = "manifest.json";

/// The file name, in the user's backup folder of a run, of the copy of the run's manifest for a
/// repository's files.
pub(crate) const REPOSITORY_MANIFEST: &str = "repository-manifest.json";

/// How many names a new file beside the one it replaces tries, each another number, before giving
/// up: another write would have to be making as many at once.
const BESIDE_TRIES: u32 = 100;

/// One run's backup folder, and what it has changed so far.
#[derive(Debug)]
pub(crate) struct Backup {
    /// The folder.
    folder: PathBuf,
    /// The run's name, `<UTC milliseconds>-<process id>`, the folder's.
    run: String,
    /// The files the manifest is written to: the user's copy first, for a run on a repository's
    /// files, then the folder's own.
    written_to: Vec<Written>,
    /// What the run changed so far.
    manifest: Manifest,
}

/// A file the manifest is written to, and what the run last wrote there.
#[derive(Debug)]
struct Written {
    /// The file.
    path: PathBuf,
    /// What the run last wrote there; `None` before its first write, when there must be no file.
    bytes: Option<Vec<u8>>,
}

/// What a backup folder's manifest says.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Manifest {
    /// The canonical root of the repository the run set up.
    pub(crate) repository: String,
    /// The canonical top of the linked working tree the run started in, whose agent files it
    /// changed, when it started in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) worktree: Option<String>,
    /// Each file changed, in the order it was.
    pub(crate) files: Vec<Changed>,
}

/// One file a run changed.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Changed {
    /// The file, as it was written: links followed.
    pub(crate) path: String,
    /// What the file is, which says how Cortex's entries come out of it.
    #[serde(flatten)]
    pub(crate) kind: Kind,
    /// The copy of what it held before, in the backup folder; `None` when it did not exist.
    pub(crate) backup: Option<BackupCopy>,
    /// The BLAKE3 hash, in hexadecimal, of what the run wrote.
    pub(crate) written_blake3: String,
}

/// The copy of what a file held before a run changed it.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BackupCopy {
    /// Its name in the run's backup folder.
    pub(crate) name: String,
    /// The BLAKE3 hash, in hexadecimal, of what it holds.
    pub(crate) blake3: String,
}

/// What a file init changes is, and for a `.gitignore` what init added to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Kind {
    /// A `.gitignore`, which gets `lines` after a `# Cortex` comment.
    Gitignore {
        /// The lines added after the comment, in order.
        lines: Vec<String>,
    },
    /// Claude Code settings, which get Cortex's hooks.
    ClaudeSettings,
    /// Another agent's settings, which get Cortex's hooks for `agent`.
    AgentSettings {
        /// The agent's name on Cortex's command line.
        agent: String,
    },
    /// Cortex's own configuration, which gets the answers to init's questions.
    CortexConfig,
}

impl Backup {
    /// A backup for a run at `now_ms` that sets up the repository whose canonical root is
    /// `repository`, in a folder of `backups` named by the time and this process, made when the
    /// first file changes.
    ///
    /// # Errors
    ///
    /// The repository's path is not UTF-8.
    pub(crate) fn new(backups: &Path, now_ms: i64, repository: &Path) -> anyhow::Result<Self> {
        let repository = repository
            .to_str()
            .with_context(|| format!("{} is not UTF-8 text", repository.display()))?;
        let run = format!("{now_ms}-{}", std::process::id());
        let folder = backups.join(&run);
        Ok(Self {
            written_to: vec![Written {
                path: folder.join(MANIFEST),
                bytes: None,
            }],
            folder,
            run,
            manifest: Manifest {
                repository: repository.to_owned(),
                worktree: None,
                files: Vec::new(),
            },
        })
    }

    /// This backup, its manifest also written to the run's folder of the same name in the user's
    /// backup folder `user_backups`, before the folder's own: for a run on a repository's files.
    pub(crate) fn with_users_copy(mut self, user_backups: &Path) -> Self {
        let path = user_backups.join(&self.run).join(REPOSITORY_MANIFEST);
        self.written_to.insert(0, Written { path, bytes: None });
        self
    }

    /// This backup, its manifest naming `worktree`, the linked working tree the run started in,
    /// whose files uninstall may then restore too.
    ///
    /// # Errors
    ///
    /// The working tree's path is not UTF-8.
    pub(crate) fn with_worktree(mut self, worktree: &Path) -> anyhow::Result<Self> {
        let worktree = worktree
            .to_str()
            .with_context(|| format!("{} is not UTF-8 text", worktree.display()))?;
        self.manifest.worktree = Some(worktree.to_owned());
        Ok(self)
    }

    /// The folder the copies and the manifest are in.
    pub(crate) fn folder(&self) -> &Path {
        &self.folder
    }

    /// Replaces the file at `path`, of `kind`, which held `before` when it was read (`None`: it
    /// did not exist), with `contents`, a copy of `before` and the manifests saved first.
    ///
    /// # Errors
    ///
    /// A path that is not UTF-8, a backup folder, copy or manifest that cannot be written, a file
    /// that changed since it was read, or a write that fails: in each case the file is left as it
    /// was, and the manifests are written again without it; failing that, the error says so.
    pub(crate) fn replace(
        &mut self,
        path: &Path,
        kind: Kind,
        before: Option<&[u8]>,
        contents: &[u8],
    ) -> anyhow::Result<()> {
        let path_text = path
            .to_str()
            .with_context(|| format!("{} is not UTF-8 text", path.display()))?
            .to_owned();
        create_private_folder(&self.folder)
            .with_context(|| format!("creating the backup folder {}", self.folder.display()))?;
        let backup = match before {
            Some(bytes) => Some(BackupCopy {
                name: self.copy(path, bytes)?,
                blake3: ContentHash::of(bytes).to_string(),
            }),
            None => None,
        };
        self.manifest.files.push(Changed {
            path: path_text,
            kind,
            backup,
            written_blake3: ContentHash::of(contents).to_string(),
        });
        if let Err(error) = self.save() {
            return Err(self.forget_last(error));
        }
        if let Err(error) = replace_unchanged(path, before, contents) {
            return Err(self.forget_last(error));
        }
        Ok(())
    }

    /// Writes `bytes`, what `path` held, to a new file in the backup folder, and returns its name.
    ///
    /// # Errors
    ///
    /// The copy cannot be written.
    fn copy(&self, path: &Path, bytes: &[u8]) -> anyhow::Result<String> {
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{}-{file}", self.manifest.files.len() + 1);
        let copy = self.folder.join(&name);
        let mut new = open_new(&copy)
            .with_context(|| format!("backing up {} to {}", path.display(), copy.display()))?;
        new.write_all(bytes)
            .and_then(|()| new.sync_all())
            .with_context(|| format!("backing up {} to {}", path.display(), copy.display()))?;
        Ok(name)
    }

    /// Writes the manifest to each of its files in turn, over what the run last wrote there, or
    /// where there is nothing yet.
    ///
    /// # Errors
    ///
    /// A folder or a file cannot be written, or a file holds anything but what the run last wrote.
    fn save(&mut self) -> anyhow::Result<()> {
        let manifest =
            serde_json::to_vec_pretty(&self.manifest).context("making the backup's manifest")?;
        for written in &mut self.written_to {
            if let Some(folder) = written.path.parent() {
                create_private_folder(folder)
                    .with_context(|| format!("creating the backup folder {}", folder.display()))?;
            }
            replace_unchanged(&written.path, written.bytes.as_deref(), &manifest)?;
            written.bytes = Some(manifest.clone());
        }
        Ok(())
    }

    /// Takes the last file out of the manifest, its change not made because of `error`, and writes
    /// the manifests again; returns `error`, saying when the manifests could not be written.
    fn forget_last(&mut self, error: anyhow::Error) -> anyhow::Error {
        self.manifest.files.pop();
        match self.save() {
            Ok(()) => error,
            Err(unsaved) => anyhow!("{error:#}; the manifest still names that file: {unsaved:#}"),
        }
    }
}

/// Writes `contents` to `path` through a new file beside it renamed over it, provided `path` still
/// holds `before` (`None`: there is nothing there) just before the rename. The new file takes the
/// old one's permissions.
///
/// # Errors
///
/// The new file cannot be made, written or renamed, or `path` changed: the new file is removed and
/// `path` left as it is.
pub(crate) fn replace_unchanged(
    path: &Path,
    before: Option<&[u8]>,
    contents: &[u8],
) -> anyhow::Result<()> {
    let (temporary, file) = new_beside(path)?;
    let written = fill(file, &temporary, path, contents).and_then(|()| {
        if holds(path, before)? {
            fs::rename(&temporary, path)
        } else {
            Err(io::Error::other("it changed since it was read"))
        }
    });
    if written.is_err() {
        // The new file may hold part of the contents; the file itself is untouched.
        drop(fs::remove_file(&temporary));
    }
    written.with_context(|| format!("writing {}", path.display()))
}

/// Whether the file at `path` holds `before` (`None`: there is nothing there), reading no more
/// than it takes to know: one byte past `before`'s length at most, and never through a link or
/// anything but a file, so that a file of any size or kind put there cannot stall the run.
pub(crate) fn holds(path: &Path, before: Option<&[u8]>) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(before.is_none()),
        Err(error) => return Err(error),
    };
    let Some(before) = before.filter(|_| metadata.is_file()) else {
        return Ok(false);
    };
    let limit = u64::try_from(before.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut current = Vec::with_capacity(before.len().saturating_add(1));
    File::open(path)?.take(limit).read_to_end(&mut current)?;
    Ok(current == before)
}

/// Writes `contents` to `file`, the new file `temporary`, durably, and gives it the permissions of
/// `path` when that exists.
fn fill(mut file: File, temporary: &Path, path: &Path, contents: &[u8]) -> io::Result<()> {
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(temporary, metadata.permissions())?;
    }
    Ok(())
}

/// A new file beside `path` for its next contents: `<name>.cortex-<process>-<n>.tmp`, the first
/// such name no file or link holds, made by this call and readable by the user alone.
///
/// # Errors
///
/// No such name is free after [`BESIDE_TRIES`], or the file cannot be made.
fn new_beside(path: &Path) -> anyhow::Result<(PathBuf, File)> {
    let name = path
        .file_name()
        .with_context(|| format!("{} names no file", path.display()))?;
    for number in 0..BESIDE_TRIES {
        let mut beside = name.to_os_string();
        beside.push(format!(".cortex-{}-{number}.tmp", std::process::id()));
        let temporary = path.with_file_name(beside);
        match open_new(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", temporary.display()));
            }
        }
    }
    bail!("no free name beside {} to write it through", path.display())
}

/// Makes the file `path` for writing, failing when anything, a link included, is there already:
/// readable by the user alone on Unix.
fn open_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names of the entries of `folder`, sorted.
    fn names(folder: &Path) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(folder)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        names.sort();
        Ok(names)
    }

    /// The manifest of `backup`.
    fn manifest(backup: &Backup) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_slice(&fs::read(
            backup.folder().join(MANIFEST),
        )?)?)
    }

    /// A file that existed is copied as it was read before it is replaced, one that did not is
    /// recorded as new, and the manifest names the repository and both files with their kinds, the
    /// copy's hash and the hash of what was written, in order. The user's copy of the manifest, in
    /// the run's folder of the same name, holds the same bytes. Nothing is left beside them.
    #[test]
    fn each_file_is_copied_then_replaced() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let backups = scratch.path().join("backups");
        let users = scratch.path().join("users");
        let repository = scratch.path().join("repo");
        let settings = scratch.path().join("settings.json");
        let fresh = scratch.path().join("new.json");
        fs::write(&settings, "old")?;
        let mut backup = Backup::new(&backups, 42, &repository)?.with_users_copy(&users);
        backup.replace(&settings, Kind::ClaudeSettings, Some(b"old"), b"new")?;
        backup.replace(&fresh, Kind::CortexConfig, None, b"created")?;
        let run = format!("42-{}", std::process::id());
        let folder = backups.join(&run);
        assert_eq!(backup.folder(), folder);
        assert_eq!(fs::read_to_string(&settings)?, "new");
        assert_eq!(fs::read_to_string(&fresh)?, "created");
        assert_eq!(fs::read_to_string(folder.join("1-settings.json"))?, "old");
        let expected = serde_json::json!({
            "repository": repository,
            "files": [
                {
                    "path": settings,
                    "kind": "claude_settings",
                    "backup": {
                        "name": "1-settings.json",
                        "blake3": ContentHash::of(b"old").to_string(),
                    },
                    "written_blake3": ContentHash::of(b"new").to_string(),
                },
                {
                    "path": fresh,
                    "kind": "cortex_config",
                    "backup": null,
                    "written_blake3": ContentHash::of(b"created").to_string(),
                },
            ],
        });
        assert_eq!(manifest(&backup)?, expected);
        assert_eq!(
            fs::read(users.join(&run).join(REPOSITORY_MANIFEST))?,
            fs::read(folder.join(MANIFEST))?
        );
        assert_eq!(
            names(scratch.path())?,
            ["backups", "new.json", "settings.json", "users"]
        );
        Ok(())
    }

    /// A file that changed since it was read is left as it is, and so is a file that appeared
    /// where there was none; the manifest names neither, and nothing is left beside them.
    #[test]
    fn a_file_changed_since_it_was_read_is_left() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let settings = scratch.path().join("settings.json");
        fs::write(&settings, "changed by someone else")?;
        let mut backup = Backup::new(&scratch.path().join("backups"), 7, scratch.path())?;
        let error = backup
            .replace(
                &settings,
                Kind::ClaudeSettings,
                Some(b"what was read"),
                b"new",
            )
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.ends_with("it changed since it was read")),
            "{error:?}"
        );
        assert!(
            backup
                .replace(&settings, Kind::ClaudeSettings, None, b"new")
                .is_err()
        );
        assert_eq!(fs::read_to_string(&settings)?, "changed by someone else");
        let expected = serde_json::json!({"repository": scratch.path(), "files": []});
        assert_eq!(manifest(&backup)?, expected);
        assert_eq!(names(scratch.path())?, ["backups", "settings.json"]);
        Ok(())
    }

    /// A file already at the name the new file would take first is not touched: the write takes
    /// the next name, and the file is still there afterwards, as it was.
    #[test]
    fn a_name_taken_beside_is_not_touched() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let settings = scratch.path().join("settings.json");
        let taken = scratch
            .path()
            .join(format!("settings.json.cortex-{}-0.tmp", std::process::id()));
        fs::write(&taken, "someone else's")?;
        replace_unchanged(&settings, None, b"new")?;
        assert_eq!(fs::read_to_string(&settings)?, "new");
        assert_eq!(fs::read_to_string(&taken)?, "someone else's");
        Ok(())
    }

    /// A link at the name the new file would take first, to a file elsewhere, is not followed:
    /// the file it leads to is untouched.
    #[cfg(unix)]
    #[test]
    fn a_link_beside_is_not_followed() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let elsewhere = tempfile::tempdir()?;
        let target = elsewhere.path().join("bashrc");
        fs::write(&target, "mine")?;
        let settings = scratch.path().join("settings.json");
        let link = scratch
            .path()
            .join(format!("settings.json.cortex-{}-0.tmp", std::process::id()));
        std::os::unix::fs::symlink(&target, link)?;
        replace_unchanged(&settings, None, b"new")?;
        assert_eq!(fs::read_to_string(&settings)?, "new");
        assert_eq!(fs::read_to_string(&target)?, "mine");
        Ok(())
    }

    /// A file that cannot be replaced (here a folder holds its name) is left as it was, both
    /// manifests name nothing, and nothing is left beside it.
    #[test]
    fn a_failed_write_leaves_the_file_and_no_leftover() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("settings.json");
        fs::create_dir(&path)?;
        let users = scratch.path().join("users");
        let mut backup = Backup::new(&scratch.path().join("backups"), 1, scratch.path())?
            .with_users_copy(&users);
        assert!(
            backup
                .replace(&path, Kind::Gitignore { lines: Vec::new() }, None, b"new")
                .is_err()
        );
        assert!(path.is_dir());
        let expected = serde_json::json!({"repository": scratch.path(), "files": []});
        assert_eq!(manifest(&backup)?, expected);
        let users_copy = users
            .join(format!("1-{}", std::process::id()))
            .join(REPOSITORY_MANIFEST);
        assert_eq!(
            fs::read(users_copy)?,
            fs::read(backup.folder().join(MANIFEST))?
        );
        assert_eq!(
            names(scratch.path())?,
            ["backups", "settings.json", "users"]
        );
        Ok(())
    }

    /// A manifest file already where the run's would go, as one a repository could carry, is
    /// left as it is, and the run stops before it changes the file.
    #[test]
    fn a_manifest_already_there_stops_the_run() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let backups = scratch.path().join("backups");
        let planted = backups.join(format!("5-{}", std::process::id()));
        fs::create_dir_all(&planted)?;
        fs::write(planted.join(MANIFEST), "planted")?;
        let settings = scratch.path().join("settings.json");
        let mut backup = Backup::new(&backups, 5, scratch.path())?;
        let error = backup
            .replace(&settings, Kind::ClaudeSettings, None, b"new")
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.contains("it changed since it was read")),
            "{error:?}"
        );
        assert_eq!(fs::read_to_string(planted.join(MANIFEST))?, "planted");
        assert!(!settings.exists());
        Ok(())
    }
}
