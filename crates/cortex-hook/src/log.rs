//! The hook's log of what went wrong: `hook.log` in the user's log folder, one line per failure,
//! at most one line a second across every hook process, rotated at 10 MiB across five files. A
//! line names the agent, the event and what failed, never the payload's text: no prompt, path,
//! command or pattern.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cortex_core::{append_private_file, create_private_folder};

/// The log's file name in the user's log folder.
const FILE_NAME: &str = "hook.log";

/// The size past which the log starts a new file.
const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// How many files the log keeps: the current one and four older ones, `hook.log.1` the newest.
const KEPT_FILES: usize = 5;

/// The least time between two lines. Every hook is a process of its own, so the gap is read from
/// the log's modification time: a burst of failures costs one line a second, not one per call.
const MIN_GAP: Duration = Duration::from_secs(1);

/// Appends `line` to the log in `folder` at `now`, unless the log was written less than
/// [`MIN_GAP`] before. Returns whether the line was written.
///
/// # Errors
///
/// The folder or the file cannot be created, rotated or written.
pub(crate) fn write(folder: &Path, now: SystemTime, line: &str) -> io::Result<bool> {
    let path = folder.join(FILE_NAME);
    match fs::metadata(&path) {
        Ok(metadata) => {
            let is_recent = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|gap| gap < MIN_GAP);
            if is_recent {
                return Ok(false);
            }
            if metadata.len() >= ROTATE_AT_BYTES {
                rotate(&path)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_folder(folder)?,
        Err(error) => return Err(error),
    }
    let mut file = append_private_file(&path)?;
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(true)
}

/// Moves each kept file one place older, the oldest dropped, and the current log to `.1`.
fn rotate(path: &Path) -> io::Result<()> {
    for older in (1..KEPT_FILES).rev() {
        let from = numbered(path, older - 1);
        match fs::rename(&from, numbered(path, older)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// The log file `number` places older than `path`: `path` itself for 0.
fn numbered(path: &Path, number: usize) -> PathBuf {
    if number == 0 {
        return path.to_path_buf();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{number}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn Error>>;

    /// The first line creates the folder and the file; a second within a second of it is skipped;
    /// one a second or more later is written.
    #[test]
    fn one_line_a_second() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let folder = scratch.path().join("logs");
        let first = SystemTime::now();
        assert!(write(&folder, first, "one")?);
        assert!(!write(&folder, first + Duration::from_millis(500), "two")?);
        assert!(write(&folder, first + Duration::from_secs(2), "three")?);
        assert_eq!(fs::read_to_string(folder.join(FILE_NAME))?, "one\nthree\n");
        Ok(())
    }

    /// A log at the size limit moves to `.1`, `.1` to `.2` and so on; the fifth file is the last
    /// kept, so the oldest drops off.
    #[test]
    fn a_full_log_rotates_across_five_files() -> TestResult {
        let scratch = tempfile::tempdir()?;
        let folder = scratch.path();
        let log = folder.join(FILE_NAME);
        for number in 1..KEPT_FILES {
            fs::write(numbered(&log, number), format!("old {number}\n"))?;
        }
        fs::File::create(&log)?.set_len(ROTATE_AT_BYTES)?;
        assert!(write(folder, SystemTime::now() + MIN_GAP * 2, "new")?);
        assert_eq!(fs::read_to_string(&log)?, "new\n");
        assert_eq!(fs::metadata(numbered(&log, 1))?.len(), ROTATE_AT_BYTES);
        for number in 2..KEPT_FILES {
            assert_eq!(
                fs::read_to_string(numbered(&log, number))?,
                format!("old {}\n", number - 1)
            );
        }
        assert!(!numbered(&log, KEPT_FILES).exists());
        Ok(())
    }
}
