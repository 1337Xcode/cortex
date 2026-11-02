//! How a renamed file is recognised: a deletion and an addition in one commit whose contents are
//! at least half the same, compared as git stores them. Attributes take no part, as they take
//! none in git's own renames, so the comparison reads nothing but the repository's objects: no
//! attributes file, index or work tree. No filter or text conversion runs on contents taken from
//! the object store, so nothing the repository names is run.

use gix::attrs::Search;
use gix::attrs::search::MetadataCollection;
use gix::diff::Rewrites;
use gix::diff::blob::Platform as ComparisonCache;
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::glob::pattern::Case;
use gix::worktree::stack::State;
use gix::worktree::stack::state::Attributes;
use gix::worktree::stack::state::attributes::Source;

/// How similar two files' contents must be for a deletion and an addition to be one rename:
/// half, git's own default.
const RENAME_SIMILARITY: f32 = 0.5;

/// Files larger than this are taken as renamed only when their contents did not change. Comparing
/// two files costs time in proportion to their size, at most `max_files` squared times a commit;
/// the indexer reads no file larger than this by default, so a larger one rarely matters to what
/// Cortex learns.
pub(crate) const MAX_COMPARED_BYTES: u64 = 1024 * 1024;

/// The rename search for a commit read with at most `max_files` changed files, by content unless
/// the repository `is_partial`, then by identical contents only.
pub(crate) fn rename_search(max_files: usize, is_partial: bool) -> Rewrites {
    Rewrites {
        copies: None,
        percentage: (!is_partial).then_some(RENAME_SIMILARITY),
        // `gix` compares every deletion with every addition unless their product passes the limit
        // squared. With the limit at `max_files`, a commit past it has more than `max_files`
        // deletions or more than `max_files` additions, each of which counts as a changed file
        // whether paired or not, so it is cut as a mass commit anyway; any other commit compares
        // at most `max_files` squared pairs. A limit of 0 would mean no limit.
        limit: max_files.max(1),
        track_empty: false,
    }
}

/// A cache for comparing files' contents with no attributes and no work tree.
///
/// # Errors
///
/// The repository's configuration names a comparison setting `gix` cannot read.
pub(crate) fn comparison_cache(repo: &gix::Repository) -> Result<ComparisonCache, gix::Error> {
    let attributes = Attributes::new(
        Search::default(),
        None,
        Source::IdMapping,
        MetadataCollection::default(),
    );
    let stack = gix::worktree::Stack::new(
        repo.git_dir(),
        State::AttributesStack(attributes),
        Case::Sensitive,
        Vec::new(),
        Vec::new(),
    );
    gix::diff::resource_cache(repo, Mode::ToGit, stack, WorktreeRoots::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limit of pairs compared follows the commit size cut, and a partial clone matches
    /// identical contents only.
    #[test]
    fn the_search_follows_the_cut() {
        let search = rename_search(30, false);
        assert_eq!(search.limit, 30);
        assert_eq!(search.percentage, Some(0.5));
        assert_eq!(rename_search(30, true).percentage, None);
        assert_eq!(rename_search(0, false).limit, 1);
    }
}
