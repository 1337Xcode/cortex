//! `cortex daemon` on temporary repositories, with an empty home folder so that no user
//! configuration applies: it indexes and says so, indexes a file again after it changes, refuses a
//! second daemon on the same repository, and in huge mode parses what it took inventory of.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Daemon, TestResult, daemon_command, isolate, wait_for};
use cortex_core::{Identifier, RepoId};
use cortex_store::{MetaKey, Reader};

/// Runs git with `args` in `folder`, with `home` as the user's home folder.
fn git(folder: &Path, home: &Path, args: &[&str]) -> TestResult {
    let mut command = Command::new("git");
    command.args(args).current_dir(folder);
    isolate(&mut command, home);
    let status = command.status()?;
    if !status.success() {
        return Err(format!("git {args:?} failed: {status}").into());
    }
    Ok(())
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
