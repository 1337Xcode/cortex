//! Blame on repositories the tests make: which commit owns the lines of a span once a formatting
//! commit, a bot's bump and a mass rename have come after the commit that wrote them. Every
//! expectation is worked out by hand from the history each test builds.

mod common;

use common::{DAY_0_S, DAY_S, Repo, TestResult, git_version, lines, make_executable};
use cortex_core::RepoPath;
use cortex_git::OpenError;
use cortex_git::blame::{BlameError, BlameRequest, BlameRun, Blamer};

/// The parser as its first commit wrote it, four spaces to a level.
const PARSER_V1: &str = "def parse(items):\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The parser after the fix: lines 2 and 3 are new.
const PARSER_V2: &str = "def parse(items):\n    if not items:\n        return []\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The parser reindented to two spaces to a level: every line changes, none by more than white
/// space.
const PARSER_V3: &str = "def parse(items):\n  if not items:\n    return []\n  out = []\n  for item in items:\n    out.append(item)\n  return out\n";

/// The commits of the history built by [`history`], newest last.
struct Commits {
    /// Day 1: the parser, a file of dependencies and thirty-five modules.
    first: String,
    /// Day 2: the fix, with its test file, that closes issue 12.
    fix: String,
    /// Day 3: the reindentation.
    reindent: String,
    /// Day 4: a bot's bump of one dependency.
    bump: String,
    /// Day 5: the parser and the modules move to new folders.
    rename: String,
}

/// Commits what the work tree holds on `day` with `message`, as `author` when one is given,
/// returning the commit's id.
fn commit_as(repo: &Repo, day: i64, author: Option<&str>, message: &str) -> TestResult<String> {
    repo.git(&["add", "-A"])?;
    let mut args = vec!["commit", "-q"];
    if let Some(author) = author {
        args.extend(["--author", author]);
    }
    args.extend(["-m", message]);
    repo.git_on(day, &args)?;
    repo.git(&["rev-parse", "HEAD"])
}

/// A repository with five commits. The parser reads 5 lines on day 1; the fix on day 2 puts 2
/// lines after its first and adds a test; the reindentation on day 3 changes the white space of
/// all 7; a bot bumps `deps.txt` on day 4; on day 5 the parser moves to `src/parsing/` and the
/// modules to `mods/`, 36 files in one commit.
fn history() -> TestResult<(Repo, Commits)> {
    let repo = Repo::new()?;
    repo.write("src/parser.py", PARSER_V1)?;
    repo.write("deps.txt", "lib==1.0\nother==2.0\n")?;
    for number in 0..35 {
        repo.write(
            &format!("pkg/m{number:02}.py"),
            &lines(&format!("mod {number}"), 3),
        )?;
    }
    let first = commit_as(&repo, 1, None, "add the parser")?;

    repo.write("src/parser.py", PARSER_V2)?;
    repo.write(
        "tests/test_parser.py",
        "def test_empty():\n    assert parse([]) == []\n",
    )?;
    let fix = commit_as(
        &repo,
        2,
        None,
        "Fix the crash on empty input\n\nParsing no items raised an error.\n\nCloses #12",
    )?;

    repo.write("src/parser.py", PARSER_V3)?;
    let reindent = commit_as(&repo, 3, None, "style: reindent the parser")?;

    repo.write("deps.txt", "lib==1.1\nother==2.0\n")?;
    let bump = commit_as(
        &repo,
        4,
        Some("dependabot[bot] <bot@example.com>"),
        "chore(deps): bump lib from 1.0 to 1.1",
    )?;

    repo.move_file("src/parser.py", "src/parsing/parser.py", None)?;
    for number in 0..35 {
        repo.move_file(
            &format!("pkg/m{number:02}.py"),
            &format!("mods/m{number:02}.py"),
            None,
        )?;
    }
    let rename = commit_as(&repo, 5, None, "move the parser and the modules")?;
    Ok((
        repo,
        Commits {
            first,
            fix,
            reindent,
            bump,
            rename,
        },
    ))
}

