//! Where git keeps a working tree's repository, as Cortex reads it from git's files without running
//! git ([`cortex_core::git_layout`]), checked against what `git rev-parse` says of the same folder:
//! a main working tree, a linked one, a bare repository's linked working tree, a submodule, and
//! the `.git` files that are no layout (one naming nothing, and one too large to read).

mod common;

use std::path::{Path, PathBuf};

use common::{Repo, TestResult, git_in};
use cortex_core::{Identifier, canonical_root, git_layout};

/// What git says of the working tree at `folder`: its top and its common folder, canonical.
fn said_by_git(folder: &Path, home: &Path) -> TestResult<(PathBuf, PathBuf)> {
    let top = git_in(folder, home, &["rev-parse", "--show-toplevel"])?;
    let common = git_in(folder, home, &["rev-parse", "--git-common-dir"])?;
    // Git names the common folder relative to where it ran when it is below it.
    Ok((
        canonical_root(Path::new(&top))?,
        canonical_root(&folder.join(common))?,
    ))
}

/// A repository with one commit, on the branch `main`.
fn committed() -> TestResult<Repo> {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    Ok(repo)
}

/// A main working tree is its own main tree, not linked, its common folder its `.git`, as git
/// says; it names no working tree and its branch is `main`.
#[test]
fn a_main_working_tree_is_its_own_main_tree() -> TestResult {
    let repo = committed()?;
    let layout = git_layout(&repo.root()).ok_or("no layout")?;
    let (top, common) = said_by_git(&repo.root(), &repo.home())?;
    assert_eq!((&layout.worktree_root, &layout.common_dir), (&top, &common));
    assert!(!layout.is_linked());
    assert_eq!(layout.main_root.as_ref(), Some(&top));
    assert_eq!(layout.worktree_name(), None);
    assert_eq!(layout.branch(), Some(Identifier::new("main")?));
    Ok(())
}

/// A linked working tree made with `git worktree add` on a new branch is linked, its top and
/// common folder as git says, its main tree the repository's top, named by its folder and on its
/// branch; checked out detached, it names no branch.
#[test]
fn a_linked_working_tree_names_its_main_tree() -> TestResult {
    let repo = committed()?;
    let linked = repo.root().with_file_name("wt");
    let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
    repo.git(&["worktree", "add", "-q", "-b", "feature/one", spelled])?;
    let layout = git_layout(&linked).ok_or("no layout")?;
    let (top, common) = said_by_git(&linked, &repo.home())?;
    assert_eq!((&layout.worktree_root, &layout.common_dir), (&top, &common));
    assert!(layout.is_linked());
    assert_eq!(layout.main_root, Some(canonical_root(&repo.root())?));
    assert_eq!(layout.worktree_name(), Some(Identifier::new("wt")?));
    assert_eq!(layout.branch(), Some(Identifier::new("feature/one")?));

    let detached = repo.root().with_file_name("detached");
    let spelled = detached.to_str().ok_or("a path that is not UTF-8")?;
    repo.git(&["worktree", "add", "-q", "--detach", spelled])?;
    let layout = git_layout(&detached).ok_or("no layout")?;
    assert!(layout.is_linked());
    assert_eq!(layout.branch(), None);
    Ok(())
}

/// A bare repository's linked working tree is linked, its top and common folder as git says, and
/// has no main tree: no folder's `.git` is the bare repository.
#[test]
fn a_bare_repository_s_linked_working_tree_has_no_main_tree() -> TestResult {
    let origin = committed()?;
    let bare = Repo::clone_of(&origin, &["--bare"])?;
    let linked = bare.root().with_file_name("bwt");
    let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
    bare.git(&["worktree", "add", "-q", spelled])?;
    let layout = git_layout(&linked).ok_or("no layout")?;
    let (top, common) = said_by_git(&linked, &bare.home())?;
    assert_eq!((&layout.worktree_root, &layout.common_dir), (&top, &common));
    assert_eq!(common, canonical_root(&bare.root())?);
    assert!(layout.is_linked());
    assert_eq!(layout.main_root, None);
    Ok(())
}

/// A submodule's `.git` is a file too, naming its folder under the outer repository's
/// `.git/modules`: it is a repository of its own, not linked, its common folder that folder, as
/// git says, and its own main tree.
#[test]
fn a_submodule_is_a_repository_of_its_own() -> TestResult {
    let inner = committed()?;
    let outer = committed()?;
    let source = inner.root();
    let source = source.to_str().ok_or("a path that is not UTF-8")?;
    outer.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        source,
        "sub",
    ])?;
    let sub = outer.root().join("sub");
    assert!(sub.join(".git").is_file(), "the submodule's .git is a file");
    let layout = git_layout(&sub).ok_or("no layout")?;
    let (top, common) = said_by_git(&sub, &outer.home())?;
    assert_eq!((&layout.worktree_root, &layout.common_dir), (&top, &common));
    assert!(!layout.is_linked());
    assert_eq!(layout.main_root, Some(top));
    Ok(())
}

/// A `.git` file naming a folder that is not there is no layout, and git finds no repository
/// there either; a linked working tree's `.git` file padded past 4 KiB is no layout, though the
/// path it names is right: it is not read at all.
#[test]
fn a_git_file_naming_nothing_or_too_large_is_no_layout() -> TestResult {
    let repo = committed()?;
    let linked = repo.root().with_file_name("wt");
    let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
    repo.git(&["worktree", "add", "-q", "--detach", spelled])?;
    let dot_git = linked.join(".git");
    let named = std::fs::read_to_string(&dot_git)?;

    std::fs::write(&dot_git, "gitdir: ../nowhere\n")?;
    assert_eq!(git_layout(&linked), None);
    assert!(git_in(&linked, &repo.home(), &["rev-parse", "--show-toplevel"]).is_err());

    std::fs::write(&dot_git, format!("{named}{}", " ".repeat(4 * 1024)))?;
    assert_eq!(git_layout(&linked), None);
    std::fs::write(&dot_git, named)?;
    assert!(git_layout(&linked).is_some_and(|layout| layout.is_linked()));
    Ok(())
}
