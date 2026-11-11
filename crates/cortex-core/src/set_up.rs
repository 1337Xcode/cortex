//! The record that a repository is set up for Cortex on this machine: a file in the user's own
//! folder, `~/.cortex/repositories/<id>`, holding the repository's root. A repository's own
//! `.cortex` cannot say so: a repository may commit any file there (the ignore lines Cortex adds
//! keep out only the files git does not track), so a clone, or an archive of one, could carry it.
//! The daemon writes the record when it starts on a repository, which `cortex init` and Cortex's
//! own set-up both start; `cortex uninstall` removes it. A hook reads it with one look at a file's
//! metadata.

use std::io::{self, Write as _};
use std::path::Path;

use crate::{RepoId, UserFolder, create_private_folder, open_private_file};

/// Whether the repository whose canonical root is `root` is set up for Cortex on this machine, for
/// the user whose folder is `user`.
#[must_use]
pub fn is_set_up_here(user: &UserFolder, root: &Path) -> bool {
    user.set_up_path(&RepoId::of(root)).is_file()
}

/// Records, in the user's folder `user`, that the repository whose canonical root is `root` is set
/// up on this machine.
///
/// # Errors
///
/// The folder or the file cannot be written.
pub fn record_set_up(user: &UserFolder, root: &Path) -> io::Result<()> {
    let path = user.set_up_path(&RepoId::of(root));
    if let Some(folder) = path.parent() {
        create_private_folder(folder)?;
    }
    let mut file = open_private_file(&path)?;
    file.set_len(0)?;
    writeln!(file, "{}", root.display())
}

/// Forgets, in the user's folder `user`, that the repository whose canonical root is `root` is set
/// up here, and says whether it was.
///
/// # Errors
///
/// The record is there and cannot be removed.
pub fn forget_set_up(user: &UserFolder, root: &Path) -> io::Result<bool> {
    match std::fs::remove_file(user.set_up_path(&RepoId::of(root))) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository is set up here once recorded, and no longer once forgotten; forgetting it
    /// twice says it was not there the second time.
    #[test]
    fn a_record_says_a_repository_is_set_up_here() -> io::Result<()> {
        let home = tempfile::tempdir()?;
        let user = UserFolder::in_home(home.path());
        let root = home.path().join("repo");
        assert!(!is_set_up_here(&user, &root));
        record_set_up(&user, &root)?;
        assert!(is_set_up_here(&user, &root));
        assert!(!is_set_up_here(&user, &home.path().join("other")));
        assert!(forget_set_up(&user, &root)?);
        assert!(!is_set_up_here(&user, &root));
        assert!(!forget_set_up(&user, &root)?);
        Ok(())
    }
}
