//! Which commit last changed each line of a span: `git blame` of a range of one file at one
//! commit, run as the rest of this crate runs git (the `git` the `PATH` names outside the
//! repository, no system or global configuration, nothing fetched), and read from its porcelain
//! output ([`parse_porcelain`]).
//!
//! A [`Blamer`] is made once for a repository: opening it reads the repository's own
//! configuration and what git can be told, and refuses what blame must not run in. Each blame
//! after that is one git process, ended at the time the request gives it. Open a new one when the
//! repository's configuration may have changed.
//!
//! The blame ignores changes of whitespace (`-w`) and follows lines moved within a file (`-M`),
//! so a reformatting does not take a line from the commit that wrote it. When it is given a
//! date it reads no further back than that: lines that have not changed since are blamed on the
//! newest commit before the date, which is flagged a boundary. The commits a repository lists in
//! `.git-blame-ignore-revs` are skipped as git skips them, when that is a regular file in its top
//! holding only comments, blank lines and full object ids, which git would otherwise refuse
//! whole.
//!
//! A blame runs no command the repository names and opens no file outside it. Text conversion,
//! which git applies to a blamed file whose attributes name a driver, is turned off with
//! `--no-textconv`; a repository whose own configuration names a filter, or a list of commits to
//! skip other than the one above, is refused, since git opens that list before anything can
//! clear it; the mailmap files of the configuration are replaced by an empty one, and
//! `blame.showRoot` is pinned, which decides which commits are boundaries. In a partial clone
//! nothing is fetched, and with a git that cannot be told so the blame is refused.

mod ignore_list;
mod porcelain;

use std::path::Path;
use std::time::Duration;

use cortex_core::{RepoPath, is_object_id};

pub use porcelain::{Blame, BlameRun, BlamedCommit, Previous, parse_porcelain};

use crate::git_program::{
    Bounds, GitProgram, IGNORE_REVS_FILE, IgnoreList, NULL_DEVICE, OpenError, RunFailure, Settings,
};

/// The earliest date, in seconds since the Unix epoch (1973-03-03), that git reads as a time: git
/// takes a smaller number in a date for a day or a year, and a date of 0 for now, so a date limit
/// below this would cut the blame at the newest commit instead.
pub const MIN_SINCE_S: i64 = 100_000_000;

/// The most lines one blame covers. A longer span is refused, so that the answer stays inside
/// [`MAX_BLAME_BYTES`].
pub const MAX_BLAME_LINES: u32 = 500;

/// The most of git's output read, 1 MiB. A line costs its header (about 50 bytes), its text and,
/// the first time a commit appears, its details (about 450 bytes with a long summary). Five
/// hundred lines of 200 bytes each, every line from a commit of its own, come to about 350 KB,
/// so 1 MiB holds the longest span allowed with room for very long lines; an answer past it is
/// refused rather than read in part.
const MAX_BLAME_BYTES: u64 = 1024 * 1024;

/// Why a blame could not be made.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BlameError {
    /// The request cannot be a blame: what is wrong with it.
    #[error("the blame is not asked for properly: {0}")]
    BadRequest(&'static str),
    /// The repository's own configuration names `.git-blame-ignore-revs` as its list of commits to
    /// skip, and the file is not a regular file in its top holding only comments, blank lines
    /// and full object ids, so git would fail or read it through a link.
    #[error("the repository's list of commits to skip cannot be used")]
    UnusableIgnoreList,
    /// Git did not end within the time given and was stopped.
    #[error("git blame did not end in time")]
    TimedOut,
    /// Git printed more than [`MAX_BLAME_BYTES`].
    #[error("git blame printed more than {MAX_BLAME_BYTES} bytes")]
    TooLarge,
    /// Git could not be run, or failed (the file is not at the commit, say).
    #[error("running git blame: {0}")]
    Git(String),
    /// Git's output is not porcelain output, or does not cover the lines asked for.
    #[error("git blame's output cannot be read: {0}")]
    Malformed(String),
}

