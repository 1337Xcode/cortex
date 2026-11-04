//! The work tree's changes read with the `git` program, on repositories the tests make: what each
//! kind of change leaves in the set, worked out from what git's own status says of it.

mod common;

use common::{Repo, TestResult};
use cortex_git::worktree::uncommitted_files;

/// The set read from `repo`, as plain paths.
fn uncommitted(repo: &Repo) -> TestResult<Vec<String>> {
    Ok(uncommitted_files(&repo.root())?
        .into_iter()
        .map(|path| path.as_str().to_owned())
        .collect())
}

/// A work tree with nothing changed since HEAD holds no uncommitted file; nor does one with no
/// commit and no file.
#[test]
fn a_clean_work_tree_holds_nothing() -> TestResult {
    let repo = Repo::new()?;
    assert!(uncommitted(&repo)?.is_empty());
    repo.write("a.py", "a = 1\n")?;
    repo.commit(1)?;
    assert!(uncommitted(&repo)?.is_empty());
    Ok(())
}

/// Every kind of change, made with the files and with git's own commands: a file changed and not
/// staged, one staged, a new one staged, one untracked in a folder of its own, one whose name has
/// a space, the new name of a file moved with `git mv`, all held; a file deleted from the work
/// tree, one deleted with `git rm`, the old name of the moved file and a file the repository's
/// `.gitignore` names, none of them.
#[test]
fn each_kind_of_change_is_held_or_not() -> TestResult {
    let repo = Repo::new()?;
    for name in [
        "changed.py",
        "staged.py",
        "deleted.py",
        "removed.py",
        "old.py",
    ] {
        repo.write(name, &format!("{name} = 1\n"))?;
    }
    repo.write(".gitignore", "ignored.log\n")?;
    repo.commit(1)?;
    repo.write("changed.py", "changed = 2\n")?;
    repo.write("staged.py", "staged = 2\n")?;
    repo.git(&["add", "staged.py"])?;
    repo.write("added.py", "added = 1\n")?;
    repo.git(&["add", "added.py"])?;
    repo.write("new/untracked.py", "untracked = 1\n")?;
    repo.write("with space.py", "space = 1\n")?;
    repo.remove("deleted.py")?;
    repo.git(&["rm", "-q", "removed.py"])?;
    repo.git(&["mv", "old.py", "moved.py"])?;
    repo.write("ignored.log", "noise\n")?;
    assert_eq!(
        uncommitted(&repo)?,
        [
            "added.py",
            "changed.py",
            "moved.py",
            "new/untracked.py",
            "staged.py",
            "with space.py"
        ]
    );
    Ok(())
}
