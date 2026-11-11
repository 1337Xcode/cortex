//! A repository whose own configuration includes another file is not read with git: git follows an
//! `include.path` or `includeIf.<condition>.path` wherever it points, a network share included,
//! and what the file names (a filter, a text conversion) would run. Every run of the crate is
//! refused first, so that no included file is read and no command in one runs.

mod common;

use common::{Repo, TestResult, every_commit};
use cortex_git::blame::Blamer;
use cortex_git::card::CommitReader;
use cortex_git::worktree::{WorktreeError, uncommitted_files};
use cortex_git::{CliHistory, GitError, GixHistory, HistorySource, OpenError};

/// What the included file holds: a filter and a text conversion whose commands leave a mark in the
/// folder they run in, the work tree's top. The commands are quoted, as a `;` outside quotes starts
/// a comment in a configuration file.
const INCLUDED: &str = "[filter \"marking\"]\n\tclean = \"touch filtered; cat\"\n\
                        [diff \"marking\"]\n\ttextconv = \"touch converted; cat\"\n";

/// The marks the commands of [`INCLUDED`] leave.
const MARKS: [&str; 2] = ["filtered", "converted"];

/// How a test's repository includes the file.
#[derive(Debug, Clone, Copy)]
enum Include {
    /// `include.path` in the repository's own file.
    Path,
    /// `includeIf` on the git folder's location, which holds here.
    IfGitDir,
    /// `include.path` in the work tree's own file.
    WorkTreeFile,
}

/// A repository with two commits of a file that a filter and a text conversion are given to,
/// which includes, in the way `include` says, a file outside it that names both; and the file
/// changed in the work tree, keeping its size and with a time unlike the index's, so that git
/// reads it, through the filter, to tell whether it changed.
fn including(include: Include, contents: &str) -> TestResult<Repo> {
    let repo = Repo::new()?;
    repo.write(".gitattributes", "*.txt filter=marking diff=marking\n")?;
    repo.write("a.txt", "one\ntwo\n")?;
    repo.commit(1)?;
    repo.write("a.txt", "one\nTWO\n")?;
    repo.commit(2)?;
    let outside = repo
        .root()
        .parent()
        .ok_or("a repository folder with no parent")?
        .join("outside.cfg");
    std::fs::write(&outside, contents)?;
    let path = outside
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?
        .replace('\\', "/");
    match include {
        Include::Path => {
            repo.git(&["config", "include.path", &path])?;
        }
        Include::IfGitDir => {
            repo.git(&["config", "includeIf.gitdir:**/repo/.git.path", &path])?;
        }
        Include::WorkTreeFile => {
            repo.git(&["config", "extensions.worktreeConfig", "true"])?;
            repo.git(&["config", "--worktree", "include.path", &path])?;
        }
    }
    repo.write("a.txt", "one\nTOO\n")?;
    repo.backdate("a.txt")?;
    Ok(repo)
}

/// Whether a mark in `repo`'s work tree exists, and which.
fn marks(repo: &Repo) -> Vec<&'static str> {
    MARKS
        .into_iter()
        .filter(|mark| repo.root().join(mark).exists())
        .collect()
}

/// Deletes the marks of `repo`.
fn clear_marks(repo: &Repo) -> TestResult {
    for mark in marks(repo) {
        std::fs::remove_file(repo.root().join(mark))?;
    }
    Ok(())
}

/// The premise of every test below: git's own status runs the included filter, its blame and its
/// show the included text conversion, each leaving its mark, so that a missing mark means the
/// crate did not run the command.
fn assert_git_runs_the_included_commands(repo: &Repo, place: &str) -> TestResult {
    let runs: [(&[&str], &str); 3] = [
        (&["status", "--porcelain"], "filtered"),
        (
            &["blame", "--porcelain", "-L", "1,2", "HEAD", "--", "a.txt"],
            "converted",
        ),
        (&["show", "-p", "HEAD"], "converted"),
    ];
    for (args, mark) in runs {
        repo.git(args)?;
        assert!(
            repo.root().join(mark).exists(),
            "git's own {args:?} did not run the command in {place}"
        );
        clear_marks(repo)?;
    }
    Ok(())
}

