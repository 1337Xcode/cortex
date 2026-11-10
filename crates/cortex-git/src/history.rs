//! A repository's history as the co-change miner reads it: newest first along the first-parent
//! line from HEAD, within a window, each commit with the files it changed against its first
//! parent. A source reads it from git; [`HistorySource`] is what every source gives.

use std::path::{Path, PathBuf};

use cortex_core::RepoPath;
use thiserror::Error;

/// The name of the git folder at the top of a work tree, or of the file naming it elsewhere in a
/// linked work tree.
const DOT_GIT: &str = ".git";

/// Whether the folder `root` holds a repository at its top: it has `.git`. A source opened on a
/// folder without one is [`GitError::NoRepository`].
#[must_use]
pub fn has_repository(root: &Path) -> bool {
    root.join(DOT_GIT).exists()
}

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
    /// The commit to read from, in hexadecimal: the HEAD a mining resolved, so that a commit made
    /// after it is left to the next mining rather than counted now and again then. HEAD as it is
    /// when the read starts when none is given.
    pub from: Option<String>,
}

/// One file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PathChange {
    /// The file's path after the commit; for a deletion, the path it had.
    pub path: RepoPath,
    /// The path it had before, when the commit renamed it.
    pub renamed_from: Option<RepoPath>,
}

impl PathChange {
    /// The change git reports of the file at `path`, renamed from `renamed_from` when it was. A
    /// path that cannot be a repository path (not UTF-8, absolute, holding a backslash, or with an
    /// empty, `.` or `..` name), which no file Cortex indexes has and only a tree written by hand
    /// holds, is left out: a rename from such a path is a change of the new path alone, a rename
    /// to one a change of the old path alone, and a change of one nothing.
    pub(crate) fn from_git(path: &[u8], renamed_from: Option<&[u8]>) -> Option<Self> {
        match (repo_path(path), renamed_from.map(repo_path)) {
            (Some(path), renamed_from) => Some(Self {
                path,
                renamed_from: renamed_from.flatten(),
            }),
            (None, Some(Some(path))) => Some(Self {
                path,
                renamed_from: None,
            }),
            (None, _) => None,
        }
    }
}

/// A path as git stores it, as a repository path; `None` when it cannot be a file's.
fn repo_path(path: &[u8]) -> Option<RepoPath> {
    let text = std::str::from_utf8(path).ok()?;
    RepoPath::new(text).ok().filter(|path| !path.is_root())
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
    /// A file as HEAD's commit holds it could not be read.
    #[error("reading a committed file: {0}")]
    Committed(String),
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
