//! The writer lock: at most one [`crate::Writer`] per database, across threads and processes.
//!
//! The lock is the operating system's exclusive lock on a file next to the database. It is taken
//! without waiting, and the system releases it when the file is closed, including when the process
//! dies, so a crashed writer never leaves a stale lock behind.

use std::fs::{File, TryLockError};
use std::path::Path;

use crate::error::{StoreError, io};
use crate::private;

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
