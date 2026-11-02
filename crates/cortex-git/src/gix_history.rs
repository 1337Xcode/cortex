//! The history read in-process with `gix`. The repository is opened isolated: of git's
//! configuration only the repository's own file is read, never the user's, the system's or the
//! environment's, and the settings that shape a read are set here. How renamed files are compared
//! is in [`crate::renames`].

use std::error::Error;
use std::path::Path;

use cortex_core::canonical_root;
use gix::diff::blob::Platform as ComparisonCache;
use gix::object::tree::diff::{Action, Change};

use crate::history::{GitError, HistorySource, Mined, MinedCommit, PathChange, Window};
use crate::renames::{MAX_COMPARED_BYTES, comparison_cache, rename_search};

/// The memory kept for git objects read recently, chiefly trees: walking newest first, a commit's
/// tree is read again as its child's parent tree. 4 MiB took reading 20,000 synthetic commits from
/// about 2.5 s to about 1.3 s; larger caches gained nothing more.
const OBJECT_CACHE_BYTES: usize = 4 * 1024 * 1024;

/// The name of the git folder at the top of a work tree.
const DOT_GIT: &str = ".git";

/// The setting older versions of git use to name, in a partial clone, the remote that holds what
/// the clone left out; newer ones set `promisor` on that remote instead.
const PARTIAL_CLONE: &str = "extensions.partialClone";

/// A repository's history read with `gix`.
pub struct GixHistory {
    /// The repository, opened isolated.
    repo: gix::Repository,
    /// Whether the repository is a partial clone.
    is_partial: bool,
}

impl GixHistory {
    /// Opens the git repository whose work tree's top is the folder `root`. A repository is not
    /// searched for above it: the folder Cortex serves is a repository's top or holds none.
    ///
    /// # Errors
    ///
    /// [`GitError::NoRepository`] when `root` has no `.git`; [`GitError::Open`] when git cannot
    /// open the repository or its work tree is not `root`.
    pub fn open(root: &Path) -> Result<Self, GitError> {
        if !root.join(DOT_GIT).exists() {
            return Err(GitError::NoRepository(root.to_path_buf()));
        }
        let options = gix::open::Options::isolated()
            .config_overrides([format!("core.bigFileThreshold={MAX_COMPARED_BYTES}")]);
        let mut repo =
            gix::open_opts(root, options).map_err(|error| GitError::Open(message(&error)))?;
        repo.object_cache_size(OBJECT_CACHE_BYTES);
        // A repository's `core.worktree` can name another folder; its paths are then relative to
        // that folder's top, not this one.
        let is_root = repo
            .workdir()
            .is_some_and(|workdir| is_same_folder(workdir, root));
        if !is_root {
            return Err(GitError::Open(format!(
                "its work tree is not {}",
                root.display()
            )));
        }
        let is_partial = is_partial_clone(&repo);
        Ok(Self { repo, is_partial })
    }

    /// The commit HEAD points to, peeled through tags; `None` while HEAD's branch has no commit.
    ///
    /// # Errors
    ///
    /// HEAD cannot be read or peeled.
    fn head_id(&self) -> Result<Option<gix::ObjectId>, GitError> {
        let head = self.repo.head().map_err(|error| head_error(&error))?;
        let id = head
            .try_into_peeled_id()
            .map_err(|error| head_error(&error))?;
        Ok(id.map(gix::Id::detach))
    }

    /// The files `commit` changed against its first parent (against nothing when it has none), in
    /// path order; `None` when they are more than `max_files`. `cache` holds what comparing files
    /// for renames needs, kept from commit to commit.
    ///
    /// # Errors
    ///
    /// The commit, its parent or their trees cannot be read, or comparing them fails.
    fn changes_of(
        &self,
        commit: &gix::Commit<'_>,
        max_files: usize,
        cache: &mut ComparisonCache,
    ) -> Result<Option<Vec<PathChange>>, GitError> {
        let failed = |error: &(dyn Error + 'static)| GitError::Changes {
            commit: commit.id.to_string(),
            message: message(error),
        };
        let tree = commit.tree().map_err(|error| failed(&error))?;
        let parent_tree = match commit.parent_ids().next() {
            Some(parent) => parent
                .object()
                .map_err(|error| failed(&error))?
                .try_into_commit()
                .map_err(|error| failed(&error))?
                .tree()
                .map_err(|error| failed(&error))?,
            None => self.repo.empty_tree(),
        };
        let mut touched = 0usize;
        let mut changes = Vec::new();
        let mut platform = parent_tree.changes().map_err(|error| failed(&error))?;
        platform.options(|options| {
            options
                .track_path()
                .track_rewrites(Some(rename_search(max_files, self.is_partial)));
        });
        let compared = platform.for_each_to_obtain_tree_with_cache(&tree, cache, |change| {
            if change.entry_mode().is_tree() {
                return Ok(Action::Continue(()));
            }
            touched += 1;
            if touched > max_files {
                return Ok(Action::Break(()));
            }
            changes.extend(path_change(&change));
            Ok(Action::Continue(()))
        });
        cache.clear_resource_cache_keep_allocation();
        // `gix` reports a comparison stopped from the callback as cancelled, an error: for a
        // commit cut for its size, that is the expected end.
        if touched > max_files {
            return Ok(None);
        }
        compared.map_err(|error| failed(&error))?;
        changes.sort();
        Ok(Some(changes))
    }
}

