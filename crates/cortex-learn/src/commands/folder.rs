//! Where a command of a line runs, as far as the line tells: the working folder the agent gave,
//! moved by each `cd` or `pushd` the line runs in the same shell, and the paths that commands
//! name, resolved from it without touching the disk. A `cd` the line does not say the target of
//! (`cd` alone, `cd -`, a `~` or a variable) leaves the folder unknown, and no path is resolved
//! from an unknown folder.

use std::path::{Component, Path, PathBuf};

use cortex_core::RepoPath;

use super::argv::after_assignments;
use super::shell::Word;
use super::tail;

/// A command's working folder: an absolute path without `.` or `..`, or unknown.
#[derive(Debug, Clone)]
pub(super) struct Folder(Option<PathBuf>);

/// What a path a command names is, within the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Named {
    /// The repository's top folder: everything in it.
    Everything,
    /// A file or folder inside it.
    Path(RepoPath),
}

impl Folder {
    /// The folder `cwd`, unknown when it is not absolute.
    pub(super) fn new(cwd: &Path) -> Self {
        Self(cwd.is_absolute().then(|| normalized(cwd)).flatten())
    }

    /// The folder after the simple command `words`, when it changes the folder: `cd` or `pushd`
    /// to the folder it names, unknown when it names none (`cd` alone goes home), and `popd` to one
    /// the line does not say.
    pub(super) fn after(&self, words: &[Word]) -> Option<Self> {
        let words = after_assignments(words);
        let (command, args) = (words.first()?.literal()?, tail(words, 1));
        match command {
            "cd" | "pushd" => {
                let target = args
                    .iter()
                    .find(|word| !word.text.starts_with('-') || word.text == "-");
                Some(Self(target.and_then(|target| self.path_of(target))))
            }
            "popd" => Some(Self(None)),
            _ => None,
        }
    }

    /// The absolute path `word` names from this folder, without `.` or `..`; `None` when the word
    /// is not literal, names no path the line tells (`-`), or the folder is unknown and the path
    /// relative.
    pub(super) fn path_of(&self, word: &Word) -> Option<PathBuf> {
        let text = word.literal()?;
        if text == "-" || text.is_empty() {
            return None;
        }
        let path = Path::new(text);
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.0.as_ref()?.join(path)
        };
        normalized(&joined)
    }

    /// What `word` names in the repository whose top is `root`, from this folder: `None` when it
    /// cannot be resolved, lies outside the repository, or is a pattern (git's pathspec globs and
    /// magic, but `:/`, the top, and what follows it).
    pub(super) fn named(&self, word: &Word, root: &Path) -> Option<Named> {
        let text = word.literal()?;
        if text.contains(['*', '?', '[']) {
            return None;
        }
        let path = match text.strip_prefix(":/") {
            Some(from_top) => normalized(&root.join(from_top))?,
            None if text.starts_with(':') => return None,
            None => self.path_of(word)?,
        };
        let path = RepoPath::from_native(root, &path).ok()?;
        Some(if path.is_root() {
            Named::Everything
        } else {
            Named::Path(path)
        })
    }
}

/// `path` without `.` and `..`, each `..` taking off the name before it; `None` when a `..` would
/// climb above the root.
fn normalized(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                clean.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(clean.components().next_back(), Some(Component::Normal(_))) {
                    return None;
                }
                clean.pop();
            }
        }
    }
    Some(clean)
}
