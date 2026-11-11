//! The two commands that feed a change card, run through the crate's git runner: the record of a
//! commit as [`super::SHOW_FORMAT`] prints it, and whether the commit only moved whitespace. Both
//! run the repository's git with none of its display settings: a configured signature check (which
//! would run the program `gpg.program` names), notes, colour, an external diff or a text
//! conversion, and with no fetch in a partial clone.

use std::path::Path;
use std::time::Duration;

use cortex_core::is_object_id;

use super::SHOW_FORMAT;
use crate::git_program::{Bounds, GitProgram, OpenError, RunFailure, Settings};

/// How long reading a commit's record may take: a few lines from git's object store.
pub const RECORD_WITHIN: Duration = Duration::from_secs(10);

/// How long the white-space check of one commit may take. It runs only for a commit that changed
/// at most `git.max_files_per_commit` files (30 by default); on Cortex's own history, 379 such
/// commits took at most 41 ms each (measured 2026-11-11), so 10 s leaves large files and a slow
/// disk two hundred times that.
pub const COSMETIC_WITHIN: Duration = Duration::from_secs(10);

/// What reading a commit's record may take: a commit's message is a few lines; one of 256 KiB is
/// not a message to show, and is refused rather than cut.
const RECORD: Bounds = Bounds {
    within: RECORD_WITHIN,
    max_bytes: 256 * 1024,
};

/// What the whitespace check may take. Its answer is only whether git printed anything, so the
/// output is cut at 64 KiB, which is already an answer.
const COSMETIC: Bounds = Bounds {
    within: COSMETIC_WITHIN,
    max_bytes: 64 * 1024,
};

/// Why a commit could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommitError {
    /// The commit is not given as a full object id.
    #[error("the commit is not an object id")]
    BadRequest,
    /// Git did not end in time and was stopped.
    #[error("git did not end in time")]
    TimedOut,
    /// A commit's record is longer than a message to show may be.
    #[error("the commit's record is longer than {} bytes", RECORD.max_bytes)]
    TooLarge,
    /// Git could not be run, or failed (the commit is not in the repository, say).
    #[error("running git: {0}")]
    Git(String),
}

/// A repository whose commits can be read for change cards.
pub struct CommitReader {
    /// Git for the repository.
    git: GitProgram,
    /// Whether the repository is a partial clone, where nothing may be fetched.
    is_partial: bool,
}

impl CommitReader {
    /// The reader for the repository whose top is `root`, in its canonical form.
    ///
    /// # Errors
    ///
    /// No repository or no git, a repository git may not be run in (a filter in its own
    /// configuration, a partial clone with a git that would fetch in it), or git does not answer.
    pub fn open(root: &Path) -> Result<Self, OpenError> {
        let (git, own) = GitProgram::open(root)?;
        Ok(Self {
            git,
            is_partial: own.is_partial,
        })
    }

    /// What `git show -s` prints of the commit `oid`, a full object id, with [`SHOW_FORMAT`]:
    /// the text [`super::read_card`] reads. Text that is not UTF-8 is read with the
    /// replacement character where it is not, since only the names, which are not kept, and
    /// the message, which is shown, could hold it.
    ///
    /// # Errors
    ///
    /// The id is not an object id, git does not end in time, prints more than a message to
    /// show may be, or fails.
    pub fn record(&self, oid: &str) -> Result<String, CommitError> {
        let args = [
            "show",
            "-s",
            "--no-show-signature",
            "--no-notes",
            "--no-color",
            "--encoding=UTF-8",
            SHOW_FORMAT,
        ];
        let output = self.run(&args, oid, RECORD)?.ok_or(CommitError::TooLarge)?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    /// Whether the commit `oid`, a full object id, only moved whitespace: whether `git show -w
    /// --stat` against its first parent prints nothing. A commit that changes nothing at all
    /// prints nothing too. A listing of more than 64 KiB is a change.
    ///
    /// # Errors
    ///
    /// The id is not an object id, git does not end in time, or fails.
    pub fn is_cosmetic(&self, oid: &str) -> Result<bool, CommitError> {
        let args = [
            "show",
            "-w",
            "--stat",
            "--no-textconv",
            "--no-ext-diff",
            "--no-renames",
            "--no-color",
            "--diff-merges=first-parent",
            "--ignore-submodules=none",
            "--format=",
        ];
        let output = self.run(&args, oid, COSMETIC)?;
        Ok(output.is_some_and(|text| text.is_empty()))
    }

    /// What git printed for `args` and then `oid`, within `bounds`; `None` when it printed more
    /// than `bounds.max_bytes`.
    fn run(
        &self,
        args: &[&str],
        oid: &str,
        bounds: Bounds,
    ) -> Result<Option<Vec<u8>>, CommitError> {
        if !is_object_id(oid) {
            return Err(CommitError::BadRequest);
        }
        let mut all = args.to_vec();
        all.push(oid);
        let options: &[&str] = if self.is_partial {
            &["--no-lazy-fetch"]
        } else {
            &[]
        };
        self.git
            .list(Settings::Isolated, options, &all, bounds)
            .map_err(|failure| match failure {
                RunFailure::TimedOut => CommitError::TimedOut,
                RunFailure::Failed(message) => CommitError::Git(message),
            })
    }
}
