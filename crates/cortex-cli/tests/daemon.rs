//! `cortex daemon` on temporary repositories, with an empty home folder so that no user
//! configuration applies: it indexes and says so, indexes a file again after it changes, refuses a
//! second daemon on the same repository, in huge mode parses what it took inventory of, and mines
//! the history of a git repository.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::git::{self, git};
use common::{Daemon, TestResult, daemon_command, isolate, wait_for};
use cortex_core::{EdgeType, Identifier, RepoId, RepoPath};
use cortex_git::mine::Mark;
use cortex_store::{FileId, MetaKey, NodeId, Reader};

/// Writes `paths` in the repository at `root`, each with text that names `commit`, and commits them
/// as `commit` with `home` as the user's home folder; returns the new HEAD.
fn commit(root: &Path, home: &Path, commit: &str, paths: &[&str]) -> TestResult<String> {
    for path in paths {
        std::fs::write(root.join(path), format!("# {path} in {commit}\n"))?;
    }
    git::commit(root, home, commit, paths)
}

/// The mark the last mining of the repository at `root` left, if any.
fn mark(root: &Path) -> TestResult<Option<Mark>> {
    Ok(Mark::read(
        Reader::open(&root.join(".cortex"), 0)?.queries(),
    )?)
}

/// What `cortex status --root <root>` printed, with `home` as the user's home folder.
fn status_of(root: &Path, home: &Path) -> TestResult<String> {
    let mut status = Command::new(env!("CARGO_BIN_EXE_cortex"));
    status.args(["status", "--root"]).arg(root);
    isolate(&mut status, home);
    Ok(String::from_utf8(status.output()?.stdout)?)
}

/// A path git reads from its configuration: with slashes, since git takes a backslash there as an
/// escape.
fn for_git(path: &Path) -> TestResult<String> {
    Ok(path
        .to_str()
        .ok_or("a path that is not UTF-8")?
        .replace('\\', "/"))
}

/// The ready line of a daemon on a repository holding `app.py` and `build_out/out.py`, after
/// `setup` has prepared the repository and the home folder, in that order.
fn ready_line_with(setup: impl FnOnce(&Path, &Path) -> TestResult) -> TestResult<String> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def kept():\n    pass\n")?;
    std::fs::create_dir(repo.path().join("build_out"))?;
    std::fs::write(
        repo.path().join("build_out/out.py"),
        "def ignored():\n    pass\n",
    )?;
    setup(repo.path(), home.path())?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    daemon.first_line()
}

/// The ready line when only `app.py` is indexed: `build_out/` is ignored.
const ONE_WRITTEN: &str = "cortex: daemon ready: mode tiny; files: 1 written, 0 unchanged";

/// The ready line when `build_out/out.py` is indexed too.
const TWO_WRITTEN: &str = "cortex: daemon ready: mode tiny; files: 2 written, 0 unchanged";

/// Whether the store of the repository at `root` holds a definition named `name`.
fn has_definition(root: &Path, name: &str) -> TestResult<bool> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(!reader
        .queries()
        .symbols_named(&Identifier::new(name)?)?
        .is_empty())
}

/// The daemon indexes the one file, says it is ready, and indexes the file again once it changes;
/// meanwhile a second daemon on the repository is refused, naming the process that holds it.
#[test]
fn indexes_watches_and_holds_the_writer() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let app = repo.path().join("app.py");
    std::fs::write(&app, "def first():\n    pass\n")?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        daemon.first_line()?,
        "cortex: daemon ready: mode tiny; files: 1 written, 0 unchanged"
    );
    assert!(has_definition(repo.path(), "first")?);
    std::fs::write(&app, "def first():\n    pass\n\ndef second():\n    pass\n")?;
    wait_for(|| has_definition(repo.path(), "second"))?;
    let refused = daemon_command(repo.path(), home.path()).output()?;
    let stderr = String::from_utf8(refused.stderr)?;
    assert_eq!(refused.status.code(), Some(1), "{stderr}");
    assert_eq!(
        stderr.trim_end(),
        format!(
            "cortex: error: another daemon serves this repository (process {})",
            daemon.child.id()
        )
    );
    Ok(())
}

