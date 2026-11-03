//! Which program reads the history: `gix`, opened for each read and dropped after it, so that no
//! file of the repository stays open between looks (on Windows, git cannot remove a file another
//! process holds open, as it must when it packs objects); or the `git` program, once `gix` failed
//! to open or read the repository where the program did not, kept from then on. A `gix` that stops
//! on an internal error, a panic, has failed as much as one that returns an error.

use std::panic::AssertUnwindSafe;
use std::path::Path;

use cortex_git::{CliHistory, GitError, GixHistory, HistorySource, has_repository};

/// The `git` program reading the history, once `gix` could not.
pub(super) struct Program {
    /// The history the program reads.
    history: CliHistory,
    /// Why `gix` could not.
    why: String,
}

impl Program {
    /// How the history is read, in words, for the daemon's status.
    pub(super) fn detail(&self) -> String {
        format!("read with the git program, as gix could not: {}", self.why)
    }
}

/// Runs `read` on the history of the repository whose top is `root`, or gives `None` when `root`
/// holds no repository, which also forgets the program in `program`. `gix` reads it, opened for
/// this read alone, until it fails where the `git` program does not; the program, kept in
/// `program`, reads it from then on.
///
/// # Errors
///
/// The read failed, with `gix`'s reason and the program's when both were tried, or the program's
/// alone once it reads the history.
pub(super) fn read_history<T>(
    root: &Path,
    program: &mut Option<Program>,
    read: impl Fn(&dyn HistorySource) -> Result<T, GitError>,
) -> Result<Option<T>, String> {
    if !has_repository(root) {
        *program = None;
        return Ok(None);
    }
    if let Some(current) = program {
        return read(&current.history).map(Some).map_err(|error| {
            format!(
                "{error} (with the git program, as gix could not: {})",
                current.why
            )
        });
    }
    let tried = std::panic::catch_unwind(AssertUnwindSafe(|| match GixHistory::open(root) {
        Ok(history) => read(&history).map(Some),
        Err(GitError::NoRepository(_)) => Ok(None),
        Err(error) => Err(error),
    }));
    let why = match tried {
        Ok(Ok(value)) => return Ok(value),
        Ok(Err(error)) => error.to_string(),
        Err(_) => "gix stopped on an internal error, logged as a panic".to_owned(),
    };
    let either = |program: GitError| format!("{why}; the git program could not either: {program}");
    let history = CliHistory::open(root).map_err(either)?;
    let value = read(&history).map_err(either)?;
    tracing::warn!(error = %why, "gix cannot read the history; the git program reads it");
    *program = Some(Program { history, why });
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A `gix` read that panics, as one stopped by a repository it cannot cope with would, falls
    /// back to the `git` program as a failed read does: the second read, the program's, gives the
    /// value, and the program is kept with why `gix` could not.
    #[test]
    #[allow(
        clippy::panic,
        reason = "the panic stands for one inside gix, which the read must survive"
    )]
    fn a_gix_read_that_panics_falls_back_to_the_program() -> TestResult {
        let folder = tempfile::tempdir()?;
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(folder.path())
            .status()?;
        assert!(status.success());
        let reads = AtomicUsize::new(0);
        let mut program = None;
        let value = read_history(folder.path(), &mut program, |_history| {
            if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("a read gix cannot finish");
            }
            Ok(7)
        })?;
        assert_eq!((value, reads.load(Ordering::SeqCst)), (Some(7), 2));
        let detail = program.as_ref().map(Program::detail);
        assert_eq!(
            detail.as_deref(),
            Some(
                "read with the git program, as gix could not: gix stopped on an internal error, \
                 logged as a panic"
            )
        );
        Ok(())
    }
}
