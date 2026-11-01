//! Where init writes. In a repository: only in folders that are the repository's own, never links
//! (a folder that is a link leads elsewhere, even to a folder beside it or to `.git`, and is refused
//! before anything is written through it), and only to files that stay inside the repository
//! whatever links they are. Elsewhere: files followed through their links. Anywhere: never to a file
//! the user made read-only.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{canonical_root, create_private_folder};

/// The file `name` in the repository whose canonical root is `root`, in its folder `folder` (such
/// as `.claude`) or at the root itself, to be written there. Nothing is created: a folder that
/// exists must be the repository's own, and the file, followed through a link, must stay inside
/// the repository.
///
/// # Errors
///
/// The folder leads elsewhere, the file is a link to nothing or leads outside the repository, or
/// either cannot be found.
pub(super) fn file_in(root: &Path, folder: Option<&str>, name: &str) -> anyhow::Result<PathBuf> {
    let parent = match folder {
        None => root.to_path_buf(),
        Some(folder) => match fs::symlink_metadata(root.join(folder)) {
            Ok(_) => own_folder(root, folder)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(root.join(folder).join(name));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("finding {}", root.join(folder).display()));
            }
        },
    };
    let file = followed(&parent.join(name))?;
    if !file.starts_with(root) {
        bail!(
            "{} leads outside the repository, to {}; Cortex does not write there",
            parent.join(name).display(),
            file.display()
        );
    }
    Ok(file)
}

/// The file `name` at the root of the repository whose canonical root is `root`, which may not be a
/// link: git reads no `.gitignore` through one, so lines written through it would never count.
///
/// # Errors
///
/// The file is a link, or cannot be found.
pub(super) fn unlinked_file_in(root: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let file = root.join(name);
    match fs::symlink_metadata(&file) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "{} is a link, which git does not read through; Cortex leaves it as it is",
            file.display()
        ),
        Ok(_) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(file),
        Err(error) => Err(error).with_context(|| format!("finding {}", file.display())),
    }
}

/// Refuses `file` when the user made it read-only: Cortex leaves such a file as it is.
///
/// # Errors
///
/// The file is read-only.
pub(super) fn writable(file: &Path) -> anyhow::Result<()> {
    if fs::metadata(file).is_ok_and(|metadata| metadata.permissions().readonly()) {
        bail!("{} is read-only; Cortex leaves it as it is", file.display());
    }
    Ok(())
}

/// The file `name` in `folder`, followed through a link when it is one; spelled as given when the
/// folder does not exist yet. Nothing is created.
///
/// # Errors
///
/// The file is a link to nothing, or cannot be found.
pub(super) fn file_beside(folder: &Path, name: &str) -> anyhow::Result<PathBuf> {
    if folder.exists() {
        followed(&folder.join(name))
    } else {
        Ok(folder.join(name))
    }
}

/// The folder `name` inside `parent`, made when missing, which must be a folder of its own there:
/// one that is a link leads elsewhere, and is refused before anything is written through it.
///
/// # Errors
///
/// The folder cannot be made or found, is a link, or leads elsewhere.
pub(super) fn own_folder(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let folder = parent.join(name);
    match fs::symlink_metadata(&folder) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "{} is a link that leads elsewhere; Cortex writes only in a folder of the \
             repository's own",
            folder.display()
        ),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_folder(&folder)
                .with_context(|| format!("creating {}", folder.display()))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("finding {}", folder.display()));
        }
    }
    let real = canonical_root(&folder).with_context(|| format!("finding {}", folder.display()))?;
    // The parents are compared, not the names: on Windows a folder's name may be spelled in
    // another case than asked for, and a link to a folder beside it stays inside all the same.
    if real.parent() != Some(canonical_root(parent)?.as_path()) {
        bail!(
            "{} leads elsewhere, to {}; Cortex does not write there",
            folder.display(),
            real.display()
        );
    }
    Ok(real)
}

/// The file at `path`, followed through a link to the file it leads to; `path` itself, in its
/// folder's canonical form, when nothing is there.
///
/// # Errors
///
/// A link that leads to nothing (writing would replace it, losing where it led), or a path that
/// cannot be followed.
pub(super) fn followed(path: &Path) -> anyhow::Result<PathBuf> {
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() && !path.exists() => {
            bail!(
                "{} is a link to a file that does not exist; Cortex leaves it as it is",
                path.display()
            );
        }
        Ok(_) => fs::canonicalize(path).with_context(|| format!("finding {}", path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error).with_context(|| format!("finding {}", path.display())),
    };
    let (parent, file) = target
        .parent()
        .zip(target.file_name())
        .with_context(|| format!("{} names no file", target.display()))?;
    Ok(canonical_root(parent)
        .with_context(|| format!("finding {}", parent.display()))?
        .join(file))
}
