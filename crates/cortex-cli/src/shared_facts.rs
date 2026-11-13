//! Reading `.cortex/shared/facts.json` from a repository: the form of the file is
//! `cortex_core`'s (`read_facts_file`, `facts_file`), and this is the part that touches the file
//! system. The file is read and written only as a file of its own in folders of the repository's
//! own: a link anywhere on the way is refused, so that a clone cannot point Cortex at a file
//! elsewhere on the machine.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{FACTS_FILE, MAX_FACTS_FILE_BYTES, SHARED_FOLDER, read_text_file};

/// The text of the shared facts file of the repository at `root`, none when there is none, read
/// only through folders of the repository's own and only from a file of its own.
///
/// # Errors
///
/// `.cortex` or its shared folder is a link, the file is a link or not a file, or it cannot be
/// read as UTF-8 text of at most [`MAX_FACTS_FILE_BYTES`].
pub(crate) fn read_shared_facts(root: &Path) -> anyhow::Result<Option<(PathBuf, String)>> {
    let cortex = root.join(".cortex");
    let shared = cortex.join(SHARED_FOLDER);
    for folder in [&cortex, &shared] {
        if fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!(
                "{} is a link; Cortex reads shared facts only from a folder of the repository's own",
                folder.display()
            );
        }
    }
    let path = shared.join(FACTS_FILE);
    if !is_own_file(&path)? {
        return Ok(None);
    }
    let text = read_text_file(&path, MAX_FACTS_FILE_BYTES)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(text.map(|text| (path, text)))
}

/// Whether there is a file of its own at `path`: false when nothing is there.
///
/// # Errors
///
/// A link or anything but a file is there, or it cannot be looked at.
pub(crate) fn is_own_file(path: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => bail!(
            "{} is a link or not a file; Cortex reads and writes shared facts only in a file of \
             its own",
            path.display()
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("finding {}", path.display())),
    }
}