/// A blamer for `repo`.
fn open(repo: &Repo) -> TestResult<Blamer> {
    Ok(Blamer::open(&repo.root())?)
}

/// A request for lines `first..=last` of `path` at `at`, with no date limit and 10 seconds.
fn ask<'a>(path: &'a RepoPath, first: u32, last: u32, at: &'a str) -> BlameRequest<'a> {
    BlameRequest {
        path,
        first_line: first,
        last_line: last,
        at,
        since_s: None,
        timeout_ms: 10_000,
    }
}

/// The run of lines `start..=end` that `oid` owns, which stood from line `source_start` of the
/// commit's own file.
fn run(start: u32, end: u32, source_start: u32, oid: &str) -> BlameRun {
    BlameRun {
        start,
        end,
        oid: oid.to_owned(),
        source_start,
    }
}

/// The reindentation changed every line of the parser and the move changed its path, and the
/// blame still gives the fix the lines it wrote (2 and 3) and the first commit the rest (1 and
/// 4 to 7): `-w` sees through the reindentation and git follows the move. The fix's lines say
/// that the commit before it held them in `src/parser.py`; the first commit, a root, is a
/// boundary, which says its lines are no older than the history, not where they came from, and
/// stays one when the repository's own configuration says otherwise. Each run says where its
/// lines stood in its own commit: the fix's two at lines 2 and 3 of its file, and the first
/// commit's last four, which stood at lines 2 to 5 of its five-line file.
#[test]
fn a_reformatting_does_not_take_the_lines_from_the_commit_that_wrote_them() -> TestResult {
    let (repo, commits) = history()?;
    let path = RepoPath::new("src/parsing/parser.py")?;

    // The premise: without `-w` git blames lines 2 and 3 on the reindentation.
    let plain = repo.git(&[
        "blame",
        "--porcelain",
        "-L",
        "2,3",
        "HEAD",
        "--",
        path.as_str(),
    ])?;
    assert_eq!(
        plain.split_whitespace().next(),
        Some(commits.reindent.as_str())
    );

    // The repository asks git to treat roots as ordinary commits, which changes what is a
    // boundary; the blame pins the setting.
    repo.git(&["config", "blame.showRoot", "true"])?;
    let head = repo.git(&["rev-parse", "HEAD"])?;
    let blame = open(&repo)?.blame(&ask(&path, 1, 7, &head))?;
    assert_eq!(
        blame.runs,
        [
            run(1, 1, 1, &commits.first),
            run(2, 3, 2, &commits.fix),
            run(4, 7, 2, &commits.first)
        ]
    );
    let fixed = blame
        .commits
        .get(&commits.fix)
        .ok_or("the fix is not listed")?;
    assert!(!fixed.is_boundary);
    let before = fixed.previous.as_ref().ok_or("the fix has no previous")?;
    assert_eq!(before.oid, commits.first);
    assert_eq!(before.path, "src/parser.py");
    let first = blame
        .commits
        .get(&commits.first)
        .ok_or("the first is not listed")?;
    assert!(first.is_boundary);
    assert_eq!(first.previous, None);
    assert_eq!(blame.commits.len(), 2, "no other commit owns a line");
    Ok(())
}

/// A span of one line is a blame of one line, and a blame at an older commit reads the file as it
/// was there: at the fix, `src/parser.py` is the path and the parser is as the fix left it, so
/// line 3 is the fix's; at the head that path is gone, which git says by failing. One blamer
/// answers every one of them.
#[test]
fn the_commit_and_the_path_are_the_ones_asked_for() -> TestResult {
    let (repo, commits) = history()?;
    let blamer = open(&repo)?;
    let old_path = RepoPath::new("src/parser.py")?;
    let at_fix = blamer.blame(&ask(&old_path, 3, 3, &commits.fix))?;
    assert_eq!(at_fix.runs, [run(3, 3, 3, &commits.fix)]);

    let at_head = blamer.blame(&ask(&old_path, 3, 3, &commits.rename));
    assert!(matches!(at_head, Err(BlameError::Git(_))), "{at_head:?}");
    let at_first = blamer.blame(&ask(&old_path, 6, 8, &commits.first));
    assert!(
        matches!(at_first, Err(BlameError::Git(_))),
        "5 lines, not 8: {at_first:?}"
    );
    Ok(())
}

