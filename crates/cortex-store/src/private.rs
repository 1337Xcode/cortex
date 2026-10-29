//! The store's folders and files, kept unreadable to other users of the machine.
//!
//! On Unix the store's folders get mode 0700 and its files 0600, whether it creates them or finds
//! them with a wider mode; SQLite gives the database's `-wal` and `-shm` files the database file's
//! mode. On Windows, folders and files inherit the access list of the folder they are created in,
//! and nothing here narrows it.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::path::Path;

/// Creates `folder` and any missing parents, and makes `folder` private.
pub(crate) fn create_folder(folder: &Path) -> io::Result<()> {
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
pub(crate) fn open_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

/// Flushes `folder`'s list of entries to disk, so that a file just renamed into it survives a
/// power cut. Windows offers no way to do this through the standard library; there it does
/// nothing, and the file's own contents are flushed before the rename.
#[cfg_attr(
    not(unix),
    allow(
        clippy::unnecessary_wraps,
        reason = "flushing can fail only on Unix, where it is done"
    )
)]
pub(crate) fn sync_folder(folder: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(folder)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}

// Unix only: Windows files have access lists rather than modes, and the store leaves them as
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
        create_folder(&folder)?;
        let file = folder.join("cortex.db");
        drop(open_file(&file)?);
        assert_eq!(mode(&folder)?, 0o700);
        assert_eq!(mode(&root.path().join("a"))?, 0o700);
        assert_eq!(mode(&file)?, 0o600);

        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;
        create_folder(&folder)?;
        drop(open_file(&file)?);
        assert_eq!(mode(&folder)?, 0o700);
        assert_eq!(mode(&file)?, 0o600);
        Ok(())
    }
}
