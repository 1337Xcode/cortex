//! Reading files a repository holds, which someone hostile may have written: never through a
//! symbolic link, never anything but a regular file, never more than a set number of bytes.
//!
//! A path is inspected without following links, opened, and the opened file checked again, so a
//! link swapped in between is caught. On Unix the check compares the device and inode; on Windows,
//! where the standard library offers no file identity, it compares the size and modification time.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::Path;

/// Why a file was not read.
#[derive(Debug)]
pub enum ReadError {
    /// The path is a symbolic link, a folder or a special file.
    NotRegular,
    /// The file holds more bytes than allowed.
    TooLarge,
    /// The file could not be inspected or read.
    Io(io::Error),
}

impl From<io::Error> for ReadError {
    /// Wraps an I/O error.
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The whole of the regular file at `path`, refused when it holds more than `max_bytes`.
///
/// # Errors
///
/// [`ReadError::NotRegular`], [`ReadError::TooLarge`] and I/O errors.
pub fn read_whole(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ReadError> {
    let file = open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > max_bytes) {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

/// The first `count` bytes of the regular file at `path`, or all of it when it is shorter.
///
/// # Errors
///
/// [`ReadError::NotRegular`] and I/O errors.
pub fn read_start(path: &Path, count: u64) -> Result<Vec<u8>, ReadError> {
    let file = open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(count).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Opens `path` when it is a regular file, and only that file.
fn open_regular(path: &Path) -> Result<File, ReadError> {
    let inspected = fs::symlink_metadata(path)?;
    if !inspected.file_type().is_file() {
        return Err(ReadError::NotRegular);
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !is_same_file(&inspected, &opened) {
        return Err(ReadError::NotRegular);
    }
    Ok(file)
}

/// Whether two inspections describe the same file.
#[cfg(unix)]
fn is_same_file(inspected: &Metadata, opened: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    inspected.dev() == opened.dev() && inspected.ino() == opened.ino()
}

/// Whether two inspections describe the same file.
#[cfg(not(unix))]
fn is_same_file(inspected: &Metadata, opened: &Metadata) -> bool {
    inspected.len() == opened.len() && inspected.modified().ok() == opened.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular file is read whole up to the limit and refused past it; the start of a file is
    /// read up to the count.
    #[test]
    fn regular_files_are_read_within_limits() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("a.txt");
        fs::write(&path, b"0123456789")?;
        assert_eq!(read_whole(&path, 10).ok(), Some(b"0123456789".to_vec()));
        assert!(matches!(read_whole(&path, 9), Err(ReadError::TooLarge)));
        assert_eq!(read_start(&path, 4).ok(), Some(b"0123".to_vec()));
        assert_eq!(read_start(&path, 40).ok(), Some(b"0123456789".to_vec()));
        Ok(())
    }

    /// The identity check tells a file from another file, so a file swapped in after the first
    /// inspection is caught, and accepts the inspected file itself.
    #[test]
    fn identity_check_tells_files_apart() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let first = folder.path().join("first.txt");
        let second = folder.path().join("second.txt");
        fs::write(&first, b"one")?;
        fs::write(&second, b"two, longer")?;
        let inspected = fs::symlink_metadata(&first)?;
        let opened_same = File::open(&first)?.metadata()?;
        let opened_other = File::open(&second)?.metadata()?;
        assert!(is_same_file(&inspected, &opened_same));
        assert!(!is_same_file(&inspected, &opened_other));
        Ok(())
    }

    /// A folder is not read, and a missing file is an I/O error.
    #[test]
    fn folders_and_missing_files_are_refused() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        assert!(matches!(
            read_whole(folder.path(), 100),
            Err(ReadError::NotRegular)
        ));
        assert!(matches!(
            read_start(&folder.path().join("missing"), 100),
            Err(ReadError::Io(_))
        ));
        Ok(())
    }

    /// Unix only, where links and pipes are made without privileges: a link to a readable file and
    /// a named pipe are both refused without being opened, so the pipe cannot block the read.
    #[cfg(unix)]
    #[test]
    fn links_and_pipes_are_refused() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let target = folder.path().join("target.txt");
        fs::write(&target, b"secret")?;
        let link = folder.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link)?;
        assert!(matches!(read_whole(&link, 100), Err(ReadError::NotRegular)));
        let pipe = folder.path().join("pipe");
        let status = std::process::Command::new("mkfifo").arg(&pipe).status()?;
        assert!(status.success());
        assert!(matches!(read_start(&pipe, 100), Err(ReadError::NotRegular)));
        Ok(())
    }
}
