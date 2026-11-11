//! The files a repository's work tree holds changed against HEAD, as `git status` lists them:
//! what the user, or an agent, is working on and has not committed. Read with the `git` program
//! with no optional locks, so that a look never makes the user's own git wait for it, and bounded
//! in time and in size.
//!
//! A look runs no command the repository names and fetches nothing. Git passes a file through its
//! filter when it has to read the file to tell whether it changed, and the repository's own
//! configuration may name the filter's command; a repository's folder is content Cortex does not
//! trust, so one whose own configuration names a filter is not looked at. Nor is one whose own
//! configuration includes another file (`include.path`, `includeIf`), which git would read
//! wherever it is, a network share included, before any look could tell what it names. A filter the user's own
//! configuration names, one the user installed, runs as it does for the user's own git. In a
//! partial clone git would fetch from the remote the configuration names what the clone left out
//! and a look needs; it is told not to, which git older than 2.45 cannot be, so with such a git a
//! partial clone is not looked at. No file system monitor runs, no submodule is looked into, and
//! the index is not written, so no hook that a write of it would start runs either.
//!
//! The top `.gitignore`, when its only change since HEAD is the block `cortex init` adds at its end,
//! is Cortex's own work, not the user's, and is left out; anything that cannot tell so leaves it
//! in.

use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use cortex_core::{RepoPath, without_ignore_block};

use crate::git_program::{Bounds, GitProgram, OwnConfig, Refusal, Settings, Start};
use crate::history::has_repository;

/// How long git has to list the work tree's changes: a large work tree takes it a second or two.
const STATUS_WITHIN: Duration = Duration::from_secs(10);

/// The most of git's listing read, 4 MiB, a hundred thousand paths of forty bytes; a listing
/// longer than that is a mass change, not read at all.
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;

/// The repository's top `.gitignore`, the one `cortex init` adds its lines to.
const GITIGNORE: &str = ".gitignore";

/// The most of a `.gitignore` read to tell whether Cortex's lines are its only change, as much as
/// git answers of a file at HEAD: a longer one is left in the set.
const MAX_GITIGNORE_BYTES: u64 = 64 * 1024;

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
    /// The repository's own configuration includes another file, which git would read wherever
    /// it is.
    #[error("the repository's own git configuration includes another file")]
    Includes,
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
/// No repository at `root`, no git program, a filter named or a file included in the repository's
/// own configuration, a partial clone with a git that would fetch in it, a mass change, or git does not list the
/// changes, as [`WorktreeError`] says.
pub fn uncommitted_files(root: &Path) -> Result<BTreeSet<RepoPath>, WorktreeError> {
    if !has_repository(root) {
        return Err(WorktreeError::NoRepository);
    }
    let git = GitProgram::start(root).map_err(|start| match start {
        Start::NoGit => WorktreeError::NoGit,
        Start::Includes => WorktreeError::Includes,
        Start::Failed(message) => WorktreeError::Status(message),
    })?;
    let own = git.own_config().map_err(WorktreeError::Status)?;
    may_look(own, || git.can_refuse_fetching())?;
    Ok(without_own_ignores(&git, listed(&git, MAX_STATUS_BYTES)?))
}

/// `changed` without the top `.gitignore` when its only change since HEAD is the block `cortex
/// init` adds at its end.
fn without_own_ignores(git: &GitProgram, mut changed: BTreeSet<RepoPath>) -> BTreeSet<RepoPath> {
    let Ok(path) = RepoPath::new(GITIGNORE) else {
        return changed;
    };
    if changed.contains(&path) && is_only_init_s_block(git) {
        changed.remove(&path);
    }
    changed
}

/// Whether the work tree's `.gitignore`, a file and not a link, with the block init adds at its
/// end taken off, is the one HEAD holds, or empty where HEAD holds none, line endings at its end
/// aside: false whenever git or the file cannot tell.
fn is_only_init_s_block(git: &GitProgram) -> bool {
    let Some(now) = work_tree_text(&git.root().join(GITIGNORE)) else {
        return false;
    };
    let Some(before) = without_ignore_block(&now) else {
        return false;
    };
    let committed = match git.ask(&["rev-parse", "--verify", "--quiet", "HEAD:.gitignore"]) {
        Ok(None) => String::new(),
        Ok(Some(id)) => match git.ask(&["cat-file", "blob", id.trim()]) {
            Ok(Some(text)) => text,
            Ok(None) | Err(_) => return false,
        },
        Err(_) => return false,
    };
    before.trim_end_matches(['\r', '\n']) == committed.trim_end_matches(['\r', '\n'])
}

/// The text of the file at `path`, read whole when it is a file of at most
/// [`MAX_GITIGNORE_BYTES`] holding UTF-8; none for a link, which may lead out of the repository,
/// or anything else.
fn work_tree_text(path: &Path) -> Option<String> {
    let kind = std::fs::symlink_metadata(path).ok()?;
    if !kind.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_GITIGNORE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > MAX_GITIGNORE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
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
    match own
        .refusal(can_refuse_fetching)
        .map_err(WorktreeError::Status)?
    {
        Some(Refusal::Filter) => Err(WorktreeError::Refused),
        Some(Refusal::WouldFetch) => Err(WorktreeError::WouldFetch),
        None => Ok(()),
    }
}

/// What git's status lists of the work tree, read within [`STATUS_WITHIN`] and `max_bytes`.
///
/// # Errors
///
/// Git lists more than `max_bytes` ([`WorktreeError::TooMany`]), or does not list the changes.
fn listed(git: &GitProgram, max_bytes: u64) -> Result<BTreeSet<RepoPath>, WorktreeError> {
    let listing = git
        .list(
            Settings::User,
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
            Bounds {
                within: STATUS_WITHIN,
                max_bytes,
            },
        )
        .map_err(|failure| WorktreeError::Status(failure.to_string()))?
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
            ..OwnConfig::default()
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
        let git = GitProgram::start(root)?;
        assert!(matches!(listed(&git, 26), Err(WorktreeError::TooMany)));
        let names: Vec<String> = listed(&git, 27)?
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt"]);
        Ok(())
    }
}
