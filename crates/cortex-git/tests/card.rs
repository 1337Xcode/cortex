//! Change cards read from what the `git` program prints, through [`CommitReader`], on repositories
//! the tests make: a commit with a body, a merge commit with two parents, a repository set up to
//! verify signatures with a program of its own, a commit that only moves whitespace, and a
//! message too long to show.

mod common;

use common::{DAY_0_S, DAY_S, Repo, TestResult, git_version, make_executable};
use cortex_git::OpenError;
use cortex_git::card::{
    AuthorKind, CardContext, ChangeKind, ChangedPaths, CommitError, CommitReader, read_card,
};

/// The context for a commit that changed one path, by no bot.
fn one_path() -> CardContext<'static> {
    CardContext {
        changed_paths: ChangedPaths::Count(1),
        max_files: 30,
        is_cosmetic: false,
        bot_patterns: &[],
    }
}

/// A reader for `repo`.
fn open(repo: &Repo) -> TestResult<CommitReader> {
    Ok(CommitReader::open(&repo.root())?)
}

/// A commit with a subject that names its pull request, a body of a reason, an issue and a
/// trailer is read from git's own output: the id, no parents, the committer time of day 3, the
/// subject, the reason that comes after the blank line, the issue and the pull request.
#[test]
fn a_commit_is_read_from_what_git_shows() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.git(&["add", "-A"])?;
    let message = "Fix the crash on empty input (#7)\n\nParsing no items raised an error.\n\n\
        Closes #12\nSigned-off-by: Ann <ann@example.com>";
    repo.git_on(3, &["commit", "-q", "-m", message])?;
    let oid = repo.git(&["rev-parse", "HEAD"])?;

    let shown = open(&repo)?.record(&oid)?;
    let card = read_card(&shown, &one_path())?;
    assert_eq!(card.oid, oid);
    assert_eq!(card.parents, Vec::<String>::new());
    assert_eq!(card.time_s, DAY_0_S + 3 * DAY_S);
    assert_eq!(card.subject, "Fix the crash on empty input (#7)");
    assert_eq!(
        card.reason.as_deref(),
        Some("Parsing no items raised an error.")
    );
    assert_eq!(card.pull_request, Some(7));
    assert_eq!(card.issues, ["#12"]);
    assert_eq!(card.author, AuthorKind::Human);
    assert_eq!(card.kind, ChangeKind::Normal);
    Ok(())
}

/// The card's time is the committer time, which a rebase or an amend moves and an author time
/// would not: the commit is authored on day 2 and committed on day 6.
#[test]
fn the_time_is_the_committer_time() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.git(&["add", "-A"])?;
    // `--date` sets the author time; the committer time is the day the environment gives.
    let author_time = format!("{} +0000", DAY_0_S + 2 * DAY_S);
    repo.git_on(6, &["commit", "-q", "--date", &author_time, "-m", "Add a"])?;
    let oid = repo.git(&["rev-parse", "HEAD"])?;
    let card = read_card(&open(&repo)?.record(&oid)?, &one_path())?;
    assert_eq!(card.time_s, DAY_0_S + 6 * DAY_S);
    Ok(())
}

/// A merge commit lists both parents, in git's order, and its subject names the pull request.
#[test]
fn a_merge_commit_lists_both_parents() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    repo.git(&["checkout", "-q", "-b", "side"])?;
    repo.write("b.txt", "b\n")?;
    let side = repo.commit(2)?;
    repo.git(&["checkout", "-q", "main"])?;
    repo.write("c.txt", "c\n")?;
    let main = repo.commit(3)?;
    repo.git_on(
        4,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge pull request #5 from owner/side",
            "side",
        ],
    )?;
    let merge = repo.git(&["rev-parse", "HEAD"])?;

    let card = read_card(&open(&repo)?.record(&merge)?, &one_path())?;
    assert_eq!(card.parents, [main, side]);
    assert_eq!(card.pull_request, Some(5));
    assert_eq!(card.reason, None);
    Ok(())
}

/// A repository that asks git to verify signatures with a program of its own does not run that
/// program for the record: a commit carrying a signature header is shown by git's own `show`,
/// which runs the program and leaves its mark, and by the reader, which does not. The signature
/// is not a real one; git runs the program all the same, and the commit is made with
/// `hash-object` for that reason.
#[test]
fn a_signature_check_the_repository_asks_for_is_not_run() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"])?;
    let signed = format!(
        "tree {tree}\nauthor Cortex Test <test> {DAY_0_S} +0000\ncommitter Cortex Test <test> {DAY_0_S} +0000\n\
        gpgsig -----BEGIN PGP SIGNATURE-----\n \n AAAA\n -----END PGP SIGNATURE-----\n\nA signed commit\n"
    );
    let oid = repo.git_fed(
        1,
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        signed.as_bytes(),
    )?;

    let marker = repo.home().join("verified");
    let program = repo.home().join("gpg.sh");
    let marker_path = marker
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\necho ran > '{}'\nexit 0\n",
            marker_path.replace('\\', "/")
        ),
    )?;
    make_executable(&program)?;
    let program_path = program
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    repo.git(&["config", "log.showSignature", "true"])?;
    repo.git(&["config", "gpg.program", &program_path.replace('\\', "/")])?;

    repo.git(&["show", "-s", &oid])?;
    assert!(marker.exists(), "git's own show did not run the program");
    std::fs::remove_file(&marker)?;

    let shown = open(&repo)?.record(&oid)?;
    assert!(!marker.exists(), "the reader ran the signature program");
    let card = read_card(&shown, &one_path())?;
    assert_eq!(card.oid, oid);
    assert_eq!(card.subject, "A signed commit");
    Ok(())
}

