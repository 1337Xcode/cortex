//! A watched repository's changes, as the watcher reports them, waited for with a time limit.

mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use common::{TestResult, write, write_all};
use cortex_core::RepoPath;
use cortex_index::{Change, DiscoverOptions, Watcher, watch};

/// How long a test waits for the changes it expects.
const PATIENCE: Duration = Duration::from_secs(20);

/// Watches the repository at `root` with the default options, its changes arriving on the
/// returned receiver.
fn watched(root: &Path) -> TestResult<(Watcher, Receiver<Change>)> {
    let options = DiscoverOptions {
        max_file_bytes: 1_048_576,
        extra_ignores: &[],
        global_gitignore: None,
    };
    let (sender, receiver) = mpsc::channel();
    let watcher = watch(root, &options, move |change| {
        // The test may have stopped listening; then nothing is left to tell.
        let _ = sender.send(change);
    })?;
    Ok((watcher, receiver))
}

/// The changes `receiver` brings until `is_done` holds for them all, or the time runs out.
fn changes_until(
    receiver: &Receiver<Change>,
    is_done: impl Fn(&[Change]) -> bool,
) -> TestResult<Vec<Change>> {
    let deadline = Instant::now() + PATIENCE;
    let mut changes = Vec::new();
    while !is_done(&changes) {
        let left = deadline.saturating_duration_since(Instant::now());
        changes.push(receiver.recv_timeout(left)?);
    }
    Ok(changes)
}

/// Every path the `changes` name.
fn paths(changes: &[Change]) -> BTreeSet<String> {
    changes
        .iter()
        .flat_map(|change| match change {
            Change::Paths(paths) => paths.iter().map(RepoPath::to_string).collect(),
            Change::Everything => Vec::new(),
        })
        .collect()
}

/// A changed file, a new one, a removed one and both names of a renamed one are reported.
#[test]
fn changed_removed_and_renamed_files_are_reported() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("src/a.py", "a = 1\n"),
            ("src/b.py", "b = 1\n"),
            ("src/c.py", "c = 1\n"),
        ],
    )?;
    let (_watcher, receiver) = watched(root)?;
    write(root, "src/a.py", b"a = 2\n")?;
    write(root, "src/new.py", b"n = 1\n")?;
    std::fs::remove_file(root.join("src/b.py"))?;
    std::fs::rename(root.join("src/c.py"), root.join("src/d.py"))?;
    let wanted: BTreeSet<String> = ["src/a.py", "src/new.py", "src/b.py", "src/c.py", "src/d.py"]
        .map(str::to_owned)
        .into();
    let changes = changes_until(&receiver, |changes| paths(changes).is_superset(&wanted))?;
    assert!(paths(&changes).is_superset(&wanted), "{changes:?}");
    Ok(())
}

/// Paths the walk leaves out are not reported: an ignored folder's files, and anything under
/// `.git` or `.cortex`. A file written after them is, and every change before it has arrived.
#[test]
fn paths_the_walk_leaves_out_are_not_reported() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            (".gitignore", "build/\n"),
            ("build/keep.txt", "k\n"),
            ("a.py", "a = 1\n"),
        ],
    )?;
    std::fs::create_dir_all(root.join(".git"))?;
    std::fs::create_dir_all(root.join(".cortex"))?;
    let (_watcher, receiver) = watched(root)?;
    write(root, "build/out.txt", b"built\n")?;
    write(root, ".git/HEAD", b"ref: refs/heads/main\n")?;
    write(root, ".cortex/x", b"x\n")?;
    write(root, "a.py", b"a = 2\n")?;
    let changes = changes_until(&receiver, |changes| paths(changes).contains("a.py"))?;
    assert_eq!(paths(&changes), ["a.py".to_owned()].into(), "{changes:?}");
    Ok(())
}

/// A change to a file of rules is a change to everything: which files count may differ.
#[test]
fn a_rules_change_is_a_change_to_everything() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(root, &[(".gitignore", "build/\n"), ("a.py", "a = 1\n")])?;
    let (_watcher, receiver) = watched(root)?;
    write(root, ".gitignore", b"build/\ndist/\n")?;
    changes_until(&receiver, |changes| changes.contains(&Change::Everything))?;
    Ok(())
}

/// More than 500 paths changed within 2 seconds, as a checkout makes, is a change to
/// everything.
#[test]
fn many_paths_at_once_are_a_change_to_everything() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("many"))?;
    let (_watcher, receiver) = watched(root)?;
    for number in 0..600 {
        write(root, &format!("many/f{number:03}.txt"), b"x\n")?;
    }
    changes_until(&receiver, |changes| changes.contains(&Change::Everything))?;
    Ok(())
}

/// A change under a link to a folder outside the repository is not reported, as the walk does not
/// follow links either; a file written after it is. Links need privileges to create on Windows,
/// so this runs on Unix only.
#[cfg(unix)]
#[test]
fn changes_behind_a_link_are_not_reported() -> TestResult {
    let repo = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let root = repo.path();
    write_all(root, &[("a.py", "a = 1\n")])?;
    write(outside.path(), "x.py", b"x = 1\n")?;
    std::os::unix::fs::symlink(outside.path(), root.join("link"))?;
    let (_watcher, receiver) = watched(root)?;
    write(outside.path(), "x.py", b"x = 2\n")?;
    write(root, "a.py", b"a = 2\n")?;
    let changes = changes_until(&receiver, |changes| paths(changes).contains("a.py"))?;
    assert_eq!(paths(&changes), ["a.py".to_owned()].into(), "{changes:?}");
    Ok(())
}