/// A bot's bump owns the line it changed and the first commit the other, until the repository
/// lists the bump in `.git-blame-ignore-revs`, when the line goes back to the first commit. The
/// bump's `previous` is its parent, the reindentation, which did not touch the file. A folder of
/// that name is not a list and is not used.
#[test]
fn commits_the_repository_lists_are_skipped() -> TestResult {
    let (repo, commits) = history()?;
    let blamer = open(&repo)?;
    let path = RepoPath::new("deps.txt")?;
    let request = ask(&path, 1, 2, &commits.rename);

    let bumped = blamer.blame(&request)?;
    assert_eq!(
        bumped.runs,
        [run(1, 1, 1, &commits.bump), run(2, 2, 2, &commits.first)]
    );
    let before = bumped
        .commits
        .get(&commits.bump)
        .and_then(|commit| commit.previous.as_ref());
    assert_eq!(
        before.map(|before| (before.oid.as_str(), before.path.as_str())),
        Some((commits.reindent.as_str(), "deps.txt"))
    );

    let list = repo.root().join(".git-blame-ignore-revs");
    std::fs::create_dir(&list)?;
    assert_eq!(blamer.blame(&request)?, bumped, "a folder is not a list");
    std::fs::remove_dir(&list)?;

    std::fs::write(&list, format!("# the dependency bot\n{}\n", commits.bump))?;
    let skipped = blamer.blame(&request)?;
    assert_eq!(skipped.runs, [run(1, 2, 1, &commits.first)]);
    Ok(())
}

/// A list git cannot read whole is not used, so one abbreviated id does not fail every blame: git
/// refuses such a list with `invalid object name`, and the blame goes on without it.
#[test]
fn a_list_git_would_refuse_is_not_used() -> TestResult {
    let (repo, commits) = history()?;
    let blamer = open(&repo)?;
    let path = RepoPath::new("deps.txt")?;
    let request = ask(&path, 1, 2, &commits.rename);
    let list = repo.root().join(".git-blame-ignore-revs");

    // The premise: git itself fails with the abbreviated id.
    std::fs::write(&list, format!("{}\n", &commits.bump[..12]))?;
    let refused = repo.git(&[
        "blame",
        "--ignore-revs-file",
        ".git-blame-ignore-revs",
        "HEAD",
        "--",
        "deps.txt",
    ]);
    assert!(refused.is_err(), "git read an abbreviated id");

    let blamed = blamer.blame(&request)?;
    assert_eq!(
        blamed.runs,
        [run(1, 1, 1, &commits.bump), run(2, 2, 2, &commits.first)]
    );

    std::fs::write(&list, format!("# fine\n{} bump\n", commits.bump))?;
    assert_eq!(
        blamer.blame(&request)?,
        blamed,
        "text after an id is refused too"
    );
    Ok(())
}

/// The repository's own configuration names the list of commits git opens when it starts, and
/// opens it wherever it is, so a blamer is refused for any name but the top file; for the top file
/// it blames only while that file can be used. The premise: git fails on a missing list.
#[test]
fn a_list_the_configuration_names_is_the_top_file_or_refused() -> TestResult {
    let (repo, commits) = history()?;
    let path = RepoPath::new("deps.txt")?;

    repo.git(&["config", "blame.ignoreRevsFile", "no/such/list"])?;
    let failing = repo.git(&["blame", "HEAD", "--", "deps.txt"]);
    assert!(failing.is_err(), "git did not open the list");
    let refused = Blamer::open(&repo.root()).err();
    assert_eq!(refused, Some(OpenError::NamesIgnoreList));

    repo.git(&["config", "blame.ignoreRevsFile", ".git-blame-ignore-revs"])?;
    let blamer = open(&repo)?;
    let request = ask(&path, 1, 2, &commits.rename);
    assert_eq!(blamer.blame(&request), Err(BlameError::UnusableIgnoreList));

    let list = repo.root().join(".git-blame-ignore-revs");
    std::fs::write(&list, format!("{}\n", commits.bump))?;
    let skipped = blamer.blame(&request)?;
    assert_eq!(skipped.runs, [run(1, 2, 1, &commits.first)]);

    std::fs::write(&list, "abbreviated\n")?;
    assert_eq!(blamer.blame(&request), Err(BlameError::UnusableIgnoreList));
    Ok(())
}