/// The repository, refused by every run of the crate: the work tree's status, the blamer, the
/// commit reader and the history read by running git.
fn assert_every_run_refused(repo: &Repo, place: &str) {
    let looked = uncommitted_files(&repo.root());
    assert!(
        matches!(looked, Err(WorktreeError::Includes)),
        "status in {place}"
    );
    assert_eq!(
        Blamer::open(&repo.root()).err(),
        Some(OpenError::Includes),
        "blame in {place}"
    );
    assert_eq!(
        CommitReader::open(&repo.root()).err(),
        Some(OpenError::Includes),
        "show in {place}"
    );
    assert!(
        matches!(
            CliHistory::open(&repo.root()).err(),
            Some(GitError::Open(message)) if message.contains("includes another file")
        ),
        "listing in {place}"
    );
    assert_eq!(marks(repo), Vec::<&str>::new(), "{place}");
}

/// A file outside the repository that names a filter and a text conversion, included by the
/// repository's own file with `include.path`, by `includeIf` on where the git folder is, or by the
/// work tree's own file, is never read, so none of the crate's runs runs a command in it: each is
/// refused, and no mark is left. Git's own status, blame and show run the commands.
#[test]
fn an_included_filter_and_text_conversion_are_never_run() -> TestResult {
    for (include, place) in [
        (Include::Path, "include.path"),
        (Include::IfGitDir, "includeIf"),
        (Include::WorkTreeFile, "the work tree's own file"),
    ] {
        let repo = including(include, INCLUDED)?;
        assert_every_run_refused(&repo, place);
        assert_git_runs_the_included_commands(&repo, place)?;
    }
    Ok(())
}

/// The refusal does not depend on what the included file holds, since it is never read: a file
/// that is missing, which git skips without a word, and one that names nothing, are refused as
/// well.
#[test]
fn an_include_is_refused_whatever_it_names() -> TestResult {
    let harmless = including(Include::Path, "[user]\n\tname = nobody\n")?;
    assert_every_run_refused(&harmless, "a harmless file");
    let missing = including(Include::Path, "")?;
    let outside = missing
        .root()
        .parent()
        .ok_or("a repository folder with no parent")?
        .join("outside.cfg");
    std::fs::remove_file(outside)?;
    assert_every_run_refused(&missing, "a missing file");
    Ok(())
}

/// A repository whose configuration includes nothing is read as before by every run, and a
/// filter its own file names is still refused as a filter, not as an include.
#[test]
fn a_repository_without_includes_is_read() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "one\n")?;
    repo.commit(1)?;
    repo.write("a.txt", "two\n")?;
    assert!(uncommitted_files(&repo.root())?.contains(&cortex_core::RepoPath::new("a.txt")?));
    assert!(Blamer::open(&repo.root()).is_ok());
    assert!(CommitReader::open(&repo.root()).is_ok());
    assert!(CliHistory::open(&repo.root()).is_ok());

    repo.git(&["config", "filter.marking.clean", "touch filtered; cat"])?;
    assert!(matches!(
        uncommitted_files(&repo.root()),
        Err(WorktreeError::Refused)
    ));
    assert_eq!(Blamer::open(&repo.root()).err(), Some(OpenError::Refused));
    Ok(())
}

/// `gix` opens a repository without following its includes: a remote an included file marks as a
/// partial clone's promisor, which git reads as one, is not one to `gix`, so nothing in an
/// included file reaches it and it needs no refusal of its own.
#[test]
fn gix_follows_no_include() -> TestResult {
    let repo = including(Include::Path, "[remote \"origin\"]\n\tpromisor = true\n")?;
    assert_eq!(
        repo.git(&["config", "--includes", "--get", "remote.origin.promisor"])?,
        "true",
        "git reads the included setting"
    );
    let mined = GixHistory::open(&repo.root())?.read(&every_commit())?;
    assert_eq!(mined.commits.len(), 2);
    assert!(!mined.is_partial, "gix read the included setting");
    Ok(())
}
