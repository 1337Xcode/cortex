//! Where a command of a line runs, as far as the line tells: the working folder the agent gave,
//! moved by each `cd` or `pushd` the line runs in the same shell, and the paths that commands
//! name, resolved from it without touching the disk. A `cd` the line does not say the target of
//! (`cd` alone, `cd -`, a `~` or a variable) leaves the folder unknown, and no path is resolved
//! from an unknown folder.

use std::path::{Component, Path, PathBuf};

use cortex_core::RepoPath;

use super::argv::after_assignments;
use super::shell::{Command, List, Word};
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

    /// The folder's absolute path, `None` when it is unknown.
    pub(super) fn path(&self) -> Option<&Path> {
        self.0.as_deref()
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

/// Walks the commands of `list` in order, moving `folder` by each `cd` that moves the shell of
/// the line, until `target`; whether it was found with no group before it.
pub(super) fn walk(list: &List, target: &[Word], folder: &mut Folder) -> bool {
    for pipeline in list.items.iter().flat_map(|item| &item.pipelines) {
        let is_alone = pipeline.commands.len() == 1;
        for command in &pipeline.commands {
            let Command::Simple(words) = command else {
                return false;
            };
            if std::ptr::eq(words.as_slice(), target) {
                return true;
            }
            if let Some(moved) = folder.after(words).filter(|_| is_alone) {
                *folder = moved;
            }
        }
    }
    false
}

/// `path` without `.` and `..`, each `..` taking off the name before it; `None` when a `..` would
/// climb above the root.
pub(super) fn normalized(path: &Path) -> Option<PathBuf> {
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
