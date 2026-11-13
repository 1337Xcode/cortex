//! Reading a small text file whole, such as a configuration or settings file, refusing one larger
//! than its reader takes, so that a file of any size never fills memory.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use thiserror::Error;

/// The largest configuration or settings file Cortex reads: Claude Code's settings and Cortex's
/// own configuration are a few kilobytes.
pub const MAX_SETTINGS_BYTES: u64 = 1_048_576;

/// Why a small text file could not be read.
#[derive(Debug, Error)]
pub enum TextFileError {
    /// The file exists but could not be opened or read.
    #[error("reading {path}")]
    Io {
        /// The file.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },
    /// The file holds more bytes than its reader takes.
    #[error("{path} holds more than the {max_bytes} bytes Cortex reads of such a file")]
    TooLarge {
        /// The file.
        path: String,
        /// The most its reader takes.
        max_bytes: u64,
    },
    /// The file is not UTF-8 text.
    #[error("{path} is not UTF-8 text")]
    NotText {
        /// The file.
        path: String,
    },
}

/// The text of the file at `path`, at most `max_bytes` long, or `None` when there is none. The
/// file is read once, up to one byte past the limit, so that it cannot grow between a size check
/// and the read.
///
/// # Errors
///
/// The file cannot be opened or read, holds more than `max_bytes`, or is not UTF-8 text.
pub fn read_text_file(path: &Path, max_bytes: u64) -> Result<Option<String>, TextFileError> {
    let shown = || path.display().to_string();
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(TextFileError::Io {
                path: shown(),
                source,
            });
        }
    };
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| TextFileError::Io {
            path: shown(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(TextFileError::TooLarge {
            path: shown(),
            max_bytes,
        });
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| TextFileError::NotText { path: shown() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing file is no file; a file of exactly the limit is read whole, and one byte more is
    /// refused; bytes that are not UTF-8 are refused.
    #[test]
    fn missing_oversized_and_binary_files() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("config.toml");
        assert_eq!(read_text_file(&path, 16)?, None);
        std::fs::write(&path, "#".repeat(16))?;
        assert_eq!(read_text_file(&path, 16)?.map(|text| text.len()), Some(16));
        std::fs::write(&path, "#".repeat(17))?;
        assert!(matches!(
            read_text_file(&path, 16),
            Err(TextFileError::TooLarge { max_bytes: 16, .. })
        ));
        std::fs::write(&path, [0xff, 0xfe])?;
        assert!(matches!(
            read_text_file(&path, 16),
            Err(TextFileError::NotText { .. })
        ));
        Ok(())
    }
}
