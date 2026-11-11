//! The history read by running the `git` program ([`GitProgram`]): the fallback when `gix` cannot
//! read a repository, and in the tests the reference `gix`'s reading is checked against. Every
//! option that shapes git's listing is on its command line, so the repository's own settings change
//! nothing it lists, and the commits are read in the order and with the checks of the `gix` source.
//!
//! Git's own rename search decides renames here, so an edited file over 1 MiB moved is a rename,
//! which `gix` takes as a deletion and an addition; elsewhere the two read the same.

use std::io::Read as _;
use std::path::Path;

use cortex_core::canonical_root;

use crate::git_log::{self, FORMAT};
use crate::git_program::{GitProgram, LISTING, Settings, Start};
use crate::history::{
    GitError, HistorySource, Mined, MinedCommit, PathChange, Window, has_repository,
};

/// The most of a shallow clone's list of cut commits read: 25,000 ids. A longer list fails the
/// read rather than leaving cut commits uncut.
const MAX_SHALLOW_BYTES: u64 = 1024 * 1024;

/// A repository's history read by running git.
pub struct CliHistory {
    /// Git for the repository.
    git: GitProgram,
}

impl CliHistory {
    /// The history of the git repository whose work tree's top is `root`, read with the git the
    /// `PATH` names outside it.
    ///
    /// # Errors
    ///
    /// [`GitError::NoRepository`] when `root` has no `.git`; [`GitError::Open`] when git is not on
    /// the `PATH`, does not answer, names another folder as the work tree's top, or the
    /// repository's own configuration includes a file, which git would read wherever it is.
    pub fn open(root: &Path) -> Result<Self, GitError> {
        if !has_repository(root) {
            return Err(GitError::NoRepository(root.to_path_buf()));
        }
        let root = canonical_root(root).map_err(|error| GitError::Open(error.to_string()))?;
        let git =
            GitProgram::start(&root).map_err(|start: Start| GitError::Open(start.to_string()))?;
        let top = git
            .ask(&["rev-parse", "--show-toplevel"])
            .map_err(GitError::Open)?
            .ok_or_else(|| GitError::Open("git names no work tree".to_owned()))?;
        let is_root = canonical_root(Path::new(top.trim_end())).is_ok_and(|top| top == root);
        if !is_root {
            return Err(GitError::Open(format!(
                "its work tree is not {}",
                root.display()
            )));
        }
        Ok(Self { git })
    }

