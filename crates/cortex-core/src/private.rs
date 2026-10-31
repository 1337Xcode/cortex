//! Folders and files kept unreadable to other users of the machine: a repository's `.cortex`
//! folder, and the user's own, with its run files and logs.
//!
//! On Unix these folders get mode 0700 and these files 0600, whether they are created here or
//! found with a wider mode. On Windows, folders and files inherit the access list of the folder
//! they are created in, and nothing here narrows it.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::path::Path;

/// Creates `folder` and any missing parents, and makes `folder` private.
///
/// # Errors
///
/// The folder or a parent cannot be created, or its mode cannot be set.
pub fn create_private_folder(folder: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)?;
    #[cfg(unix)]
    std::fs::set_permissions(folder, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

/// Opens `path` for reading and writing, creating it when it does not exist and leaving its
/// contents as they are, and makes it private.
///
/// # Errors
///
/// The file cannot be opened or created, or its mode cannot be set.
pub fn open_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

// Unix only: Windows files have access lists rather than modes, and these helpers leave them as
// they are inherited.
#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// The permission bits of `path`.
    fn mode(path: &Path) -> io::Result<u32> {
        Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
    }

    /// New folders and files are private to the user whatever the umask allows, and existing ones
    /// with a wider mode are narrowed.
    #[test]
    fn entries_are_private() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("a").join(".cortex");
        create_private_folder(&folder)?;
        let file = folder.join("cortex.db");
        drop(open_private_file(&file)?);
        assert_eq!(mode(&folder)?, 0o700);
        assert_eq!(mode(&root.path().join("a"))?, 0o700);
        assert_eq!(mode(&file)?, 0o600);

        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;
        create_private_folder(&folder)?;
        drop(open_private_file(&file)?);
        assert_eq!(mode(&folder)?, 0o700);
        assert_eq!(mode(&file)?, 0o600);
        Ok(())
    }
}
