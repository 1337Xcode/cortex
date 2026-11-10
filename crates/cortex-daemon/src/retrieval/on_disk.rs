//! A file of the repository as it is on disk, read once for an answer as the index reads files and
//! parsed for its definitions at most once, however many parts of the answer need them: an edit's
//! warnings and its notices' contracts read the edited file once.

use std::cell::OnceCell;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cortex_core::{ContentHash, FileFlags, Language};
use cortex_index::{Extraction, Extractor, Grammars, open_regular};
use cortex_store::FileRecord;

/// A file as it is on disk, with its definitions once they are read.
pub(crate) struct OnDisk<'f> {
    /// The file as the store records it.
    file: &'f FileRecord,
    /// Its language.
    language: Language,
    /// Its bytes.
    bytes: Vec<u8>,
    /// The hash of its bytes.
    hash: ContentHash,
    /// Its definitions, once read: none when they could not be read in time.
    parsed: OnceCell<Option<Extraction>>,
}

impl<'f> OnDisk<'f> {
    /// `file`, in the working tree at `root`, when the index parsed it for its definitions, read
    /// as [`parsed_bytes`] reads it; none otherwise.
    pub(crate) fn read(root: &Path, file: &'f FileRecord, max_bytes: u64) -> Option<Self> {
        let language = file.language?;
        let bytes = parsed_bytes(root, file, max_bytes)?;
        Some(Self {
            file,
            language,
            hash: ContentHash::of(&bytes),
            bytes,
            parsed: OnceCell::new(),
        })
    }

    /// The hash of its bytes.
    pub(crate) fn hash(&self) -> ContentHash {
        self.hash
    }

    /// Its bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Its definitions, read with `grammars` within `timeout` the first time they are asked for;
    /// none when they cannot be read in time.
    pub(crate) fn definitions(
        &self,
        (grammars, timeout): (&Arc<Grammars>, Duration),
    ) -> Option<&Extraction> {
        self.parsed
            .get_or_init(|| {
                let mut extractor = Extractor::new(Arc::clone(grammars), timeout);
                extractor
                    .extract(&self.file.path, self.language, &self.bytes)
                    .ok()
            })
            .as_ref()
    }
}

/// The bytes of `file`, in the repository at `root`, when the index parsed it for its definitions:
/// one whose name matches a secret pattern, too large or binary was never read and is not opened
/// here, and a generated or vendored one was only listed. Read as the index reads files: never
/// through a link, never anything but a regular file, at most `max_bytes`; none otherwise.
fn parsed_bytes(root: &Path, file: &FileRecord, max_bytes: u64) -> Option<Vec<u8>> {
    let is_parsed =
        file.flags.contains(FileFlags::PARSED) && !file.flags.contains(FileFlags::SECRET_PATTERN);
    if !is_parsed {
        return None;
    }
    let file = open_regular(&file.path.to_native(root)).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (u64::try_from(bytes.len()).ok()? <= max_bytes).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use cortex_core::{Language, RepoPath};

    use super::*;

    /// The record of `path` as the index would store it with `flags`.
    fn record(path: &str, flags: FileFlags) -> Result<FileRecord, Box<dyn std::error::Error>> {
        Ok(FileRecord {
            path: RepoPath::new(path)?,
            language: Some(Language::Python),
            size_bytes: 6,
            line_count: Some(1),
            content_hash: None,
            mtime_ns: 1,
            flags,
            indexed_at_ms: 1,
        })
    }

    /// A parsed file's bytes are read, at most the cap; the same file flagged as a secret's, or
    /// never parsed, is not, though it is there to read.
    #[test]
    fn only_a_parsed_file_is_read() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("app.py"), "x = 1\n")?;
        let parsed = FileFlags::PARSED;
        let read = |flags, max_bytes| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            Ok(parsed_bytes(
                root.path(),
                &record("app.py", flags)?,
                max_bytes,
            ))
        };
        assert_eq!(read(parsed, 6)?, Some(b"x = 1\n".to_vec()));
        assert_eq!(read(parsed, 5)?, None);
        assert_eq!(read(parsed.with(FileFlags::SECRET_PATTERN), 6)?, None);
        assert_eq!(read(FileFlags::empty(), 6)?, None);
        assert_eq!(read(FileFlags::GENERATED, 6)?, None);
        Ok(())
    }
}