/// A span to blame.
#[derive(Debug, Clone, Copy)]
pub struct BlameRequest<'a> {
    /// The file, by its path at `at`.
    pub path: &'a RepoPath,
    /// The first line of the span, counted from 1.
    pub first_line: u32,
    /// The last line of the span, inclusive, at most [`MAX_BLAME_LINES`] lines after the first.
    pub last_line: u32,
    /// The commit whose file is blamed, as a full hexadecimal id.
    pub at: &'a str,
    /// The oldest commit time, in seconds since the Unix epoch, blame looks at, or none for no
    /// limit; lines older than this are blamed on the boundary commit. At least
    /// [`MIN_SINCE_S`].
    pub since_s: Option<i64>,
    /// How long git has, in milliseconds (`history.blame_timeout_ms`, default 2,000).
    pub timeout_ms: u64,
    /// Commits whose changes the blame looks through, as full hexadecimal ids (`--ignore-rev`):
    /// a line one of them changed is blamed on the commit that changed it before, git mapping the
    /// line through the ignored change.
    pub ignore_revs: &'a [String],
}

impl BlameRequest<'_> {
    /// Whether the request can be run.
    ///
    /// # Errors
    ///
    /// [`BlameError::BadRequest`]: no file, lines that are not a span of 1 to
    /// [`MAX_BLAME_LINES`] lines counted from 1, a commit, or a commit to ignore, that is not a
    /// full object id, or a date git would not read as a time.
    fn check(&self) -> Result<(), BlameError> {
        if self.path.as_str().is_empty() {
            return Err(BlameError::BadRequest("the path is the repository's root"));
        }
        let is_span = self.first_line >= 1
            && self.first_line <= self.last_line
            && self.last_line - self.first_line < MAX_BLAME_LINES;
        if !is_span {
            return Err(BlameError::BadRequest("the lines are not a span to blame"));
        }
        if !is_object_id(self.at) {
            return Err(BlameError::BadRequest("the commit is not an object id"));
        }
        if !self.ignore_revs.iter().all(|oid| is_object_id(oid)) {
            return Err(BlameError::BadRequest(
                "a commit to ignore is not an object id",
            ));
        }
        if self.since_s.is_some_and(|since_s| since_s < MIN_SINCE_S) {
            return Err(BlameError::BadRequest("the date is too early to be a time"));
        }
        Ok(())
    }
}

/// A repository that can be blamed in: what opening it found out, kept for every blame.
pub struct Blamer {
    /// Git for the repository.
    git: GitProgram,
    /// Whether the repository is a partial clone, where nothing may be fetched.
    is_partial: bool,
    /// Whether the repository's own configuration names the top `.git-blame-ignore-revs`, which
    /// git then opens whether or not a blame asks for it.
    config_names_ignore_list: bool,
}

impl Blamer {
    /// The blamer for the repository whose top is `root`, in its canonical form. It asks git
    /// about the repository's own configuration and the git's version once, within the
    /// time those short questions are given.
    ///
    /// # Errors
    ///
    /// No repository or no git, a repository git may not be run in (a filter in its own
    /// configuration, a partial clone with a git that would fetch in it, a list of commits to skip
    /// that is not the one in its top), or git does not answer.
    pub fn open(root: &Path) -> Result<Self, OpenError> {
        let (git, own) = GitProgram::open(root)?;
        if own.ignore_list == IgnoreList::Other {
            return Err(OpenError::NamesIgnoreList);
        }
        Ok(Self {
            git,
            is_partial: own.is_partial,
            config_names_ignore_list: own.ignore_list == IgnoreList::Top,
        })
    }

