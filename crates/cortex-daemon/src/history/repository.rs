//! The repository as the history worker reads it: `gix` for HEAD, the files HEAD holds and the
//! paths a commit changed, opened for each job and closed after it, so that no file of the
//! repository stays open between jobs (on Windows git cannot remove a pack file another process
//! holds open); and the `git` program for blame and a commit's record. The program's two handles,
//! a [`Blamer`] and a [`CommitReader`], read the repository's own configuration when they are
//! opened, so they are opened again whenever that may have changed:
//!
//! - when HEAD's commit is not the one they were opened at (a checkout, a pull or a commit can
//!   come with a changed configuration, and costs two short git runs at most once per move), and
//! - when a configuration file of the repository, `config` in its shared git folder or
//!   `config.worktree` in the working tree's own, has another size or modification time than when
//!   they were opened. A `.git` file, as a linked working tree or a submodule has, is followed to
//!   the folders it names (`cortex_core::git_layout`).
//!
//! A repository git must not be run in (its own configuration names a filter, a partial clone
//! with a git that would fetch, a blame ignore list outside its top), or that has no git or no
//! repository, is refused, and the refusal holds for the daemon's life: nothing in it is run
//! again. Any other failure ends the job that met it only.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use cortex_core::git_layout;
use cortex_git::blame::Blamer;
use cortex_git::card::CommitReader;
use cortex_git::{GitError, GixHistory, HistorySource, OpenError};

/// The size and modification time of each configuration file of the repository, as far as the
/// file system says, each when it could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigStamp(Vec<Option<(u64, SystemTime)>>);

impl ConfigStamp {
    /// The stamp of the repository whose working tree's top is `root`, now: none of its files
    /// when its layout cannot be read.
    fn of(root: &Path) -> Self {
        let Some(layout) = git_layout(root) else {
            return Self(Vec::new());
        };
        let files = [
            layout.common_dir.join("config"),
            layout.git_dir.join("config.worktree"),
        ];
        Self(
            files
                .iter()
                .map(|file| {
                    std::fs::metadata(file)
                        .and_then(|metadata| Ok((metadata.len(), metadata.modified()?)))
                        .ok()
                })
                .collect(),
        )
    }
}

/// Why git is not run for a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unavailable {
    /// The repository is refused for the daemon's life, for this reason.
    Refused(String),
    /// HEAD's branch has no commit yet.
    NoCommit,
    /// Opening or asking git failed this time, for this reason.
    Failed(String),
}

/// The program's handles, open at one HEAD and one configuration.
struct Program {
    /// The configuration's stamp when they were opened.
    stamp: ConfigStamp,
    /// HEAD's commit when they were opened.
    head: String,
    /// What blames spans.
    blamer: Blamer,
    /// What reads commits.
    reader: CommitReader,
}

/// The repository's handles, opened when first needed and again when they may be stale.
pub(super) struct Repository {
    /// The repository's top.
    root: PathBuf,
    /// The program's handles, once opened.
    program: Option<Program>,
    /// Why the repository is refused, once it is.
    refused: Option<String>,
}

/// What one job reads the repository with, at HEAD's commit: the history opened for the job, and
/// the program's handles.
pub(super) struct Git<'a> {
    /// The repository's top.
    pub(super) root: &'a Path,
    /// HEAD's commit.
    pub(super) head: &'a str,
    /// The history, read in-process, closed when the job ends.
    pub(super) history: GixHistory,
    /// What blames spans.
    pub(super) blamer: &'a Blamer,
    /// What reads commits.
    pub(super) reader: &'a CommitReader,
}

impl Repository {
    /// The repository at `root`, nothing opened yet.
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            program: None,
            refused: None,
        }
    }

    /// What a job reads the repository with at HEAD's commit: the history opened afresh, and the
    /// program's handles, opened again first when they may be stale (see the module's
    /// documentation).
    ///
    /// # Errors
    ///
    /// The repository is refused, now or before; HEAD has no commit; or git failed this time.
    pub(super) fn at_head(&mut self) -> Result<Git<'_>, Unavailable> {
        if let Some(why) = &self.refused {
            return Err(Unavailable::Refused(why.clone()));
        }
        let history = match GixHistory::open(&self.root) {
            Ok(history) => history,
            Err(error) => return Err(self.history_error(&error)),
        };
        let head = history
            .head()
            .map_err(|error| Unavailable::Failed(error.to_string()))?
            .ok_or(Unavailable::NoCommit)?;
        let stamp = ConfigStamp::of(&self.root);
        let is_stale = self
            .program
            .as_ref()
            .is_none_or(|program| program.head != head || program.stamp != stamp);
        if is_stale {
            self.program = None;
            let blamer = Blamer::open(&self.root).map_err(|error| self.open_error(&error))?;
            let reader = CommitReader::open(&self.root).map_err(|error| self.open_error(&error))?;
            self.program = Some(Program {
                stamp,
                head,
                blamer,
                reader,
            });
        }
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| Unavailable::Failed("git was not opened".to_owned()))?;
        Ok(Git {
            root: &self.root,
            head: &program.head,
            history,
            blamer: &program.blamer,
            reader: &program.reader,
        })
    }

    /// `error`, met opening the history, as why git is not run: a folder that holds no
    /// repository is refused for good.
    fn history_error(&mut self, error: &GitError) -> Unavailable {
        match error {
            GitError::NoRepository(_) => self.refuse(error.to_string()),
            _ => Unavailable::Failed(error.to_string()),
        }
    }

    /// `error`, met opening git's handles, as why git is not run: every reason but git failing to
    /// answer is a refusal, kept for good.
    fn open_error(&mut self, error: &OpenError) -> Unavailable {
        match error {
            OpenError::Git(_) => Unavailable::Failed(error.to_string()),
            _ => self.refuse(error.to_string()),
        }
    }

    /// Refuses the repository for good, for `why`.
    fn refuse(&mut self, why: String) -> Unavailable {
        self.refused = Some(why.clone());
        Unavailable::Refused(why)
    }
}