/// A list that is a link is not used, since the link could lead out of the repository: the link
/// here points at a list naming the bump, and the bump still owns its line.
#[cfg(unix)]
#[test]
fn a_list_that_is_a_link_is_not_used() -> TestResult {
    let (repo, commits) = history()?;
    let outside = tempfile::tempdir()?;
    let target = outside.path().join("revs");
    std::fs::write(&target, format!("{}\n", commits.bump))?;
    std::os::unix::fs::symlink(&target, repo.root().join(".git-blame-ignore-revs"))?;

    let path = RepoPath::new("deps.txt")?;
    let blame = open(&repo)?.blame(&ask(&path, 1, 2, &commits.rename))?;
    assert_eq!(
        blame.runs,
        [run(1, 1, 1, &commits.bump), run(2, 2, 2, &commits.first)]
    );
    Ok(())
}

/// Nothing older than the date is looked at. The date, the start of day 3, falls between the fix
/// (day 2) and the reindentation (day 3): the reindentation is read and the fix is not, and the
/// lines the reindentation left as they were, 6 of them written by the first commit, are blamed
/// on the fix, the newest commit before the date, which is flagged a boundary. It is where the
/// blame stopped, not where the lines came from.
#[test]
fn lines_older_than_the_date_are_blamed_on_the_boundary() -> TestResult {
    let (repo, commits) = history()?;
    let path = RepoPath::new("src/parsing/parser.py")?;
    let request = BlameRequest {
        since_s: Some(DAY_0_S + 3 * DAY_S),
        ..ask(&path, 1, 7, &commits.rename)
    };
    let blame = open(&repo)?.blame(&request)?;
    assert_eq!(blame.runs, [run(1, 7, 1, &commits.fix)]);
    let boundary = blame.commits.get(&commits.fix).ok_or("not listed")?;
    assert!(boundary.is_boundary);
    Ok(())
}

/// A blame that is given no time is stopped and says so; the same blame with time succeeds, so
/// that the stop is the time's.
#[test]
fn a_blame_past_its_time_is_a_timeout() -> TestResult {
    let (repo, commits) = history()?;
    let blamer = open(&repo)?;
    let path = RepoPath::new("deps.txt")?;
    let request = BlameRequest {
        timeout_ms: 0,
        ..ask(&path, 1, 2, &commits.rename)
    };
    assert_eq!(blamer.blame(&request), Err(BlameError::TimedOut));
    let patient = BlameRequest {
        timeout_ms: 10_000,
        ..request
    };
    assert!(blamer.blame(&patient).is_ok());
    Ok(())
}

/// A blame that prints more than 1 MiB is refused, not read in part: 400 lines of 3,000 bytes
/// print 1.2 MB of text.
#[test]
fn a_blame_that_prints_too_much_is_refused() -> TestResult {
    let repo = Repo::new()?;
    let text: String = (0..400)
        .map(|number| format!("{number:03}{}\n", "x".repeat(2_997)))
        .collect();
    repo.write("big.txt", &text)?;
    let head = repo.commit(1)?;
    let path = RepoPath::new("big.txt")?;
    let blamer = open(&repo)?;
    assert_eq!(
        blamer.blame(&ask(&path, 1, 400, &head)),
        Err(BlameError::TooLarge)
    );
    let small = blamer.blame(&ask(&path, 1, 100, &head))?;
    assert_eq!(small.runs, [run(1, 100, 1, &head)]);
    Ok(())
}

