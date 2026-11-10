//! The repository's linked working trees (`git worktree add`) that agents work in: which events
//! truly come from one, and what each holds uncommitted.
//!
//! An event names its linked working tree by its top. The daemon takes it only when git's files
//! say that folder is a linked working tree of this repository, whose main working tree is the
//! daemon's root ([`cortex_core::git_layout`]): any other is refused, and nothing of it is read or
//! recorded. Each linked working tree an agent works in gets a look of its own at its uncommitted
//! diff, run and paced as the main tree's (see [`crate::worktree`]), and asked for again by every
//! event from it, since the watcher watches the main tree alone. At most [`MAX_LINKED`] are looked
//! at; one past them has no diff known, as a working tree has before its first look, and its
//! prompts are seeded with none.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use cortex_core::git_layout;
use cortex_core::ipc::LinkedWorktreeReport;
use cortex_store::NodeId;

use crate::inbox::Inbox;
use crate::paced::Paced;
use crate::worktree::{self, Worktree};

/// The most linked working trees looked at, each with a thread of its own and a `git status` every
/// few seconds while an agent works there: more than anyone runs agents in at once on one machine.
pub(crate) const MAX_LINKED: usize = 16;

/// The linked working trees agents worked in, and what their looks read. One made by default
/// knows no main working tree, and takes no event of a linked one.
#[derive(Debug, Default)]
pub(crate) struct Linked {
    /// The repository's main working tree: the daemon's root.
    root: PathBuf,
    /// The store a look finds file nodes in, and the memory map it opens with, once the first
    /// index is done: no look runs before.
    store: OnceLock<(PathBuf, u64)>,
    /// Where a look that fails is kept among the latest errors.
    inbox: Arc<Inbox>,
    /// Each linked working tree looked at, by its top.
    trees: Mutex<BTreeMap<PathBuf, Tree>>,
}

/// One linked working tree's diff and the thread that looks at it.
#[derive(Debug)]
struct Tree {
    /// Its diff as last read.
    worktree: Arc<Worktree>,
    /// The thread looking at it, stopped when dropped.
    _looker: Paced,
}

impl Linked {
    /// The linked working trees of the repository whose main working tree is `root`, canonical,
    /// none looked at yet; looks that fail are kept in `inbox`.
    pub(crate) fn new(root: &Path, inbox: Arc<Inbox>) -> Self {
        Self {
            root: root.to_path_buf(),
            store: OnceLock::new(),
            inbox,
            trees: Mutex::new(BTreeMap::new()),
        }
    }

    /// Lets looks run from now on, finding file nodes in the store in `folder`, opened with
    /// `mmap_bytes` of memory map.
    pub(crate) fn read_store(&self, folder: &Path, mmap_bytes: u64) {
        drop(self.store.set((folder.to_path_buf(), mmap_bytes)));
    }

    /// Whether a request naming the working tree `top` may be answered: one of the main working
    /// tree (none named), or of a folder git's files say is a linked working tree of this
    /// repository. A working tree looked at already was found so before its first look, and is not
    /// read again. It reads files: the server calls it on the blocking pool.
    pub(crate) fn is_ours(&self, top: Option<&Path>) -> bool {
        let Some(top) = top else {
            return true;
        };
        if self.trees().contains_key(top) {
            return true;
        }
        // INVARIANT: true-at-render. Only a linked working tree of this repository lends the
        // files an answer is read from and checked against.
        git_layout(top).is_some_and(|layout| {
            layout.is_linked()
                && layout.worktree_root == top
                && layout.main_root.as_deref() == Some(self.root.as_path())
        })
    }

    /// The file nodes of the diff of the linked working tree at `top`, as last read, a look at it
    /// asked for, and started when it is the first; none before the first index, before its first
    /// look ended, or past [`MAX_LINKED`] working trees.
    pub(crate) fn uncommitted(&self, top: &Path) -> Option<Arc<Vec<NodeId>>> {
        let (folder, mmap_bytes) = self.store.get()?;
        let mut trees = self.trees();
        if let Some(tree) = trees.get(top) {
            tree.worktree.asks().ask();
            return tree.worktree.uncommitted();
        }
        if trees.len() >= MAX_LINKED {
            return None;
        }
        let worktree = Arc::new(Worktree::default());
        // The thread's first look is asked for as it starts.
        let looker = worktree::watch(&worktree, top, (folder, *mmap_bytes), &self.inbox);
        trees.insert(
            top.to_path_buf(),
            Tree {
                worktree,
                _looker: looker,
            },
        );
        None
    }

    /// What each linked working tree's looks read, by its top.
    pub(crate) fn reports(&self) -> Vec<LinkedWorktreeReport> {
        self.trees()
            .iter()
            .map(|(top, tree)| LinkedWorktreeReport {
                root: top.clone(),
                worktree: tree.worktree.report(),
            })
            .collect()
    }

    /// Stops every look, waiting for those under way, and forgets the working trees.
    pub(crate) fn stop(&self) {
        // Taken out first, so that no one waits on the lock while the threads finish.
        let trees = std::mem::take(&mut *self.trees());
        drop(trees);
    }

    /// The working trees looked at, locked. A panic while they were held leaves the map whole,
    /// so a poisoned lock is taken as it is.
    fn trees(&self) -> MutexGuard<'_, BTreeMap<PathBuf, Tree>> {
        self.trees.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request of the main working tree is taken; one naming a folder that is not a linked
    /// working tree of this repository is not, nor one naming a linked working tree of another.
    #[test]
    fn only_this_repository_s_linked_working_trees_are_taken()
    -> Result<(), Box<dyn std::error::Error>> {
        let base = tempfile::tempdir()?;
        let base = cortex_core::canonical_root(base.path())?;
        let (main, linked) = written(&base, "main", "wt")?;
        let (other, elsewhere) = written(&base, "other", "wt2")?;
        let ours = Linked::new(&main, Arc::default());
        assert!(ours.is_ours(None));
        assert!(ours.is_ours(Some(&linked)));
        assert!(!ours.is_ours(Some(&elsewhere)));
        assert!(!ours.is_ours(Some(&main)));
        assert!(!ours.is_ours(Some(&other)));
        Ok(())
    }

    /// A linked working tree `name` of a main tree `main` under `base`, written as git writes one;
    /// returns both tops, canonical.
    fn written(
        base: &Path,
        main: &str,
        name: &str,
    ) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let main = base.join(main);
        let state = main.join(".git").join("worktrees").join(name);
        std::fs::create_dir_all(&state)?;
        let linked = base.join(name);
        std::fs::create_dir_all(&linked)?;
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", state.display()),
        )?;
        std::fs::write(state.join("commondir"), "../..\n")?;
        std::fs::write(
            state.join("gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )?;
        Ok((
            cortex_core::canonical_root(&main)?,
            cortex_core::canonical_root(&linked)?,
        ))
    }
}
