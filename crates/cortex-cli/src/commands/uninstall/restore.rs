//! Undoing what one run of `cortex init` did to one file. A file that holds what it held before the
//! run has nothing to undo. A file that still holds what the run wrote gets back what it held
//! before, from the run's backup copy once the copy is checked against the hash the manifest
//! keeps, or is removed when the run made it, with the repository's `.claude` folder when that is
//! left empty. A file changed since keeps the change: Cortex's hooks come out of Claude Code
//! settings and the very lines the run added out of a `.gitignore`, while Cortex's own
//! configuration stays as the user left it. Only plain files are read or written, never through
//! a link, and a repository's file only while the folder it is in is still the repository's own.
//! Every write is atomic and made only while the file still holds what was read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{ContentHash, MAX_SETTINGS_BYTES, canonical_root, read_text_file};

use super::runs::Run;
use crate::backup::{BackupCopy, Changed, Kind, holds, replace_unchanged};
use crate::claude_settings;
use crate::commands::init::without_lines;

/// Undoes `change`, a change of `run`, `program` being this binary, and says what it did; `None`
/// when there was nothing to undo.
///
/// # Errors
///
/// The file is outside the repository or reached through a link, is not a plain file, cannot be
/// read or written, changed while it was undone, or, changed since init, cannot be read as its
/// kind; or the run's copy is gone or does not hold what init copied. The file is left as it is.
pub(super) fn undo(run: &Run, change: &Changed, program: &str) -> anyhow::Result<Option<String>> {
    let path = PathBuf::from(&change.path);
    if let Some(bound) = &run.bound {
        checked_inside(&path, bound)?;
    }
    let Some(current) = read_plain(&path)? else {
        return Ok(None);
    };
    let hash = ContentHash::of(current.as_bytes()).to_string();
    if change
        .backup
        .as_ref()
        .is_some_and(|copy| copy.blake3 == hash)
    {
        return Ok(None);
    }
    if hash == change.written_blake3 {
        return restored(run, change, &path, &current).map(Some);
    }
    taken_out(&change.kind, &path, &current, program)
}

/// Refuses `path`, a file of the repository at `bound`, when it is outside the repository or its
/// folder now leads elsewhere through a link, as it could after a pull: init writes only in
/// folders of the repository's own, and records their canonical spelling.
///
/// # Errors
///
/// As said, or the folder cannot be found.
fn checked_inside(path: &Path, bound: &Path) -> anyhow::Result<()> {
    let Some(folder) = path.parent().filter(|folder| folder.starts_with(bound)) else {
        bail!(
            "{} is outside {}, where init writes; it is left as it is",
            path.display(),
            bound.display()
        );
    };
    match canonical_root(folder) {
        Ok(real) if real != folder => bail!(
            "{} now leads to {} through a link; {} is left as it is",
            folder.display(),
            real.display(),
            path.display()
        ),
        Ok(_) => Ok(()),
        // A folder that is gone holds nothing to undo.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("finding {}", folder.display())),
    }
}

/// What the plain file at `path` holds, at most [`MAX_SETTINGS_BYTES`]; `None` when there is none.
///
/// # Errors
///
/// It is a link or anything but a plain file, or cannot be read as small text.
fn read_plain(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "{} is not a plain file; it is left as it is",
                path.display()
            )
        }
        Ok(_) => {}
    }
    Ok(read_text_file(path, MAX_SETTINGS_BYTES)?)
}

/// Gives the file at `path`, which still holds `current`, what `change` wrote, back what it held
/// before, or removes it when `run` made it, and says which.
///
/// # Errors
///
/// As [`undo`].
fn restored(run: &Run, change: &Changed, path: &Path, current: &str) -> anyhow::Result<String> {
    let Some(copy) = &change.backup else {
        remove_unchanged(path, current)?;
        if let Some(bound) = &run.bound {
            remove_claude_left_empty(path, bound)?;
        }
        return Ok(format!("removed {}, which init made", path.display()));
    };
    let before = copied(run, copy)?;
    replace_unchanged(path, Some(current.as_bytes()), before.as_bytes())?;
    Ok(format!("restored {} from its backup", path.display()))
}

/// What the backup `copy` of `run` holds, once it is known to hold what init copied.
///
/// # Errors
///
/// The copy's name is not a file name, or the copy is gone, is not a plain file, cannot be read,
/// or holds anything else.
fn copied(run: &Run, copy: &BackupCopy) -> anyhow::Result<String> {
    let is_file_name =
        !matches!(copy.name.as_str(), "" | "." | "..") && !copy.name.contains(['/', '\\']);
    if !is_file_name {
        bail!(
            "the manifest names a backup that is not a file name: {:?}",
            copy.name
        );
    }
    let path = run.copies.join(&copy.name);
    let text =
        read_plain(&path)?.with_context(|| format!("the backup {} is gone", path.display()))?;
    if ContentHash::of(text.as_bytes()).to_string() != copy.blake3 {
        bail!(
            "the backup {} does not hold what init copied; nothing is restored from it",
            path.display()
        );
    }
    Ok(text)
}

/// Takes the entries the run added out of the file at `path`, of `kind`, which holds `current` and
/// changed since init, `program` being this binary, and says so; `None` when it holds none.
///
/// # Errors
///
/// As [`undo`].
fn taken_out(
    kind: &Kind,
    path: &Path,
    current: &str,
    program: &str,
) -> anyhow::Result<Option<String>> {
    let (without, what) = match kind {
        Kind::ClaudeSettings => (
            claude_settings::without_cortex_hooks(current, program).with_context(|| {
                format!(
                    "{} changed since init and Claude Code could not read it; Cortex's hooks are \
                     left in it",
                    path.display()
                )
            })?,
            "hooks",
        ),
        Kind::Gitignore { lines } => (without_lines(current, lines), "lines"),
        Kind::CortexConfig => {
            return Ok(Some(format!(
                "kept {}, which changed since init",
                path.display()
            )));
        }
    };
    let Some(without) = without else {
        return Ok(None);
    };
    replace_unchanged(path, Some(current.as_bytes()), without.as_bytes())?;
    Ok(Some(format!(
        "took Cortex's {what} out of {}, which changed since init",
        path.display()
    )))
}

/// Removes the file at `path` provided it still holds `before`, read no further than that takes.
///
/// # Errors
///
/// The file cannot be read or removed, or changed since it was read.
fn remove_unchanged(path: &Path, before: &str) -> anyhow::Result<()> {
    if !holds(path, Some(before.as_bytes()))
        .with_context(|| format!("reading {}", path.display()))?
    {
        bail!(
            "{} changed while uninstall read it; it is left as it is",
            path.display()
        );
    }
    fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
}

/// Removes the `.claude` folder of the repository at `bound` when `path` was in it and nothing is
/// left in it: init makes that folder for the settings it makes.
///
/// # Errors
///
/// The folder cannot be read or removed.
fn remove_claude_left_empty(path: &Path, bound: &Path) -> anyhow::Result<()> {
    let folder = bound.join(".claude");
    if path.parent() != Some(folder.as_path()) {
        return Ok(());
    }
    super::remove_if_empty(&folder)
}
