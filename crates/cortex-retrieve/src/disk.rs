//! Files read from disk while text is rendered, and only when they are the very files the index
//! read: a file whose bytes no longer hash to what the index stored lends no line number and no
//! line of code to anything Cortex shows.
//!
//! A file is read as the index reads a repository's files: never through a link at its own path,
//! never anything but a regular file, never more than a set number of bytes. A link among the
//! folders above it could lead elsewhere, and the hash turns that away too, unless it leads to the
//! very bytes the index read. Its lines are numbered as the index counts them, from 1: a line ends
//! at a newline, a carriage return before it is not part of the line, and text after the last
//! newline is a line of its own.

use std::io::Read;
use std::path::Path;

use cortex_core::{ContentHash, LineRange, RepoPath};
use cortex_index::{ReadError, open_regular};

/// Why a file lends no lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unchecked {
    /// Its bytes are not the bytes the index read: it changed since, and its stored lines are not
    /// its lines now.
    #[error("the file changed since the index read it")]
    Changed,
    /// It is larger than may be read.
    #[error("the file is larger than may be read")]
    TooLarge,
    /// It is a link, a folder or a special file, or it could not be read.
    #[error("the file is not a regular file that could be read")]
    Unreadable,
}

/// Whether the file at `path` in the repository at `root` is a regular file that opens for
/// reading, as [`CheckedFile::read`] opens it: a link, a folder, a special file, a file that is
/// gone and a file that cannot be opened are not.
#[must_use]
pub(crate) fn is_readable(root: &Path, path: &RepoPath) -> bool {
    open_regular(&path.to_native(root)).is_ok()
}

/// A file as it is on disk, its bytes those the index read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFile {
    /// Its text, any byte that is not UTF-8 shown as the replacement character.
    text: String,
    /// Where each line starts in the text, by byte.
    starts: Vec<usize>,
}