/// A commit that only reindents is cosmetic, one that changes a line, adds a file, or is the first
/// commit is not, and a merge is read against its first parent.
#[test]
fn only_a_commit_that_moves_whitespace_is_cosmetic() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.py", "def f():\n    return 1\n")?;
    let first = repo.commit(1)?;
    repo.write("a.py", "def f():\n  return 1\n")?;
    let reindent = repo.commit(2)?;
    repo.write("a.py", "def f():\n  return 2\n")?;
    let change = repo.commit(3)?;
    repo.write("b.py", "x = 1\n")?;
    let added = repo.commit(4)?;
    repo.git(&["checkout", "-q", "-b", "side"])?;
    repo.write("c.py", "y = 1\n")?;
    repo.commit(5)?;
    repo.git(&["checkout", "-q", "main"])?;
    repo.git_on(6, &["merge", "-q", "--no-ff", "-m", "Merge side", "side"])?;
    let merge = repo.git(&["rev-parse", "HEAD"])?;

    let reader = open(&repo)?;
    assert!(reader.is_cosmetic(&reindent)?);
    for (name, oid) in [
        ("the first commit", &first),
        ("a changed line", &change),
        ("an added file", &added),
        ("a merge that brings a file", &merge),
    ] {
        assert!(!reader.is_cosmetic(oid)?, "{name}");
    }
    Ok(())
}

/// A text conversion or an external diff the repository names does not run for the check: git's
/// own `show -p` runs the program, which leaves its mark and is the premise here, and the check
/// leaves none.
#[test]
fn the_whitespace_check_runs_no_program_the_repository_names() -> TestResult {
    let repo = Repo::new()?;
    repo.write(".gitattributes", "*.txt diff=probe\n")?;
    repo.write("a.txt", "one\n")?;
    repo.commit(1)?;
    repo.write("a.txt", "two\n")?;
    let changed = repo.commit(2)?;

    let marker = repo.home().join("ran");
    let probe = repo.home().join("probe.sh");
    let marker_path = marker
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    std::fs::write(
        &probe,
        format!(
            "#!/bin/sh\necho ran > '{}'\ncat \"$1\"\n",
            marker_path.replace('\\', "/")
        ),
    )?;
    make_executable(&probe)?;
    let probe_path = probe
        .to_str()
        .ok_or("a temporary folder that is not UTF-8")?;
    let config = repo.root().join(".git").join("config");
    let mut text = std::fs::read_to_string(&config)?;
    text.push_str(&format!(
        "[diff \"probe\"]\n\ttextconv = '{}'\n\tcommand = '{}'\n",
        probe_path.replace('\\', "/"),
        probe_path.replace('\\', "/")
    ));
    std::fs::write(&config, text)?;

    repo.git(&["show", "-p", &changed])?;
    assert!(marker.exists(), "git's own show did not run the program");
    std::fs::remove_file(&marker)?;

    assert!(!open(&repo)?.is_cosmetic(&changed)?);
    assert!(
        !marker.exists(),
        "the check ran a program the repository names"
    );
    Ok(())
}

/// What cannot be read is an error that says why: an id that is not a full object id never reaches
/// git, an object that is not there is git's failure, and a message over 256 KiB is refused, not
/// cut.
#[test]
fn a_commit_that_cannot_be_read_is_an_error() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    let oid = repo.commit(1)?;
    let reader = open(&repo)?;
    for bad in ["HEAD", "--help", &oid[..10], ""] {
        assert_eq!(reader.record(bad), Err(CommitError::BadRequest), "{bad:?}");
        assert_eq!(
            reader.is_cosmetic(bad),
            Err(CommitError::BadRequest),
            "{bad:?}"
        );
    }
    let missing = "0".repeat(40);
    assert!(matches!(reader.record(&missing), Err(CommitError::Git(_))));
    assert!(matches!(
        reader.is_cosmetic(&missing),
        Err(CommitError::Git(_))
    ));

    repo.write("a.txt", "b\n")?;
    repo.git(&["add", "-A"])?;
    let long = format!("Add b\n\n{}\n", "word ".repeat(60_000));
    repo.git_fed(2, &["commit", "-q", "-F", "-"], long.as_bytes())?;
    let long_oid = repo.git(&["rev-parse", "HEAD"])?;
    assert_eq!(reader.record(&long_oid), Err(CommitError::TooLarge));
    Ok(())
}

/// A repository whose own configuration names a filter, or a folder with no repository, cannot be
/// opened; a partial clone can be only with a git that fetches nothing.
#[test]
fn a_repository_git_may_not_be_run_in_is_refused() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "a\n")?;
    repo.commit(1)?;
    repo.git(&["config", "filter.marking.clean", "touch filtered; cat"])?;
    assert_eq!(
        CommitReader::open(&repo.root()).err(),
        Some(OpenError::Refused)
    );

    let folder = tempfile::tempdir()?;
    assert_eq!(
        CommitReader::open(folder.path()).err(),
        Some(OpenError::NoRepository)
    );

    if git_version()? < (2, 45) {
        let origin = Repo::new()?;
        origin.git(&["config", "uploadpack.allowFilter", "true"])?;
        origin.write("a.txt", "a\n")?;
        origin.commit(1)?;
        let clone = Repo::clone_of(&origin, &["--filter=blob:none"])?;
        assert_eq!(
            CommitReader::open(&clone.root()).err(),
            Some(OpenError::WouldFetch)
        );
    }
    Ok(())
}
