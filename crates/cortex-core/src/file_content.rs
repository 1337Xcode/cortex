//! What a file held at one moment, as the index saw it: no file, contents of a known hash, or
//! contents the index does not read.

use crate::ContentHash;

/// What a file held at one moment, as the index saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileContent {
    /// There was no file.
    Absent,
    /// Contents with this hash.
    Hashed(ContentHash),
    /// Contents the index does not read, such as a secret file's or a file's past the size
    /// limit: not known.
    Unread,
}

impl FileContent {
    /// What a stored file holds, from the hash stored for it: none when its contents were not
    /// read.
    #[must_use]
    pub fn of_stored(content_hash: Option<ContentHash>) -> Self {
        content_hash.map_or(Self::Unread, Self::Hashed)
    }
}
