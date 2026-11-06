//! The work tree's changes read with the `git` program, on repositories the tests make: what each
//! kind of change leaves in the set, worked out from what git's own status says of it.

mod common;

use common::{Repo, TestResult, git_version};
use cortex_git::worktree::{WorktreeError, uncommitted_files};

/// The set read from `repo`, as plain paths.
fn uncommitted(repo: &Repo) -> TestResult<Vec<String>> {
    Ok(uncommitted_files(&repo.root())?
        .into_iter()
        .map(|path| path.as_str().to_owned())
        .collect())
}

/// A folder that is not the top of a git repository has no diff to read, which is said as such
/// rather than as git failing.
#[test]
fn a_folder_without_a_repository_has_no_diff() -> TestResult {
    let folder = tempfile::tempdir()?;
    assert!(matches!(
        uncommitted_files(folder.path()),
        Err(WorktreeError::NoRepository)
    ));
    Ok(())
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
/// tree, one deleted with `git rm`, the old name of the moved file, a file the repository's
/// `.gitignore` names and one in Cortex's own `.cortex` folder, which nothing ignores here, none
/// of them.
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
    repo.write(".cortex/config.toml", "mode = \"medium\"\n")?;
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

/// The filter every case below names: its command leaves a mark in the folder it runs in.
const MARKING: &str = "touch filtered; cat";

/// A repository whose own configuration names a filter, which `.gitattributes` gives every file, is
/// not looked at, wherever the configuration names it: in the repository's own file, in a file it
/// includes, in one it includes while a branch is checked out, and in the work tree's own file.
/// The changed file keeps its size and has a time unlike the index's, so git reads it, through the
/// filter, to tell whether it changed: git's own status runs the filter's command, which leaves
/// its mark, and Cortex's look, refused before git runs, leaves none.
#[test]
fn a_filter_the_repository_names_is_never_run() -> TestResult {
    let included = format!("[filter \"marking\"]\n\tclean = {MARKING}\n");
    let cases: [(&str, &[&[&str]]); 4] = [
        (
            "its own file",
            &[&["config", "filter.marking.clean", MARKING]],
        ),
        (
            "an included file",
            &[&["config", "include.path", "extra.cfg"]],
        ),
        (
            "a file included on its branch",
            &[&["config", "includeIf.onbranch:main.path", "extra.cfg"]],
        ),
        (
            "the work tree's own file",
            &[
                &["config", "extensions.worktreeConfig", "true"],
                &["config", "--worktree", "filter.marking.clean", MARKING],
            ],
        ),
    ];
    for (place, setup) in cases {
        let repo = Repo::new()?;
        repo.write("a.txt", "a\n")?;
        repo.commit(1)?;
        std::fs::write(repo.root().join(".git").join("extra.cfg"), &included)?;
        for args in setup {
            repo.git(args)?;
        }
        repo.write(".gitattributes", "* filter=marking\n")?;
        repo.write("a.txt", "b\n")?;
        repo.backdate("a.txt")?;
        let mark = repo.root().join("filtered");
        let looked = uncommitted_files(&repo.root());
        assert!(!mark.exists(), "{place}");
        assert!(matches!(looked, Err(WorktreeError::Refused)), "{place}");
        repo.git(&["status", "--porcelain"])?;
        assert!(mark.exists(), "git's own status runs the filter in {place}");
    }
    Ok(())
}

/// A submodule is not looked into, so a filter its own configuration names does not run: git's own
/// status looks into the submodule, reads its changed file through the filter and leaves the
/// filter's mark there, and Cortex's look leaves none.
#[test]
fn a_filter_a_submodule_names_is_never_run() -> TestResult {
    let inner = Repo::new()?;
    inner.write("s.txt", "s\n")?;
    inner.commit(1)?;
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    let inner_root = inner.root();
    let source = inner_root
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    repo.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        source,
        "sub",
    ])?;
    repo.commit(2)?;
    repo.git(&["-C", "sub", "config", "filter.marking.clean", MARKING])?;
    repo.write("sub/.gitattributes", "* filter=marking\n")?;
    repo.write("sub/s.txt", "t\n")?;
    repo.backdate("sub/s.txt")?;
    let mark = repo.root().join("sub").join("filtered");
    assert!(uncommitted(&repo)?.is_empty());
    assert!(!mark.exists());
    repo.git(&["status", "--porcelain"])?;
    assert!(mark.exists(), "git's own status looks into the submodule");
    Ok(())
}

