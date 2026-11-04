//! The files a repository's work tree holds changed against HEAD, as `git status` lists them:
//! what the user, or an agent, is working on and has not committed. Read with the `git` program
//! with no optional locks, so that a look never makes the user's own git wait for it, and bounded
//! in time and in size.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use cortex_core::RepoPath;

use crate::git_program::GitProgram;

/// How long git has to list the work tree's changes: a large work tree takes it a second or two.
const STATUS_WITHIN: Duration = Duration::from_secs(10);

/// The most of git's listing read, 4 MiB, a hundred thousand paths of forty bytes; a listing cut
/// short is not read at all.
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;

/// Why the work tree's changes could not be read.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    /// The `PATH` names no `git` program outside the repository.
    #[error("no git program is on the PATH")]
    NoGit,
    /// Git did not answer in time, failed, or listed more than [`MAX_STATUS_BYTES`].
    #[error("listing the work tree's changes: {0}")]
    Status(String),
}

/// The files of the work tree at `root`, a repository's top in its canonical form, that differ
/// from HEAD or that git does not track and does not ignore, by path. A file deleted from the work
/// tree is left out, since there is nothing of it to read, and so is a path that is not UTF-8.
///
/// # Errors
///
/// No git program, or git does not list the changes, as [`WorktreeError`] says.
pub fn uncommitted_files(root: &Path) -> Result<BTreeSet<RepoPath>, WorktreeError> {
    let git = GitProgram::find(root).ok_or(WorktreeError::NoGit)?;
    let listing = git
        .list_as_user(
            &["--no-optional-locks", "-c", "core.fsmonitor=false"],
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--no-renames",
                "--ignore-submodules=all",
            ],
            (STATUS_WITHIN, MAX_STATUS_BYTES),
        )
        .map_err(WorktreeError::Status)?;
    Ok(held_in_work_tree(&listing))
}

/// The paths of `listing`, from `git status --porcelain=v1 -z --no-renames`, whose files the work
/// tree holds. Each entry is `XY path` and a NUL, X the index's state of the file and Y the work
/// tree's; with renames off, no entry names a second path. A file is gone when the work tree
/// deleted it (Y `D`), or when it is staged for deletion and nothing has come back (`D `).
fn held_in_work_tree(listing: &[u8]) -> BTreeSet<RepoPath> {
    listing
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let (state, path) = (entry.get(..2)?, entry.get(3..)?);
            let is_gone = state.get(1) == Some(&b'D') || state == b"D ";
            if is_gone {
                return None;
            }
            RepoPath::new(std::str::from_utf8(path).ok()?).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked by hand from git's porcelain format: a file changed in the work tree, one untracked,
    /// one unmerged and one staged are held; one staged for deletion, one deleted from the work
    /// tree and one added then deleted are not; an empty trailing entry and a path that is no
    /// repository path are left out.
    #[test]
    fn the_work_tree_holds_what_was_not_deleted() {
        let listing =
            b" M src/a.py\0D  b.py\0?? c d.py\0 D d.py\0UU e.py\0AD f.py\0A  g.py\0?? /abs\0";
        let held: Vec<String> = held_in_work_tree(listing)
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect();
        assert_eq!(held, ["c d.py", "e.py", "g.py", "src/a.py"]);
    }
}
