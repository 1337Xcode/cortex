//! The files a repository's work tree holds changed against HEAD, as `git status` lists them:
//! what the user, or an agent, is working on and has not committed. Read with the `git` program
//! with no optional locks, so that a look never makes the user's own git wait for it, and bounded
//! in time and in size.
//!
//! A look runs no command the repository names and fetches nothing. Git passes a file through its
//! filter when it has to read the file to tell whether it changed, and the repository's own
//! configuration may name the filter's command; a repository's folder is content Cortex does not
//! trust, so one whose own configuration names a filter is not looked at. A filter the user's own
//! configuration names, one the user installed, runs as it does for the user's own git. In a
//! partial clone git would fetch from the remote the configuration names what the clone left out
//! and a look needs; it is told not to, which git older than 2.45 cannot be, so with such a git a
//! partial clone is not looked at. No file system monitor runs, no submodule is looked into, and
//! the index is not written, so no hook that a write of it would start runs either.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use cortex_core::RepoPath;

use crate::git_program::{GitProgram, OwnConfig};
use crate::history::has_repository;

/// How long git has to list the work tree's changes: a large work tree takes it a second or two.
const STATUS_WITHIN: Duration = Duration::from_secs(10);

/// The most of git's listing read, 4 MiB, a hundred thousand paths of forty bytes; a listing
/// longer than that is a mass change, not read at all.
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;

/// Why the work tree's changes could not be read.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    /// The `PATH` names no `git` program outside the repository.
    #[error("no git program is on the PATH")]
    NoGit,
    /// The folder holds no `.git`, so it is not a git repository's top: there is no diff to read.
    #[error("no git repository is at the root")]
    NoRepository,
    /// The repository's own configuration names a filter, whose command a look would run.
    #[error("the repository's own git configuration names a filter, which a look would run")]
    Refused,
    /// The repository is a partial clone, and git, older than 2.45, cannot be told not to fetch
    /// what the clone left out.
    #[error("the repository is a partial clone, and git older than 2.45 could fetch in it")]
    WouldFetch,
    /// Git listed more than [`MAX_STATUS_BYTES`] of changes: a mass change.
    #[error("git listed more than {MAX_STATUS_BYTES} bytes of changes")]
    TooMany,
    /// Git did not answer in time, or failed.
    #[error("listing the work tree's changes: {0}")]
    Status(String),
}

/// The files of the work tree at `root`, a repository's top in its canonical form, that differ
/// from HEAD or that git does not track and does not ignore, by path. A file deleted from the work
/// tree is left out, since there is nothing of it to read, and so is a path that is not UTF-8.
///
/// # Errors
///
/// No repository at `root`, no git program, a filter named in the repository's own configuration,
/// a partial clone with a git that would fetch in it, a mass change, or git does not list the
/// changes, as [`WorktreeError`] says.
pub fn uncommitted_files(root: &Path) -> Result<BTreeSet<RepoPath>, WorktreeError> {
    if !has_repository(root) {
        return Err(WorktreeError::NoRepository);
    }
    let git = GitProgram::find(root).ok_or(WorktreeError::NoGit)?;
    let own = git.own_config().map_err(WorktreeError::Status)?;
    may_look(own, || git.can_refuse_fetching())?;
    listed(&git, MAX_STATUS_BYTES)
}

/// Whether a repository whose own configuration says `own` may be looked at, the git that would
/// look answering `can_refuse_fetching`, which is asked only of a partial clone.
///
/// # Errors
///
/// [`WorktreeError::Refused`] for a filter named, [`WorktreeError::WouldFetch`] for a partial
/// clone with a git that cannot be told not to fetch, and [`WorktreeError::Status`] when git does
/// not say which it is.
fn may_look(
    own: OwnConfig,
    can_refuse_fetching: impl FnOnce() -> Result<bool, String>,
) -> Result<(), WorktreeError> {
    if own.names_filter {
        return Err(WorktreeError::Refused);
    }
    // INVARIANT: local-only. Git 2.45 and later read `GIT_NO_LAZY_FETCH`, which every git Cortex
    // runs is given; an older git in a partial clone would fetch what a look needs.
    if own.is_partial && !can_refuse_fetching().map_err(WorktreeError::Status)? {
        return Err(WorktreeError::WouldFetch);
    }
    Ok(())
}