/// A partial clone is looked at only by a git that can be told not to fetch what the clone left
/// out, 2.45 or later, as `git version` says; with an older git the look is refused before git
/// runs. The clone's origin is gone, so a look that tried to fetch would fail.
#[test]
fn a_partial_clone_is_looked_at_only_by_a_git_that_fetches_nothing() -> TestResult {
    let origin = Repo::new()?;
    origin.git(&["config", "uploadpack.allowFilter", "true"])?;
    origin.write("a.txt", "a\n")?;
    origin.commit(1)?;
    let clone = Repo::clone_of(&origin, &["--filter=blob:none"])?;
    drop(origin);
    clone.write("b.txt", "b\n")?;
    if git_version()? >= (2, 45) {
        assert_eq!(uncommitted(&clone)?, ["b.txt"]);
    } else {
        let looked = uncommitted_files(&clone.root());
        assert!(
            matches!(looked, Err(WorktreeError::WouldFetch)),
            "{looked:?}"
        );
    }
    Ok(())
}

/// A file system monitor the repository's own configuration names does not run: git's own status
/// runs it, which leaves its mark, and Cortex's look leaves none and reads the change.
#[test]
fn a_monitor_the_repository_names_is_never_run() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    repo.git(&["config", "core.fsmonitor", "touch monitored; true"])?;
    repo.write("a.txt", "a changed\n")?;
    let mark = repo.root().join("monitored");
    assert_eq!(uncommitted(&repo)?, ["a.txt"]);
    assert!(!mark.exists());
    repo.git(&["status", "--porcelain"])?;
    assert!(mark.exists(), "git's own status runs the monitor");
    Ok(())
}

/// A look writes nothing. A file whose contents are as committed but whose time is unlike the one
/// the index holds is read to tell whether it changed, and git's own status then writes the index
/// anew with the file's new time; the look reads no change and leaves the index as it was, byte
/// for byte, where git's own status does not.
#[test]
fn a_look_leaves_the_index_as_it_was() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    repo.backdate("a.txt")?;
    let index = repo.root().join(".git").join("index");
    let before = std::fs::read(&index)?;
    assert!(uncommitted(&repo)?.is_empty());
    assert_eq!(std::fs::read(&index)?, before);
    repo.git(&["status", "--porcelain"])?;
    assert_ne!(std::fs::read(&index)?, before, "git's own status writes it");
    Ok(())
}

/// A `.gitignore` whose only change since HEAD is the block `cortex init` adds at its end is
/// Cortex's own work and is not held, whether HEAD held the file, with or without a line break at
/// its end, or init made it; once the user changes it too, as when a line goes before the block or
/// after it, it is held, and so is a block line the user commits and then removes.
#[test]
fn init_s_own_ignore_lines_are_no_change() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.py", "a = 1\n")?;
    repo.write(".gitignore", "target/")?;
    repo.commit(1)?;
    repo.write(
        ".gitignore",
        "target/\n# Cortex\n.cortex/\n.claude/settings.local.json\n",
    )?;
    assert!(uncommitted(&repo)?.is_empty());
    repo.write(".gitignore", "target/\n*.log\n# Cortex\n.cortex/\n")?;
    assert_eq!(uncommitted(&repo)?, [".gitignore"]);
    repo.write(".gitignore", "target/\n# Cortex\n.cortex/\n*.log\n")?;
    assert_eq!(uncommitted(&repo)?, [".gitignore"]);

    let made = Repo::new()?;
    made.write("a.py", "a = 1\n")?;
    made.commit(1)?;
    made.write(".gitignore", "# Cortex\n.cortex/\n")?;
    assert!(uncommitted(&made)?.is_empty());

    let committed = Repo::new()?;
    committed.write(".gitignore", "target/\n# Cortex\n.cortex/\n")?;
    committed.commit(1)?;
    committed.write(".gitignore", "# Cortex\n.cortex/\n")?;
    assert_eq!(uncommitted(&committed)?, [".gitignore"]);
    Ok(())
}

/// A `.gitignore` that is a link is not read, since it may lead out of the repository: it stays
/// held, though the file it leads to holds init's block alone. Unix only: making a link on Windows
/// needs a privilege a test cannot count on.
#[cfg(unix)]
#[test]
fn a_linked_gitignore_is_never_read() -> TestResult {
    let outside = tempfile::tempdir()?;
    let target = outside.path().join("ignores");
    std::fs::write(&target, "# Cortex\n.cortex/\n")?;
    let repo = Repo::new()?;
    repo.write("a.py", "a = 1\n")?;
    repo.commit(1)?;
    std::os::unix::fs::symlink(&target, repo.root().join(".gitignore"))?;
    assert_eq!(uncommitted(&repo)?, [".gitignore"]);
    Ok(())
}