/// A root spelled through a subfolder and `..`, and on Windows in upper case, is one repository
/// with its plain spelling: the store is written in the root's `.cortex/` and carries the id of
/// the root as the operating system resolves it.
#[test]
fn every_spelling_of_the_root_names_one_repository() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let repo = scratch.path().join("repo");
    std::fs::create_dir_all(repo.join("sub"))?;
    std::fs::write(repo.join("app.py"), "def kept():\n    pass\n")?;
    let mut spelling = repo.join("sub").join("..");
    if cfg!(windows) {
        let text = spelling.to_str().ok_or("a path that is not UTF-8")?;
        spelling = PathBuf::from(text.to_uppercase());
    }
    let daemon = Daemon::start(&spelling, home.path())?;
    assert_eq!(daemon.first_line()?, ONE_WRITTEN);
    drop(daemon);
    let reader = Reader::open(&repo.join(".cortex"), 0)?;
    let stored = reader.queries().meta(MetaKey::RepoId)?;
    let resolved = RepoId::of(&std::fs::canonicalize(&repo)?);
    assert_eq!(stored.as_deref(), Some(resolved.as_str()));
    Ok(())
}

/// The user's global gitignore applies, found as git finds it: `core.excludesFile` in the home
/// folder's `.gitconfig` names a file that ignores `build_out/`.
#[test]
fn the_global_gitignore_applies() -> TestResult {
    let line = ready_line_with(|_, home| {
        let ignore = home.join("global-ignore");
        std::fs::write(&ignore, "build_out/\n")?;
        let config = format!("[core]\n\texcludesFile = {}\n", for_git(&ignore)?);
        std::fs::write(home.join(".gitconfig"), config)?;
        Ok(())
    })?;
    assert_eq!(line, ONE_WRITTEN);
    Ok(())
}

/// With `core.excludesFile` unset, git's default file applies: `.config/git/ignore` in the home
/// folder.
#[test]
fn git_s_default_global_gitignore_applies() -> TestResult {
    let line = ready_line_with(|_, home| {
        std::fs::create_dir_all(home.join(".config/git"))?;
        std::fs::write(home.join(".config/git/ignore"), "build_out/\n")?;
        Ok(())
    })?;
    assert_eq!(line, ONE_WRITTEN);
    Ok(())
}

/// `core.excludesFile` set to nothing turns the global gitignore off, as in git: the default file
/// is not read either.
#[test]
fn an_empty_excludes_file_reads_no_global_gitignore() -> TestResult {
    let line = ready_line_with(|_, home| {
        std::fs::create_dir_all(home.join(".config/git"))?;
        std::fs::write(home.join(".config/git/ignore"), "build_out/\n")?;
        std::fs::write(home.join(".gitconfig"), "[core]\n\texcludesFile =\n")?;
        Ok(())
    })?;
    assert_eq!(line, TWO_WRITTEN);
    Ok(())
}

/// A repository's own configuration cannot name the global gitignore, which the daemon would open:
/// `core.excludesFile` in its `.git/config` is not read.
#[test]
fn a_repository_cannot_name_the_global_gitignore() -> TestResult {
    let line = ready_line_with(|repo, home| {
        let ignore = repo.join("ignore-build");
        std::fs::write(&ignore, "build_out/\n")?;
        git(repo, home, &["init", "-q"])?;
        git(
            repo,
            home,
            &["config", "core.excludesFile", &for_git(&ignore)?],
        )?;
        Ok(())
    })?;
    assert_eq!(line, TWO_WRITTEN.replace("2 written", "3 written"));
    Ok(())
}

/// A git the repository holds is never run to find the global gitignore, even where the `PATH`
/// leads to it: a relative entry and the repository's own `bin` folder come before the folder of
/// the real git, and the repository's `git` would leave a mark. Unix only: a relative `PATH` entry
/// leads to a program in the folder a process runs in only on Unix.
#[cfg(unix)]
#[test]
fn a_git_the_repository_holds_is_never_run() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def kept():\n    pass\n")?;
    let bin = repo.path().join("bin");
    std::fs::create_dir(&bin)?;
    let marker = home.path().join("ran");
    let script = format!("#!/bin/sh\necho ran > '{}'\nexit 1\n", marker.display());
    std::fs::write(bin.join("git"), script)?;
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o755))?;
    let real_git = Command::new("sh").args(["-c", "command -v git"]).output()?;
    let real_git = String::from_utf8(real_git.stdout)?;
    let real_folder = Path::new(real_git.trim())
        .parent()
        .ok_or("git has no folder")?;
    let path = std::env::join_paths([Path::new("bin"), &bin, real_folder])?;
    let mut command = daemon_command(repo.path(), home.path());
    command.env("PATH", path);
    let daemon = Daemon::spawn(command)?;
    let ready = daemon.first_line()?;
    assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
    assert!(!marker.exists(), "the repository's git ran");
    Ok(())
}