    /// The commits a shallow clone cut, whose parents it does not hold; none in a full clone.
    ///
    /// # Errors
    ///
    /// Git does not say where the list is, or it cannot be read or is longer than
    /// [`MAX_SHALLOW_BYTES`].
    fn shallow_cut(&self) -> Result<Vec<String>, GitError> {
        let failed =
            |message: String| GitError::Walk(format!("reading the shallow commits: {message}"));
        let path = self
            .git
            .ask(&["rev-parse", "--git-path", "shallow"])
            .map_err(failed)?
            .ok_or_else(|| failed("git names no file".to_owned()))?;
        let file = self.git.root().join(path.trim_end());
        let opened = match std::fs::File::open(&file) {
            Ok(opened) => opened,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(failed(error.to_string())),
        };
        let mut text = String::new();
        opened
            .take(MAX_SHALLOW_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| failed(error.to_string()))?;
        if text.len() as u64 > MAX_SHALLOW_BYTES {
            return Err(failed(format!(
                "the list is over {MAX_SHALLOW_BYTES} bytes"
            )));
        }
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Git's listing of the commits `window` covers, from HEAD along the first-parent line: as
    /// many as the window takes and one more, so that the stop commit shows when it comes right
    /// after them (a shallow clone's cut commit, which is left out, is the last on the line). The
    /// window's oldest time is checked as the commits are read, as the `gix` source checks it,
    /// so that a stop commit older than the window is seen.
    ///
    /// # Errors
    ///
    /// Git does not end in time, fails, or lists more than it may.
    fn listing(&self, window: &Window, is_partial: bool) -> Result<Vec<u8>, GitError> {
        let max_count = format!("--max-count={}", window.max_commits.saturating_add(1));
        // Git compares contents for renames as `gix` does: at most `max_files` squared pairs, and
        // in a partial clone none, which would need contents it does not hold.
        let renames = if is_partial {
            "--find-renames=100%"
        } else {
            "--find-renames=50%"
        };
        let limit = format!("-l{}", window.max_files.max(1));
        // The commit to read from is an id the mining resolved, hexadecimal; anything else names
        // no commit, and never reaches git as an option.
        let from = match window.from.as_deref() {
            Some(id) if id.bytes().all(|byte| byte.is_ascii_hexdigit()) => id,
            Some(id) => return Err(GitError::Head(format!("{id} names no commit"))),
            None => "HEAD",
        };
        // Git older than 2.45 refuses `--no-lazy-fetch`, and so reads no partial clone.
        let options: &[&str] = if is_partial {
            &["--no-lazy-fetch", "-c", "log.showRoot=true"]
        } else {
            &["-c", "log.showRoot=true"]
        };
        let args = [
            "log",
            "--first-parent",
            "--diff-merges=first-parent",
            renames,
            &limit,
            "--name-status",
            "-z",
            "--ignore-submodules=none",
            "--no-color",
            "--no-decorate",
            "--no-show-signature",
            "--no-relative",
            "--no-ext-diff",
            "--no-textconv",
            FORMAT,
            &max_count,
            from,
            "--",
        ];
        self.git
            .list(Settings::Isolated, options, &args, LISTING)
            .map_err(|failure| GitError::Walk(failure.to_string()))?
            .ok_or_else(|| {
                GitError::Walk(format!("git listed more than {} bytes", LISTING.max_bytes))
            })
    }
}

impl HistorySource for CliHistory {
    fn head(&self) -> Result<Option<String>, GitError> {
        let ask = |args: &[&str]| self.git.ask(args).map_err(GitError::Head);
        if let Some(id) = ask(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])? {
            return Ok(Some(id.trim_end().to_owned()));
        }
        // No commit: HEAD's branch has none yet, which is no error, or HEAD names something that
        // is not a commit or is missing, which is.
        let branch = ask(&["symbolic-ref", "--quiet", "HEAD"])?;
        let is_unborn = match branch {
            Some(branch) => ask(&[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                branch.trim_end(),
            ])?
            .is_none(),
            None => false,
        };
        if is_unborn {
            Ok(None)
        } else {
            Err(GitError::Head("HEAD names no commit".to_owned()))
        }
    }

    fn read(&self, window: &Window) -> Result<Mined, GitError> {
        let mut mined = Mined {
            // A `promisor` git cannot read as a boolean counts as set, as the `gix` source counts
            // it: comparing contents in a partial clone fails the read, comparing none only finds
            // fewer renames.
            is_partial: self.git.own_config().map_err(GitError::Open)?.is_partial,
            ..Mined::default()
        };
        if window.from.is_none() && self.head()?.is_none() {
            return Ok(mined);
        }
        let cut = self.shallow_cut()?;
        mined.is_shallow = !cut.is_empty();
        let listing = self.listing(window, mined.is_partial)?;
        let listed = git_log::parse(&listing).map_err(|error| GitError::Walk(error.to_string()))?;
        for commit in listed {
            if window.stop_at.as_deref() == Some(commit.id.as_str()) {
                mined.is_stop_reached = true;
                break;
            }
            if mined.commits.len() >= window.max_commits {
                break;
            }
            if commit.time_s < window.oldest_s {
                break;
            }
            if cut.contains(&commit.id) {
                continue;
            }
            let changes = (commit.changes.len() <= window.max_files).then(|| {
                let mut changes: Vec<PathChange> = commit
                    .changes
                    .iter()
                    .filter_map(|(path, from)| PathChange::from_git(path, from.as_deref()))
                    .collect();
                changes.sort();
                changes
            });
            mined.commits.push(MinedCommit {
                id: commit.id,
                time_s: commit.time_s,
                changes,
            });
        }
        Ok(mined)
    }
}