impl CheckedFile {
    /// The file at `path` in the repository at `root`, read whole when it holds at most `max_bytes`
    /// and its BLAKE3 hash is `stored`, the hash the index stored for it.
    ///
    /// # Errors
    ///
    /// [`Unchecked::Changed`] when its hash is another, [`Unchecked::TooLarge`] past `max_bytes`,
    /// [`Unchecked::Unreadable`] for a link, a folder, a special file or a failed read.
    pub fn read(
        root: &Path,
        path: &RepoPath,
        stored: &ContentHash,
        max_bytes: u64,
    ) -> Result<Self, Unchecked> {
        let file = open_regular(&path.to_native(root)).map_err(|error| match error {
            ReadError::TooLarge => Unchecked::TooLarge,
            ReadError::NotRegular | ReadError::Io(_) => Unchecked::Unreadable,
        })?;
        let mut bytes = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| Unchecked::Unreadable)?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > max_bytes) {
            return Err(Unchecked::TooLarge);
        }
        // INVARIANT: true-at-render (a file lends lines only when its bytes are the ones the index
        // read; one that changed since has no line here)
        if ContentHash::of(&bytes) != *stored {
            return Err(Unchecked::Changed);
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut starts = vec![0];
        starts.extend(
            text.match_indices('\n')
                .map(|(at, _)| at + 1)
                .filter(|start| *start < text.len()),
        );
        if text.is_empty() {
            starts.clear();
        }
        Ok(Self { text, starts })
    }

    /// How many lines it has.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        u32::try_from(self.starts.len()).unwrap_or(u32::MAX)
    }

    /// The lines `range` covers, each with its number, or `None` when the range runs past the
    /// file's last line.
    #[must_use]
    pub fn lines(&self, range: LineRange) -> Option<Vec<(u32, &str)>> {
        if range.end_line() > self.line_count() {
            return None;
        }
        (range.start_line()..=range.end_line())
            .map(|number| Some((number, self.line(number)?)))
            .collect()
    }

    /// Line `number`, from 1, without its line ending.
    fn line(&self, number: u32) -> Option<&str> {
        let at = usize::try_from(number).ok()?.checked_sub(1)?;
        let start = *self.starts.get(at)?;
        let end = self.starts.get(at + 1).copied().unwrap_or(self.text.len());
        let line = self.text.get(start..end)?;
        let line = line.strip_suffix('\n').unwrap_or(line);
        Some(line.strip_suffix('\r').unwrap_or(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A repository with `a.py` holding `text`, and that file's path.
    fn repository(text: &str) -> Result<(tempfile::TempDir, RepoPath), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("a.py"), text)?;
        Ok((root, RepoPath::new("a.py")?))
    }

    /// A file whose bytes hash to what was stored gives its lines by number: three lines, the
    /// second ending in a carriage return before its newline and the last with no newline at all.
    /// A range past the last line gives none.
    #[test]
    fn a_file_as_stored_gives_its_lines() -> TestResult {
        let text = "def add(a, b):\r\n    return a + b\nprint(add(1, 2))";
        let (root, path) = repository(text)?;
        let file = CheckedFile::read(
            root.path(),
            &path,
            &ContentHash::of(text.as_bytes()),
            1 << 20,
        )?;
        assert_eq!(file.line_count(), 3);
        assert_eq!(
            file.lines(LineRange::new(1, 3)?),
            Some(vec![
                (1, "def add(a, b):"),
                (2, "    return a + b"),
                (3, "print(add(1, 2))")
            ])
        );
        assert_eq!(
            file.lines(LineRange::new(2, 2)?),
            Some(vec![(2, "    return a + b")])
        );
        assert_eq!(file.lines(LineRange::new(3, 4)?), None);
        Ok(())
    }

    /// A file that changed since the index read it gives nothing: one byte more, and its hash is
    /// another. A file past the size limit gives nothing either, and neither does a folder.
    #[test]
    fn a_changed_file_gives_no_lines() -> TestResult {
        let (root, path) = repository("x = 1\n")?;
        let stored = ContentHash::of(b"x = 1\n");
        assert!(CheckedFile::read(root.path(), &path, &stored, 100).is_ok());
        std::fs::write(root.path().join("a.py"), "x = 12\n")?;
        assert_eq!(
            CheckedFile::read(root.path(), &path, &stored, 100),
            Err(Unchecked::Changed)
        );
        assert_eq!(
            CheckedFile::read(root.path(), &path, &ContentHash::of(b"x = 12\n"), 6),
            Err(Unchecked::TooLarge)
        );
        std::fs::create_dir(root.path().join("folder"))?;
        assert_eq!(
            CheckedFile::read(root.path(), &RepoPath::new("folder")?, &stored, 100),
            Err(Unchecked::Unreadable)
        );
        Ok(())
    }

    /// A link at the file's own path is not followed, even to a file holding the stored bytes.
    #[cfg(unix)]
    #[test]
    fn a_link_lends_no_lines() -> TestResult {
        let (root, _) = repository("x = 1\n")?;
        std::os::unix::fs::symlink(root.path().join("a.py"), root.path().join("b.py"))?;
        assert_eq!(
            CheckedFile::read(
                root.path(),
                &RepoPath::new("b.py")?,
                &ContentHash::of(b"x = 1\n"),
                100
            ),
            Err(Unchecked::Unreadable)
        );
        Ok(())
    }

    /// How many lines `text` has when written to a file and read back checked.
    fn lines_read(text: &str) -> Result<u32, Box<dyn std::error::Error>> {
        let (root, path) = repository(text)?;
        let stored = ContentHash::of(text.as_bytes());
        Ok(CheckedFile::read(root.path(), &path, &stored, 1_000)?.line_count())
    }

    proptest::proptest! {
        /// Lines are counted as the index counts them for the file's stored line count, whatever
        /// the text: line feeds, carriage returns and other characters in any order.
        #[test]
        fn lines_are_counted_as_the_index_counts_them(text in "[a\\r\\n\u{e9}]{0,40}") {
            let read = lines_read(&text)
                .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
            proptest::prop_assert_eq!(u64::from(read), cortex_index::count_lines(text.as_bytes()));
        }
    }

    /// A file ending in a newline has as many lines as newlines; an empty file has none.
    #[test]
    fn lines_are_counted_as_an_editor_counts_them() -> TestResult {
        for (text, count) in [("a\nb\n", 2), ("a\nb", 2), ("\n", 1), ("", 0)] {
            let (root, path) = repository(text)?;
            let file =
                CheckedFile::read(root.path(), &path, &ContentHash::of(text.as_bytes()), 100)?;
            assert_eq!(file.line_count(), count, "{text:?}");
        }
        Ok(())
    }
}
