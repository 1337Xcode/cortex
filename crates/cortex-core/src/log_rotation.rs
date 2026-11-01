//! Rotating a log by size: the log moves to `.1`, each older file one place further, and the
//! oldest kept is dropped. The hook's log and the daemon's log rotate the same way.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Makes room for a new log at `path`, of which `kept_files` files are kept, the log itself
/// counted: each older file moves one place older (`.1` to `.2` and so on), the oldest kept is
/// replaced, and the log moves to `.1`. A file missing on the way is skipped.
///
/// # Errors
///
/// A file that exists cannot be renamed.
pub fn rotate_log(path: &Path, kept_files: usize) -> io::Result<()> {
    for older in (1..kept_files).rev() {
        match fs::rename(numbered(path, older - 1), numbered(path, older)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// The log file `number` places older than `path`: `path` itself for 0, else `path` with
/// `.<number>` after its name.
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

    /// Five files kept: `log`, `.1` to `.4`. A rotation moves `log` to `.1`, `.1` to `.2` and so
    /// on, `.3` replacing `.4`, which drops; no `.5` appears. With `.2` missing, `.1` still moves
    /// on and `.3` becomes `.4`.
    #[test]
    fn each_file_moves_one_place_older() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("a.log");
        let name = |number: usize| numbered(&log, number);
        for number in 0..5 {
            fs::write(name(number), format!("{number}"))?;
        }
        rotate_log(&log, 5)?;
        assert!(!log.exists());
        let moved: Vec<String> = (1..5)
            .map(|number| fs::read_to_string(name(number)))
            .collect::<Result<_, _>>()?;
        assert_eq!(moved, ["0", "1", "2", "3"]);
        assert!(!name(5).exists());
        fs::remove_file(name(2))?;
        fs::write(&log, "new")?;
        rotate_log(&log, 5)?;
        let names: Vec<Option<String>> = (0..5)
            .map(|number| fs::read_to_string(name(number)).ok())
            .collect();
        let expected = [None, Some("new"), Some("0"), None, Some("2")];
        assert_eq!(names, expected.map(|text| text.map(str::to_owned)));
        Ok(())
    }
}