/// With the mode pinned to huge in the repository's configuration, the first index takes
/// inventory of both files and parses neither; the crawler parses them after the ready line.
#[test]
fn huge_mode_crawls_what_it_took_inventory_of() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::fs::write(repo.path().join(".cortex/config.toml"), "mode = \"huge\"\n")?;
    std::fs::write(repo.path().join("a.py"), "def alpha():\n    pass\n")?;
    std::fs::write(repo.path().join("b.py"), "def beta():\n    pass\n")?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        daemon.first_line()?,
        "cortex: daemon ready: mode huge; files: 2 written, 0 unchanged, 2 waiting to be parsed"
    );
    wait_for(|| Ok(has_definition(repo.path(), "alpha")? && has_definition(repo.path(), "beta")?))?;
    Ok(())
}

/// Writes 3,000 Python files into `root`, each defining one function named `prefix` and its
/// number.
fn write_modules(root: &Path, prefix: &str) -> TestResult {
    for number in 0..3000 {
        let source = format!(
            "def {prefix}{number}():
    pass
"
        );
        std::fs::write(root.join(format!("m{number}.py")), source)?;
    }
    Ok(())
}

/// An index killed partway, after a complete one, leaves the index marked unfinished, and the next
/// daemon writes every file again; were the earlier index's mark kept, the files the killed index
/// wrote would be kept as they are. The first daemon completes an index of 3,000 files and stops.
/// All of them change; the second daemon is killed once its index has written its first batch, a
/// dozen batches before the end.
#[test]
fn an_index_killed_partway_is_redone_whole() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    write_modules(repo.path(), "f")?;
    let mut complete = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        complete.first_line()?,
        "cortex: daemon ready: mode medium; files: 3000 written, 0 unchanged"
    );
    complete.child.kill()?;
    complete.child.wait()?;
    write_modules(repo.path(), "g")?;
    let mut killed = Daemon::start(repo.path(), home.path())?;
    wait_for(|| has_definition(repo.path(), "g0"))?;
    killed.child.kill()?;
    killed.child.wait()?;
    let status = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["status", "--root"])
        .arg(repo.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()?;
    let stdout = String::from_utf8(status.stdout)?;
    assert!(
        stdout.contains(
            "
index: not ready, the last index did not finish; the next one reads every file again
"
        ),
        "{stdout}"
    );
    let redone = Daemon::start(repo.path(), home.path())?;
    assert_eq!(
        redone.first_line()?,
        "cortex: daemon ready: mode medium; files: 3000 written, 0 unchanged"
    );
    Ok(())
}

