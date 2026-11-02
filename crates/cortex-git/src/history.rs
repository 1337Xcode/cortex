//! A repository's history as the co-change miner reads it: newest first along the first-parent
//! line from HEAD, within a window, each commit with the files it changed against its first
//! parent. A source reads it from git; [`HistorySource`] is what every source gives.

use std::path::PathBuf;

use cortex_core::RepoPath;
use thiserror::Error;

/// What part of the history to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The most commits read.
    pub max_commits: usize,
    /// Commits made before this time, in seconds since the Unix epoch, are not read, nor is
    /// anything older than the first of them on the line.
    pub oldest_s: i64,
    /// A commit that changed more files than this is read but its files are not: it is a mass
    /// change (a refactor across the tree, formatting) that says nothing about which files go
    /// together.
    pub max_files: usize,
    /// The commit at which to stop, not read itself: the newest one read before, when reading only
    /// what is new.
    pub stop_at: Option<String>,
}

/// One file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PathChange {
    /// The file's path after the commit; for a deletion, the path it had.
    pub path: RepoPath,
    /// The path it had before, when the commit renamed it.
    pub renamed_from: Option<RepoPath>,
}

/// One commit read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedCommit {
    /// The commit's id, in lowercase hexadecimal.
    pub id: String,
    /// When it was committed, in seconds since the Unix epoch.
    pub time_s: i64,
    /// The files it changed against its first parent (against nothing for a commit without
    /// parents), in path order; `None` when it changed more than [`Window::max_files`].
    pub changes: Option<Vec<PathChange>>,
}

/// What a read gave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mined {
    /// The commits read, newest first.
    pub commits: Vec<MinedCommit>,
    /// Whether the read stopped at [`Window::stop_at`]. When it did not, that commit is not on the
    /// first-parent line within the window (the history was rewritten, or it is too old), and
    /// what was read before no longer leads up to HEAD.
    pub is_stop_reached: bool,
    /// Whether the history is cut short by a shallow clone: its oldest commits have no parents in
    /// the repository, and are left out rather than counted as adding every file.
    pub is_shallow: bool,
    /// Whether the repository is a partial clone, which holds file contents only as it needs them:
    /// a renamed file is then recognised only when its contents did not change, since comparing
    /// contents would need contents the clone does not hold.
    pub is_partial: bool,
}

/// Why a history could not be read. Each names what failed and the commit when there is one, never
/// a file's contents.
#[derive(Debug, Error)]
pub enum GitError {
    /// The folder is not the top of a git repository's work tree: there is no history to read.
    #[error("{} is not the top of a git repository", .0.display())]
    NoRepository(PathBuf),
    /// The folder holds a git repository that cannot be opened, or whose work tree is elsewhere.
    #[error("opening the git repository: {0}")]
    Open(String),
    /// HEAD cannot be read.
    #[error("reading HEAD: {0}")]
    Head(String),
    /// Walking the history failed.
    #[error("walking the history: {0}")]
    Walk(String),
    /// A commit's changes could not be read.
    #[error("reading the changes of commit {commit}: {message}")]
    Changes {
        /// The commit, in hexadecimal.
        commit: String,
        /// What failed.
        message: String,
    },
}

/// A source of a repository's history.
pub trait HistorySource {
    /// The commit HEAD points to, in lowercase hexadecimal; `None` in a repository with no commit
    /// yet.
    ///
    /// # Errors
    ///
    /// HEAD cannot be read.
    fn head(&self) -> Result<Option<String>, GitError>;

    /// The commits `window` covers, newest first, from HEAD.
    ///
    /// # Errors
    ///
    /// The history or a commit's changes cannot be read.
    fn read(&self, window: &Window) -> Result<Mined, GitError>;
}
