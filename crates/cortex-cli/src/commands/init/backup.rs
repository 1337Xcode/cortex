//! Changing a file `cortex init` writes, safely.
//!
//! First a copy of what the file held when it was read goes into a backup folder of the run's own,
//! `<backups>/<UTC milliseconds>-<process id>/`, with a manifest naming the file, its copy (none
//! when it did not exist) and the BLAKE3 hash of what is written in its place, so that uninstalling
//! can restore exactly what was there, or, when the user changed the file since, take out only
//! Cortex's entries. Then the new contents go to a new file beside the old one, made for this
//! write alone (never one already there, nor through a link), readable by the user alone until it
//! takes the old file's permissions, and are renamed over the old file, so that a crash leaves the
//! old file or the new one, never part of either. A file that changed since it was read is left as
//! it is, and the manifest forgets a write that did not happen.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{ContentHash, create_private_folder};
use serde::Serialize;

/// The manifest's file name in a backup folder.
const MANIFEST: &str = "manifest.json";

/// How many names a new file beside the one it replaces tries, each another number, before giving
/// up: another write would have to be making as many at once.
const BESIDE_TRIES: u32 = 100;

/// One run's backup folder, and what it has changed so far.
#[derive(Debug)]
pub(crate) struct Backup {
    /// The folder.
    folder: PathBuf,
    /// The files changed, in the order they were.
    manifest: Manifest,
}

/// What a backup folder's manifest says.
#[derive(Debug, Default, Serialize)]
struct Manifest {
    /// Each file changed.
    files: Vec<Changed>,
}

/// One file a run changed.
#[derive(Debug, Serialize)]
struct Changed {
    /// The file, as it was written: links followed.
    path: String,
    /// The copy of what it held before, in the backup folder; `None` when it did not exist.
    backup: Option<String>,
    /// The BLAKE3 hash, in hexadecimal, of what the run wrote.
    written_blake3: String,
}

impl Backup {
    /// A backup for a run at `now_ms` in `backups`, in a folder named by the time and this process,
    /// made when the first file changes.
    pub(crate) fn new(backups: &Path, now_ms: i64) -> Self {
        Self {
            folder: backups.join(format!("{now_ms}-{}", std::process::id())),
            manifest: Manifest::default(),
        }
    }

    /// The folder the copies and the manifest are in.
    pub(crate) fn folder(&self) -> &Path {
        &self.folder
    }

    /// Replaces the file at `path`, which held `before` when it was read (`None`: it did not
    /// exist), with `contents`, a copy of `before` and the manifest saved first.
    ///
    /// # Errors
    ///
    /// A path that is not UTF-8, a backup folder, copy or manifest that cannot be written, a file
    /// that changed since it was read, or a write that fails: in each case the file is left as it
    /// was, and the manifest names no file it did not write.
    pub(crate) fn replace(
        &mut self,
        path: &Path,
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
            Some(bytes) => Some(self.copy(path, bytes)?),
            None => None,
        };
        self.manifest.files.push(Changed {
            path: path_text,
            backup,
            written_blake3: ContentHash::of(contents).to_string(),
        });
        self.save()?;
        if let Err(error) = replace_unchanged(path, before, contents) {
            self.manifest.files.pop();
            // The manifest without the file it did not write; failing that, the error that matters
            // is the write's.
            drop(self.save());
            return Err(error);
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

    /// Writes the manifest in the backup folder, over the one an earlier change of this run wrote.
    ///
    /// # Errors
    ///
    /// The manifest cannot be written.
    fn save(&self) -> anyhow::Result<()> {
        let manifest =
            serde_json::to_vec_pretty(&self.manifest).context("writing the backup's manifest")?;
        let path = self.folder.join(MANIFEST);
        let current = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        replace_unchanged(&path, current.as_deref(), &manifest)
    }
}

/// Writes `contents` to `path` through a new file beside it renamed over it, provided `path` still
/// holds `before` (`None`: it does not exist) just before the rename. The new file takes the old
/// one's permissions.
///
/// # Errors
///
/// The new file cannot be made, written or renamed, or `path` changed: the new file is removed and
/// `path` left as it is.
fn replace_unchanged(path: &Path, before: Option<&[u8]>, contents: &[u8]) -> anyhow::Result<()> {
    let (temporary, file) = new_beside(path)?;
    let written = fill(file, &temporary, path, contents).and_then(|()| {
        let current = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if current.as_deref() == before {
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
    /// recorded as new, and the manifest names both with the hash of what was written, in order.
    /// Nothing is left beside them.
    #[test]
    fn each_file_is_copied_then_replaced() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let backups = scratch.path().join("backups");
        let settings = scratch.path().join("settings.json");
        let fresh = scratch.path().join("new.json");
        fs::write(&settings, "old")?;
        let mut backup = Backup::new(&backups, 42);
        backup.replace(&settings, Some(b"old"), b"new")?;
        backup.replace(&fresh, None, b"created")?;
        let folder = backups.join(format!("42-{}", std::process::id()));
        assert_eq!(backup.folder(), folder);
        assert_eq!(fs::read_to_string(&settings)?, "new");
        assert_eq!(fs::read_to_string(&fresh)?, "created");
        assert_eq!(fs::read_to_string(folder.join("1-settings.json"))?, "old");
        let expected = serde_json::json!({"files": [
            {
                "path": settings,
                "backup": "1-settings.json",
                "written_blake3": ContentHash::of(b"new").to_string(),
            },
            {
                "path": fresh,
                "backup": null,
                "written_blake3": ContentHash::of(b"created").to_string(),
            },
        ]});
        assert_eq!(manifest(&backup)?, expected);
        assert_eq!(
            names(scratch.path())?,
            ["backups", "new.json", "settings.json"]
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
        let mut backup = Backup::new(&scratch.path().join("backups"), 7);
        let error = backup
            .replace(&settings, Some(b"what was read"), b"new")
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.ends_with("it changed since it was read")),
            "{error:?}"
        );
        assert!(backup.replace(&settings, None, b"new").is_err());
        assert_eq!(fs::read_to_string(&settings)?, "changed by someone else");
        assert_eq!(manifest(&backup)?, serde_json::json!({"files": []}));
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

    /// A file that cannot be replaced (here a folder holds its name) is left as it was, the
    /// manifest names nothing, and nothing is left beside it.
    #[test]
    fn a_failed_write_leaves_the_file_and_no_leftover() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("settings.json");
        fs::create_dir(&path)?;
        let mut backup = Backup::new(&scratch.path().join("backups"), 1);
        assert!(backup.replace(&path, None, b"new").is_err());
        assert!(path.is_dir());
        assert_eq!(manifest(&backup)?, serde_json::json!({"files": []}));
        assert_eq!(names(scratch.path())?, ["backups", "settings.json"]);
        Ok(())
    }
}
