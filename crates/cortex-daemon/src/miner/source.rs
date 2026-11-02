//! Which program reads the history: `gix`, opened for each read and dropped after it, so that no
//! file of the repository stays open between looks (on Windows, git cannot remove a file another
//! process holds open, as it must when it packs objects); or the `git` program, once `gix` failed
//! to open or read the repository where the program did not, kept from then on.

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
    let error = match GixHistory::open(root) {
        Ok(history) => match read(&history) {
            Ok(value) => return Ok(Some(value)),
            Err(error) => error,
        },
        Err(GitError::NoRepository(_)) => return Ok(None),
        Err(error) => error,
    };
    let why = error.to_string();
    let either = |program: GitError| format!("{why}; the git program could not either: {program}");
    let history = CliHistory::open(root).map_err(either)?;
    let value = read(&history).map_err(either)?;
    tracing::warn!(error = %why, "gix cannot read the history; the git program reads it");
    *program = Some(Program { history, why });
    Ok(Some(value))
}