    /// Which commit owns each line of the span `request` names. One git process runs, and is
    /// stopped at the request's `timeout_ms`.
    ///
    /// # Errors
    ///
    /// The request is not one that can be blamed, git does not end in time
    /// ([`BlameError::TimedOut`]) or prints more than [`MAX_BLAME_BYTES`], fails, or prints
    /// something that is not a blame of the span.
    pub fn blame(&self, request: &BlameRequest<'_>) -> Result<Blame, BlameError> {
        request.check()?;
        let has_ignore_list = ignore_list::is_usable(self.git.root());
        if self.config_names_ignore_list && !has_ignore_list {
            return Err(BlameError::UnusableIgnoreList);
        }

        let range = format!("{},{}", request.first_line, request.last_line);
        let pins = pinned_settings();
        let mut options: Vec<&str> = pins.iter().map(String::as_str).collect();
        // A partial clone's missing objects are not fetched: a git older than 2.45 was refused
        // when the blamer was opened.
        if self.is_partial {
            options.push("--no-lazy-fetch");
        }
        let mut args = vec![
            "blame",
            "--porcelain",
            "-w",
            "-M",
            "--no-textconv",
            "-L",
            &range,
        ];
        if has_ignore_list {
            args.extend(["--ignore-revs-file", IGNORE_REVS_FILE]);
        }
        for oid in request.ignore_revs {
            args.extend(["--ignore-rev", oid.as_str()]);
        }
        // Git's own date format, a time in seconds and a zone.
        let since = request
            .since_s
            .map(|since_s| format!("--since={since_s} +0000"));
        args.extend(since.as_deref());
        args.extend([request.at, "--", request.path.as_str()]);

        let bounds = Bounds {
            within: Duration::from_millis(request.timeout_ms),
            max_bytes: MAX_BLAME_BYTES,
        };
        let output = self
            .git
            .list(Settings::Isolated, &options, &args, bounds)
            .map_err(|failure| match failure {
                RunFailure::TimedOut => BlameError::TimedOut,
                RunFailure::Failed(message) => BlameError::Git(message),
            })?
            .ok_or(BlameError::TooLarge)?;
        let blame = parse_porcelain(&output)?;
        check_covers(&blame, request)?;
        Ok(blame)
    }
}

/// The settings of the repository's own configuration that a blame overrides, as options before
/// git's command: the mailmap file and blob, which git reads for a name and which could be a file
/// anywhere, become an empty file and none; and `blame.showRoot` is off, since it decides whether
/// a root commit is a boundary.
fn pinned_settings() -> Vec<String> {
    [
        format!("mailmap.file={NULL_DEVICE}"),
        "mailmap.blob=".to_owned(),
        "blame.showRoot=false".to_owned(),
    ]
    .into_iter()
    .flat_map(|setting| ["-c".to_owned(), setting])
    .collect()
}

/// Whether the runs of `blame` are exactly the lines `request` asked for, each once.
///
/// # Errors
///
/// [`BlameError::Malformed`] when they are not.
fn check_covers(blame: &Blame, request: &BlameRequest<'_>) -> Result<(), BlameError> {
    let first = blame.runs.first().map(|run| run.start);
    let last = blame.runs.last().map(|run| run.end);
    let lines: u64 = blame
        .runs
        .iter()
        .map(|run| u64::from(run.end.saturating_sub(run.start)) + 1)
        .sum();
    let wanted = u64::from(request.last_line.saturating_sub(request.first_line)) + 1;
    if first == Some(request.first_line) && last == Some(request.last_line) && lines == wanted {
        Ok(())
    } else {
        Err(BlameError::Malformed(
            "the lines blamed are not the lines asked for".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overrides are single-valued settings given on the command line, which win over the
    /// repository's own file: each is one `-c` and its setting.
    #[test]
    fn the_repositorys_mailmap_and_root_setting_are_overridden() {
        let pins = pinned_settings();
        let expected = [
            "-c",
            &format!("mailmap.file={NULL_DEVICE}"),
            "-c",
            "mailmap.blob=",
            "-c",
            "blame.showRoot=false",
        ];
        assert_eq!(pins, expected);
    }
}