/// What git's status lists of the work tree, read within [`STATUS_WITHIN`] and `max_bytes`.
///
/// # Errors
///
/// Git lists more than `max_bytes` ([`WorktreeError::TooMany`]), or does not list the changes.
fn listed(git: &GitProgram, max_bytes: u64) -> Result<BTreeSet<RepoPath>, WorktreeError> {
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
                // Cortex's own folder is not the user's work, even where git does not ignore it.
                "--",
                ".",
                ":(exclude).cortex",
            ],
            (STATUS_WITHIN, max_bytes),
        )
        .map_err(WorktreeError::Status)?
        .ok_or(WorktreeError::TooMany)?;
    Ok(held_in_work_tree(&listing))
}

/// The paths of `listing`, from `git status --porcelain=v1 -z --no-renames`, whose files the work
/// tree holds. Each entry is `XY path` and a NUL, X the index's state of the file and Y the work
/// tree's; with renames off, no entry names a second path. A file a merge left in conflict (`U` on
/// either side, `AA` or `DD`) is held unless both sides deleted it (`DD`). Any other file is gone
/// when the work tree deleted it (Y `D`), or when it is staged for deletion and nothing has come
/// back (`D `).
fn held_in_work_tree(listing: &[u8]) -> BTreeSet<RepoPath> {
    listing
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let (state, path) = (entry.get(..2)?, entry.get(3..)?);
            let is_unmerged = state.contains(&b'U') || state == b"AA" || state == b"DD";
            let is_gone = if is_unmerged {
                state == b"DD"
            } else {
                state.get(1) == Some(&b'D') || state == b"D "
            };
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
    /// one staged, and those a merge left in conflict (both changed, deleted by them, deleted by
    /// us, both added) are held; one staged for deletion, one deleted from the work tree, one added
    /// then deleted and one both sides of a merge deleted are not; an empty trailing entry and a
    /// path that is no repository path are left out.
    #[test]
    fn the_work_tree_holds_what_was_not_deleted() {
        let listing =
            b" M src/a.py\0D  b.py\0?? c d.py\0 D d.py\0UU e.py\0AD f.py\0A  g.py\0?? /abs\0\
            UD h.py\0DU i.py\0DD j.py\0AA k.py\0";
        let held: Vec<String> = held_in_work_tree(listing)
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect();
        assert_eq!(
            held,
            ["c d.py", "e.py", "g.py", "h.py", "i.py", "k.py", "src/a.py"]
        );
    }

    /// Which repositories may be looked at: one naming a filter never, whatever else holds; a
    /// partial clone only with a git that can be told not to fetch, and not when git does not say;
    /// a full clone always, git's version never asked.
    #[test]
    fn a_look_is_refused_where_it_would_run_or_fetch() {
        let own = |names_filter, is_partial| OwnConfig {
            names_filter,
            is_partial,
        };
        let never_asked = || -> Result<bool, String> { Err("asked".to_owned()) };
        assert!(matches!(
            may_look(own(true, true), || Ok(true)),
            Err(WorktreeError::Refused)
        ));
        assert!(matches!(
            may_look(own(true, false), never_asked),
            Err(WorktreeError::Refused)
        ));
        assert!(matches!(
            may_look(own(false, true), || Ok(false)),
            Err(WorktreeError::WouldFetch)
        ));
        assert!(matches!(
            may_look(own(false, true), never_asked),
            Err(WorktreeError::Status(_))
        ));
        assert!(may_look(own(false, true), || Ok(true)).is_ok());
        assert!(may_look(own(false, false), never_asked).is_ok());
    }

    /// A listing longer than the bytes allowed is a mass change, not a failure. Three untracked
    /// files list `?? a.txt`, `?? b.txt` and `?? c.txt`, each with its NUL: 9 bytes each, 27 in
    /// all, more than 26 allowed and within 27.
    #[test]
    fn a_listing_past_its_bytes_is_a_mass_change() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let root = folder.path();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()?;
        assert!(init.success());
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(root.join(name), "x\n")?;
        }
        let git = GitProgram::find(root).ok_or("a git program")?;
        assert!(matches!(listed(&git, 26), Err(WorktreeError::TooMany)));
        let names: Vec<String> = listed(&git, 27)?
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt"]);
        Ok(())
    }
}