impl HistorySource for GixHistory {
    fn head(&self) -> Result<Option<String>, GitError> {
        Ok(self.head_id()?.map(|id| id.to_string()))
    }

    fn read(&self, window: &Window) -> Result<Mined, GitError> {
        let mut mined = Mined {
            is_partial: self.is_partial,
            ..Mined::default()
        };
        let Some(head) = self.head_id()? else {
            return Ok(mined);
        };
        let shallow = self
            .repo
            .shallow_commits()
            .map_err(|error| walk_error(&error))?;
        mined.is_shallow = shallow.is_some();
        // A first-parent walk follows the line from HEAD, newest first; `gix` takes no time
        // cut-off on such a walk, so the window's oldest time is checked below.
        let walk = self
            .repo
            .rev_walk([head])
            .first_parent_only()
            .all()
            .map_err(|error| walk_error(&error))?;
        let mut cache = comparison_cache(&self.repo).map_err(|error| walk_error(&error))?;
        // An id that is not hexadecimal names no commit, so the read never stops at it.
        let stop_at = window
            .stop_at
            .as_deref()
            .and_then(|id| gix::ObjectId::from_hex(id.as_bytes()).ok());
        for info in walk {
            let info = info.map_err(|error| walk_error(&error))?;
            let id = info.id().detach();
            if stop_at == Some(id) {
                mined.is_stop_reached = true;
                break;
            }
            if mined.commits.len() >= window.max_commits {
                break;
            }
            let commit = info.object().map_err(|error| walk_error(&error))?;
            let time_s = commit.time().map_err(|error| walk_error(&error))?.seconds;
            if time_s < window.oldest_s {
                break;
            }
            // A shallow clone's oldest commits name parents the repository does not have: compared
            // with nothing, they would count as adding every file.
            let is_cut = shallow
                .as_ref()
                .is_some_and(|cut| cut.iter().any(|boundary| *boundary == id));
            if is_cut {
                continue;
            }
            let changes = self.changes_of(&commit, window.max_files, &mut cache)?;
            mined.commits.push(MinedCommit {
                id: id.to_string(),
                time_s,
                changes,
            });
        }
        Ok(mined)
    }
}

/// The change as a path change, by [`PathChange::from_git`]'s rules.
fn path_change(change: &Change<'_, '_, '_>) -> Option<PathChange> {
    match change {
        Change::Addition { location, .. }
        | Change::Deletion { location, .. }
        | Change::Modification { location, .. } => PathChange::from_git(location, None),
        Change::Rewrite {
            source_location,
            location,
            ..
        } => PathChange::from_git(location, Some(source_location)),
    }
}

/// Whether `repo` is a partial clone, as git tells one: `extensions.partialClone` set, or
/// `promisor` set on a remote. A `promisor` value that is not a boolean counts as set: comparing
/// contents in a partial clone fails the read, while comparing none only finds fewer renames.
fn is_partial_clone(repo: &gix::Repository) -> bool {
    let config = repo.config_snapshot();
    let file = config.plumbing();
    file.string(PARTIAL_CLONE).is_some()
        || file
            .sections_by_name("remote")
            .into_iter()
            .flatten()
            .any(|remote| {
                let promisor =
                    file.boolean_by("remote", remote.header().subsection_name(), "promisor");
                matches!(promisor, Ok(Some(true)) | Err(_))
            })
}

/// Whether `a` and `b` name the same folder.
fn is_same_folder(a: &Path, b: &Path) -> bool {
    match (canonical_root(a), canonical_root(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// `error` and each of its causes, joined by colons: `gix` names the step that failed and its
/// causes say why.
fn message(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// A failure reading HEAD.
fn head_error(error: &(dyn Error + 'static)) -> GitError {
    GitError::Head(message(error))
}

/// A failure walking the history.
fn walk_error(error: &(dyn Error + 'static)) -> GitError {
    GitError::Walk(message(error))
}