/// A text conversion the repository's configuration sets up for the file is never run: git's own
/// blame runs it, which leaves its mark, and the blame here leaves none.
#[test]
fn a_text_conversion_the_repository_names_is_never_run() -> TestResult {
    let repo = Repo::new()?;
    repo.write(".gitattributes", "*.txt diff=probe\n")?;
    repo.write("a.txt", "one\ntwo\n")?;
    let first = repo.commit(1)?;
    repo.write("a.txt", "one\nTWO\n")?;
    repo.commit(2)?;

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
        "[diff \"probe\"]\n\ttextconv = '{}'\n",
        probe_path.replace('\\', "/")
    ));
    std::fs::write(&config, text)?;

    let head = repo.git(&["rev-parse", "HEAD"])?;
    repo.git(&["blame", "--porcelain", "-L", "1,2", "HEAD", "--", "a.txt"])?;
    assert!(
        marker.exists(),
        "git's own blame did not run the text conversion"
    );
    std::fs::remove_file(&marker)?;

    let path = RepoPath::new("a.txt")?;
    let blame = open(&repo)?.blame(&ask(&path, 1, 2, &head))?;
    assert!(!marker.exists(), "the blame ran the text conversion");
    assert_eq!(
        blame.runs.first().map(|found| found.oid.as_str()),
        Some(first.as_str())
    );
    Ok(())
}

/// A repository whose own configuration names a filter is refused before git runs, wherever it
/// names it as far as the work tree reader's tests go; and a folder with no repository is said
/// to have none.
#[test]
fn a_filter_or_no_repository_is_refused() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.txt", "one\n")?;
    repo.commit(1)?;
    repo.git(&["config", "filter.marking.clean", "touch filtered; cat"])?;
    repo.write(".gitattributes", "* filter=marking\n")?;
    assert_eq!(Blamer::open(&repo.root()).err(), Some(OpenError::Refused));
    assert!(!repo.root().join("filtered").exists());

    let folder = tempfile::tempdir()?;
    assert_eq!(
        Blamer::open(folder.path()).err(),
        Some(OpenError::NoRepository)
    );
    Ok(())
}

/// A partial clone is blamed only by a git that can be told not to fetch what the clone left out,
/// 2.45 or later, and then without fetching. The origin stays, so that a fetch would succeed:
/// git's own blame in a second clone fetches the old blob, and in the first the blame fails and
/// the blob is still missing afterwards. With an older git the blamer is refused before git runs.
#[test]
fn a_partial_clone_is_blamed_without_fetching() -> TestResult {
    let origin = Repo::new()?;
    origin.git(&["config", "uploadpack.allowFilter", "true"])?;
    origin.write("a.txt", &lines("old", 3))?;
    let first = origin.commit(1)?;
    origin.write("a.txt", &lines("new", 3))?;
    let head = origin.commit(2)?;
    let old_blob = origin.git(&["rev-parse", &format!("{first}:a.txt")])?;
    let clone = Repo::clone_of(&origin, &["--filter=blob:none"])?;
    let witness = Repo::clone_of(&origin, &["--filter=blob:none"])?;

    let path = RepoPath::new("a.txt")?;
    if git_version()? < (2, 45) {
        assert_eq!(
            Blamer::open(&clone.root()).err(),
            Some(OpenError::WouldFetch)
        );
        return Ok(());
    }
    let missing = |repo: &Repo| repo.git(&["--no-lazy-fetch", "cat-file", "-e", &old_blob]);

    // The premise: the old blob is missing, and git's own blame fetches it from the origin.
    assert!(missing(&witness).is_err(), "the clone holds the old blob");
    witness.git(&["blame", "-L", "1,3", "HEAD", "--", "a.txt"])?;
    assert!(missing(&witness).is_ok(), "git's blame did not fetch");

    assert!(missing(&clone).is_err());
    let blamed = open(&clone)?.blame(&ask(&path, 1, 3, &head));
    assert!(matches!(blamed, Err(BlameError::Git(_))), "{blamed:?}");
    assert!(missing(&clone).is_err(), "the blame fetched the old blob");
    Ok(())
}