/// The daemon mines the history once its first index is done, and then the commits HEAD gains.
/// Two commits change `a.py` and `b.py` together: each file has 2 commits and the pair 2
/// co-changes, at the same weights, so each file's edge to the other has a confidence of exactly
/// 1 and a count of 2. A third commit changes `a.py` and `c.py`: `a.py` has 3 commits, the new
/// pair 1 co-change, under the 2 an edge needs, and the mark moves to the new HEAD at sequence 3.
/// Status says git is ready and how far the history is mined, and that the graph holds the root
/// folder, the three files and the two co-change links.
#[test]
fn the_daemon_mines_the_history_and_the_commits_head_gains() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    git(root, home.path(), &["init", "-q"])?;
    commit(root, home.path(), "one", &["a.py", "b.py"])?;
    let second = commit(root, home.path(), "two", &["a.py", "b.py"])?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(mark(root)?.is_some_and(|mark| mark.last_commit == second)))?;
    // Built at the first index without co-change links, the graph gains them only when the
    // mining's write ends: the root folder, `a.py` and `b.py`, the folder's hold on each file both
    // ways, and the two co-change links.
    wait_for(|| {
        Ok(status_of(root, home.path())?
            .contains("\ngraph: 3 nodes, 6 links, with co-change links; built "))
    })?;
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let file = |path: &str| -> TestResult<(FileId, NodeId)> {
        let id = queries
            .file(&RepoPath::new(path)?)?
            .ok_or_else(|| format!("{path} is not stored"))?
            .id;
        let node = queries.file_node(id)?.ok_or("a file without its node")?;
        Ok((id, node))
    };
    let (a, a_node) = file("a.py")?;
    let (b, b_node) = file("b.py")?;
    let edges = |node: NodeId| -> TestResult<Vec<(NodeId, f64, u32)>> {
        Ok(queries
            .edges_from(node)?
            .into_iter()
            .filter(|row| row.edge_type == EdgeType::CoChange)
            .map(|row| (row.edge.dst, row.edge.weight, row.edge.count))
            .collect())
    };
    assert_eq!(edges(a_node)?, [(b_node, 1.0, 2)]);
    assert_eq!(edges(b_node)?, [(a_node, 1.0, 2)]);
    let counts: Vec<(FileId, u32)> = queries
        .file_commits_of(&[a, b])?
        .into_iter()
        .map(|row| (row.file, row.count))
        .collect();
    assert_eq!(counts, [(a, 2), (b, 2)]);

    let third = commit(root, home.path(), "three", &["a.py", "c.py"])?;
    wait_for(|| Ok(mark(root)?.is_some_and(|mark| mark.last_commit == third)))?;
    assert_eq!(mark(root)?.map(|mark| mark.last_seq), Some(3));
    let (c, c_node) = file("c.py")?;
    let pairs: Vec<(FileId, FileId, u32)> = queries
        .cochange_pairs_of(&[c])?
        .into_iter()
        .map(|pair| (pair.file_a, pair.file_b, pair.count))
        .collect();
    assert_eq!(pairs, [(a, c, 1)]);
    assert_eq!(edges(c_node)?, []);
    let counts: Vec<(FileId, u32)> = queries
        .file_commits_of(&[a])?
        .into_iter()
        .map(|row| (row.file, row.count))
        .collect();
    assert_eq!(counts, [(a, 3)]);

    // The graph is rebuilt once the mining's write ends: the root folder and the three files, no
    // symbol among them, so four nodes; the folder's hold on each file, both ways, six links, and
    // the two co-change links between `a.py` and `b.py`, one each way.
    wait_for(|| {
        Ok(status_of(root, home.path())?
            .contains("\ngraph: 4 nodes, 8 links, with co-change links; built "))
    })?;
    let status = status_of(root, home.path())?;
    assert!(status.contains("\ngit: ready\n"), "{status}");
    let head_start = third.get(..12).ok_or("a short commit id")?;
    assert!(
        status.contains(&format!(
            "\nco-change: 3 commits mined, up to {head_start}; "
        )),
        "{status}"
    );
    Ok(())
}

/// A repository whose root is not the top of a git repository has no history to mine, and the
/// daemon's status says so.
#[test]
fn a_folder_without_git_is_said_to_have_no_history() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("a.py"), "def a():\n    pass\n")?;
    let daemon = Daemon::start(repo.path(), home.path())?;
    daemon.first_line()?;
    let expected = "\ngit: no repository at the root, so no history is mined\n";
    wait_for(|| Ok(status_of(repo.path(), home.path())?.contains(expected)))?;
    let status = status_of(repo.path(), home.path())?;
    assert!(
        status.contains("\nco-change: nothing mined yet\n"),
        "{status}"
    );
    Ok(())
}

/// A repository `gix` cannot read, here one whose objects are named by SHA-256 (this build's `gix`
/// knows SHA-1 only), is mined with the `git` program, and status says so.
#[test]
fn a_repository_gix_cannot_read_is_mined_with_git() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    git(root, home.path(), &["init", "-q", "--object-format=sha256"])?;
    commit(root, home.path(), "one", &["a.py", "b.py"])?;
    let second = commit(root, home.path(), "two", &["a.py", "b.py"])?;
    assert_eq!(second.len(), 64);
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(mark(root)?.is_some_and(|mark| mark.last_commit == second)))?;
    let status = status_of(root, home.path())?;
    assert!(
        status.contains("\ngit: ready: read with the git program, as gix could not: "),
        "{status}"
    );
    Ok(())
}

/// The graph follows the index: a repository of one Python file holding only a comment has a graph
/// of two nodes, the root folder and the file, and two links, the folder's hold on the file each
/// way; once a second such file is written, the watcher's change is indexed and the graph rebuilt
/// with three nodes and four links. There is no git repository, so there is no co-change link.
#[test]
fn the_graph_follows_the_index() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::write(root.join("a.py"), "# a\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| {
        Ok(status_of(root, home.path())?
            .contains("\ngraph: 2 nodes, 2 links, without co-change links; built "))
    })?;
    std::fs::write(root.join("b.py"), "# b\n")?;
    wait_for(|| {
        Ok(status_of(root, home.path())?
            .contains("\ngraph: 3 nodes, 4 links, without co-change links; built "))
    })?;
    Ok(())
}
