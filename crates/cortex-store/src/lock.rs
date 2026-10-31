//! The writer lock: at most one [`crate::Writer`] per database, across threads and processes.
//!
//! The lock is the operating system's exclusive lock on a file next to the database. It is taken
//! without waiting, and the system releases it when the file is closed, including when the process
//! dies, so a crashed writer never leaves a stale lock behind.

use std::fs::{File, TryLockError};
use std::path::Path;

use crate::error::{StoreError, io};
use crate::{layout, private};

/// Whether a writer holds the lock of the store in `folder`. The lock is taken shared for an
/// instant to find out, so a writer opening the store at that instant is refused as if another
/// writer held it.
///
/// # Errors
///
/// I/O errors other than a missing lock file, which means no writer ever ran.
pub fn is_writer_running(folder: &Path) -> Result<bool, StoreError> {
    let path = layout::writer_lock(folder);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io("opening the writer lock", &path)(error)),
    };
    match file.try_lock_shared() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(source)) => Err(io("checking the writer lock", &path)(source)),
    }
}

/// A held writer lock, released when dropped.
#[derive(Debug)]
pub(crate) struct WriterLock {
    /// The locked file. Closing it releases the lock.
    _file: File,
}

impl WriterLock {
    /// Takes the lock on the file at `path`, creating the file when it is missing.
    ///
    /// # Errors
    ///
    /// [`StoreError::WriterBusy`] when another writer holds the lock, and I/O errors.
    pub(crate) fn acquire(path: &Path) -> Result<Self, StoreError> {
        let file = private::open_file(path).map_err(io("opening the writer lock", path))?;
        // INVARIANT: single-writer
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StoreError::WriterBusy {
                lock: path.to_owned(),
            }),
            Err(TryLockError::Error(source)) => Err(io("locking the writer lock", path)(source)),
        }
    }
}
