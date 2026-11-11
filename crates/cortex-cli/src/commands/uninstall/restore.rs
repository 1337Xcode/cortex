//! Undoing what one run of `cortex init` did to one file. A file that holds what it held before the
//! run has nothing to undo. A file that still holds what the run wrote gets back what it held
//! before, from the run's backup copy once the copy is checked against the hash the manifest
//! keeps, or is removed when the run made it; a folder the run made for an agent's hooks file, as
//! its manifest records, is removed once nothing is left in it. A file changed
//! since keeps the change: Cortex's hooks or MCP server come out of an agent's settings and the
//! very lines the run added out of a `.gitignore`, while Cortex's own configuration, and a file of
//! Cortex's own in an agent's folder (a plugin, a hook script), stay as the user left them.
//! Only plain files are read or written, never through a link, and a repository's file only while
//! the folder it is in is still the repository's own. Every write is atomic and made only while
//! the file still holds what was read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{ContentHash, MAX_SETTINGS_BYTES, canonical_root, read_text_file};

use super::runs::Run;
use crate::agents::{claude, mcp, settings};
use crate::backup::{BackupCopy, Changed, Kind, holds, replace_unchanged};
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
    let bound = bound_of(run, &path)?;
    if let Some(bound) = bound {
        checked_inside(&path, bound)?;
    }
    if let Kind::Folder = change.kind {
        return removed_if_empty(&path);
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

/// The folder among `run`'s bounds that `path` is in, the innermost when it is in more than one
/// (a linked working tree inside the repository's folder); none for a run on the user's own files,
/// which has none.
///
/// # Errors
///
/// `path` is in none of them.
fn bound_of<'r>(run: &'r Run, path: &Path) -> anyhow::Result<Option<&'r Path>> {
    if run.bounds.is_empty() {
        return Ok(None);
    }
    let folder = path.parent();
    let inside = run
        .bounds
        .iter()
        .filter(|bound| folder.is_some_and(|folder| folder.starts_with(bound)))
        .max_by_key(|bound| bound.components().count());
    match inside {
        Some(bound) => Ok(Some(bound)),
        None => bail!(
            "{} is outside {}, where init writes; it is left as it is",
            path.display(),
            run.bounds
                .iter()
                .map(|bound| bound.display().to_string())
                .collect::<Vec<_>>()
                .join(" and ")
        ),
    }
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
            settings::without_cortex_hooks(current, program, &claude::STYLE).with_context(|| {
                format!(
                    "{} changed since init and Claude Code could not read it; Cortex's hooks are \
                     left in it",
                    path.display()
                )
            })?,
            "hooks",
        ),
        Kind::AgentSettings { agent } => {
            let style = crate::agents::setup(agent)
                .and_then(|setup| setup.hooks.as_ref())
                .map(|hooks| hooks.style)
                .with_context(|| format!("{} holds the hooks of an agent this Cortex does not know", path.display()))?;
            (
                settings::without_cortex_hooks(current, program, &style).with_context(|| {
                    format!("{} changed since init and could not be read; Cortex's hooks are left in it", path.display())
                })?,
                "hooks",
            )
        }
        Kind::AgentMcp { agent } => {
            let file = crate::agents::setup(agent)
                .and_then(|setup| setup.mcp.as_ref())
                .with_context(|| {
                    format!(
                        "{} holds the MCP server of an agent this Cortex does not know",
                        path.display()
                    )
                })?;
            (
                mcp::without(current, program, file).with_context(|| {
                    format!(
                        "{} changed since init and could not be read; Cortex's MCP server is left \
                         in it",
                        path.display()
                    )
                })?,
                "MCP server",
            )
        }
        Kind::Gitignore { lines } => (without_lines(current, lines), "lines"),
        // A file of Cortex's own the user changed is theirs now.
        Kind::AgentFile { .. } | Kind::CortexConfig => {
            return Ok(Some(format!(
                "kept {}, which changed since init",
                path.display()
            )));
        }
        // A folder is undone before its contents are read.
        Kind::Folder => return Ok(None),
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

/// Removes the folder at `path`, which the run made, when nothing is left in it, and says so; a
/// folder that holds anything, is gone, or is now a link or no folder at all is left as it is.
///
/// # Errors
///
/// The folder cannot be read or removed.
fn removed_if_empty(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("finding {}", path.display())),
        Ok(metadata) if !metadata.is_dir() => return Ok(None),
        Ok(_) => {}
    }
    let is_empty = fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .next()
        .is_none();
    if !is_empty {
        return Ok(None);
    }
    fs::remove_dir(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(Some(format!("removed {}, which init made", path.display())))
}