/// A request that cannot be a blame is refused before git runs: line 0, a span that ends before
/// it starts, one of 501 lines, a revision name where an object id is needed, a date before git
/// reads dates as times, and the repository's root for a file. The span of 500 lines is allowed.
/// The earliest date that is a time is the one git reads as one: 100,000,000.
#[test]
fn a_request_that_is_not_a_blame_is_refused() -> TestResult {
    let (repo, commits) = history()?;
    let blamer = open(&repo)?;
    let path = RepoPath::new("deps.txt")?;
    let root = RepoPath::root();
    let at = commits.first.as_str();
    let cases = [
        ("line 0", ask(&path, 0, 1, at)),
        ("backwards", ask(&path, 3, 2, at)),
        ("501 lines", ask(&path, 1, 501, at)),
        ("HEAD", ask(&path, 1, 1, "HEAD")),
        ("a short id", ask(&path, 1, 1, &at[..10])),
        (
            "a date before the epoch",
            BlameRequest {
                since_s: Some(-1),
                ..ask(&path, 1, 1, at)
            },
        ),
        (
            "a date of 0",
            BlameRequest {
                since_s: Some(0),
                ..ask(&path, 1, 1, at)
            },
        ),
        (
            "a date git reads as a day",
            BlameRequest {
                since_s: Some(99_999_999),
                ..ask(&path, 1, 1, at)
            },
        ),
        ("the root", ask(&root, 1, 1, at)),
    ];
    for (name, request) in cases {
        let result = blamer.blame(&request);
        assert!(
            matches!(result, Err(BlameError::BadRequest(_))),
            "{name}: {result:?}"
        );
    }
    let earliest = BlameRequest {
        since_s: Some(100_000_000),
        ..ask(&path, 1, 2, &commits.rename)
    };
    assert_eq!(blamer.blame(&earliest)?.runs.len(), 2);
    // 500 lines pass the check; git clamps the span to the file's 2 lines, which are not the
    // lines asked for.
    let long = blamer.blame(&ask(&path, 1, 500, at));
    assert!(matches!(long, Err(BlameError::Malformed(_))), "{long:?}");
    Ok(())
}

/// A line moved within its file keeps the commit that wrote it: the second commit moves the first
/// line of four to the end, and the blame gives all four lines to the first commit, in two runs
/// since the moved line stood at line 1 of the commit's file and not after line 4: lines 1 to 3
/// stood at 2 to 4. The premise: without `-M` git blames the moved line on the second commit.
#[test]
fn a_line_moved_within_its_file_keeps_its_commit() -> TestResult {
    let repo = Repo::new()?;
    let (alpha, beta, gamma, delta) = (
        "alpha_function_goes_here_one()\n",
        "beta_function_goes_here_two()\n",
        "gamma_function_goes_here_three()\n",
        "delta_function_goes_here_four()\n",
    );
    repo.write("a.txt", &format!("{alpha}{beta}{gamma}{delta}"))?;
    let first = repo.commit(1)?;
    repo.write("a.txt", &format!("{beta}{gamma}{delta}{alpha}"))?;
    let moved = repo.commit(2)?;

    let plain = repo.git(&["blame", "--porcelain", "-L", "4,4", "HEAD", "--", "a.txt"])?;
    assert_eq!(plain.split_whitespace().next(), Some(moved.as_str()));

    let path = RepoPath::new("a.txt")?;
    let blame = open(&repo)?.blame(&ask(&path, 1, 4, &moved))?;
    assert_eq!(blame.runs, [run(1, 3, 2, &first), run(4, 4, 1, &first)]);
    Ok(())
}
