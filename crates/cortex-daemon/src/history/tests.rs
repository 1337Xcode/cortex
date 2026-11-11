//! The history worker on repositories made with the `git` program at fixed dates and names, and a
//! store holding the files HEAD has as the index stores them, the writer's part played by the
//! test: what a blame request leaves in the store and the text cache, what a failed blame leaves,
//! a refused repository, what upkeep warms, and that with both switches off nothing starts. Every expectation is
//! worked out by hand from the history each test builds.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_core::{
    Config, ContentHash, FileFlags, HistoryConfig, Identifier, InterestTier, PositionMode,
    RepoPath, ShortHash, SymbolKind,
};
use cortex_store::{
    BlameRecord, BlameRunRecord, CardKind, ChangeCardRecord, FileIndex, FileRecord, InterestRow,
    NodeId, PositionRecord, SymbolRecord, Writer,
};

use super::blaming::MAX_CARDS_A_JOB;
use super::queue::{BlameJob, Job};
use super::texts::CardText;
use super::{History, Jobs, store};
use crate::inbox::{Inbox, Work};
use crate::test_store::{TestResult, writer};
use crate::upkeep::{LOG_LIMIT_BYTES, Upkeep, UpkeepSettings};

/// 2026-01-01T00:00:00Z in seconds since the Unix epoch: day 0 of every history here.
const DAY_0_S: i64 = 1_767_225_600;

/// The seconds in a day: a commit made on day `n` is made at `DAY_0_S + n * DAY_S`.
const DAY_S: i64 = 86_400;

/// How long a test waits for the worker before it fails: every job here runs a few git processes.
const WAIT: Duration = Duration::from_secs(120);

/// The worker's clock: day 10, in UTC milliseconds.
fn day_10_ms() -> i64 {
    (DAY_0_S + 10 * DAY_S) * 1_000
}

/// The time of day `day`, in seconds.
fn day(day: i64) -> i64 {
    DAY_0_S + day * DAY_S
}

/// The parser as its first commit wrote it.
const PARSER_V1: &str = "def parse(items):\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The parser after the fix: lines 2 and 3 are new.
const PARSER_FIX: &str = "def parse(items):\n    if not items:\n        raise ValueError('no items')\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The parser after the formatting commit: a comment put above the fix's lines, which moves them
/// down one, and the quotes of the fix's second line changed.
const PARSER_STYLE: &str = "def parse(items):\n    # Nothing to parse is an error.\n    if not items:\n        raise ValueError(\"no items\")\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The parser as HEAD holds it, moved: line 1 changed by the mass commit.
const PARSER_HEAD: &str = "def parse(items, strict=False):\n    # Nothing to parse is an error.\n    if not items:\n        raise ValueError(\"no items\")\n    out = []\n    for item in items:\n        out.append(item)\n    return out\n";

/// The test the fix added.
const TEST_FILE: &str = "def test_empty():\n    assert parse([]) == []\n";

/// The dependencies HEAD holds, after the bot's bump of line 1.
const DEPS_HEAD: &str = "lib==1.1\nother==2.0\n";

/// A repository made for one test, in a temporary folder with an empty home folder.
struct Repo {
    /// The folder holding the work tree (`repo`) and git's home folder (`home`).
    folder: tempfile::TempDir,
}

impl Repo {
    /// A new repository on `main` with no commit yet, git leaving the store's folder out.
    fn new() -> TestResult<Self> {
        let folder = tempfile::tempdir()?;
        std::fs::create_dir(folder.path().join("repo"))?;
        std::fs::create_dir(folder.path().join("home"))?;
        let repo = Self { folder };
        repo.git(&["init", "-q", "-b", "main"], None)?;
        repo.git(&["config", "core.autocrlf", "false"], None)?;
        repo.git(&["config", "commit.gpgsign", "false"], None)?;
        std::fs::write(repo.root().join(".git/info/exclude"), ".cortex/\n")?;
        Ok(repo)
    }

    /// The top of the work tree.
    fn root(&self) -> PathBuf {
        self.folder.path().join("repo")
    }

    /// Writes `text` to `path` in the work tree, making its folders.
    fn write(&self, path: &str, text: &str) -> TestResult {
        let file = self.root().join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, text)?;
        Ok(())
    }

    /// Commits every change on day `on` as `author` (the test's own name when none) with
    /// `message`: the new commit's id.
    fn commit(&self, on: i64, author: Option<&str>, message: &str) -> TestResult<String> {
        self.git(&["add", "-A"], None)?;
        let mut args = vec!["commit", "-q", "-m", message];
        if let Some(author) = author {
            args.extend(["--author", author]);
        }
        self.git(&args, Some(on))?;
        self.git(&["rev-parse", "HEAD"], None)
    }

    /// Runs git with `args` in the work tree, reading no configuration but the repository's own,
    /// every time it records that of day `on` when one is given: what it printed, trimmed.
    fn git(&self, args: &[&str], on: Option<i64>) -> TestResult<String> {
        let home = self.folder.path().join("home");
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(self.root())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Cortex Test")
            .env("GIT_AUTHOR_EMAIL", "test")
            .env("GIT_COMMITTER_NAME", "Cortex Test")
            .env("GIT_COMMITTER_EMAIL", "test");
        for name in [
            "GIT_CONFIG_GLOBAL",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "XDG_CONFIG_HOME",
        ] {
            command.env_remove(name);
        }
        if let Some(on) = on {
            let time = format!("{} +0000", day(on));
            command
                .env("GIT_AUTHOR_DATE", &time)
                .env("GIT_COMMITTER_DATE", &time);
        }
        let output = command.output()?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git {args:?} failed: {message}").into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

/// The commits of [`history`], oldest first.
struct Commits {
    /// Day 1: the parser and the dependencies.
    first: String,
    /// Day 2: thirty-five modules.
    modules: String,
    /// Day 3: the fix, with its test, closing issue 12.
    fix: String,
    /// Day 4: double quotes in the fix's line 3.
    style: String,
    /// Day 5: a bot's bump of the dependencies' line 1.
    bump: String,
    /// Day 6: the parser and the modules moved, the parser's line 1 changed: 36 files.
    mass: String,
}

/// A repository with six commits, one a day from day 1: the parser and the dependencies; 35
/// modules; the fix, which puts two lines after the parser's first and adds a test; a formatting
/// commit that puts a comment above the fix's lines and changes the quotes of its second; a bot's
/// bump of the first dependency; and a move of the parser to `src/parsing/` and the modules to
/// `mods/` that also changes the parser's first line.
fn history() -> TestResult<(Repo, Commits)> {
    let repo = Repo::new()?;
    repo.write("src/parser.py", PARSER_V1)?;
    repo.write("deps.txt", "lib==1.0\nother==2.0\n")?;
    let first = repo.commit(1, None, "add the parser")?;
    for number in 0..35 {
        repo.write(&format!("pkg/m{number:02}.py"), &format!("mod {number}\n"))?;
    }
    let modules = repo.commit(2, None, "add the modules")?;
    repo.write("src/parser.py", PARSER_FIX)?;
    repo.write("tests/test_parser.py", TEST_FILE)?;
    let fix = repo.commit(
        3,
        None,
        "Fix the crash on empty input\n\nParsing no items raised an error.\n\nCloses #12",
    )?;
    repo.write("src/parser.py", PARSER_STYLE)?;
    let style = repo.commit(4, None, "style: use double quotes")?;
    repo.write("deps.txt", DEPS_HEAD)?;
    let bump = repo.commit(
        5,
        Some("dependabot[bot] <bot@example.com>"),
        "chore(deps): bump lib from 1.0 to 1.1",
    )?;
    std::fs::remove_file(repo.root().join("src/parser.py"))?;
    repo.write("src/parsing/parser.py", PARSER_HEAD)?;
    for number in 0..35 {
        let from = repo.root().join(format!("pkg/m{number:02}.py"));
        let text = std::fs::read_to_string(&from)?;
        std::fs::remove_file(from)?;
        repo.write(&format!("mods/m{number:02}.py"), &text)?;
    }
    let mass = repo.commit(6, None, "move the parser and the modules")?;
    let commits = Commits {
        first,
        modules,
        fix,
        style,
        bump,
        mass,
    };
    Ok((repo, commits))
}

/// The nodes of the files the store holds.
struct Nodes {
    /// `deps.txt`.
    deps: NodeId,
    /// `tests/test_parser.py`, which holds a test.
    test: NodeId,
}

/// Stores the file at `path` holding `text` as the index would, with a test in it when
/// `is_test`: its node.
fn index(writer: &mut Writer, path: &str, text: &str, is_test: bool) -> TestResult<NodeId> {
    let lines = u32::try_from(text.lines().count())?;
    let symbols = if is_test {
        vec![SymbolRecord {
            parent: None,
            kind: SymbolKind::Test,
            name: Identifier::new("test_empty")?,
            qual_name: Identifier::new("test_empty")?,
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            end_line: lines,
            sig_hash: ShortHash::of(b"test_empty"),
            body_hash: ShortHash::of(b"test_empty"),
        }]
    } else {
        Vec::new()
    };
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: RepoPath::new(path)?,
            language: None,
            size_bytes: u64::try_from(text.len())?,
            line_count: Some(u64::from(lines)),
            content_hash: Some(ContentHash::of(text.as_bytes())),
            mtime_ns: 1,
            flags: FileFlags::empty(),
            indexed_at_ms: 1,
        },
        symbols,
        refs: Vec::new(),
        passes_on: Vec::new(),
    })?;
    let node = transaction
        .queries()
        .file_node_at(&RepoPath::new(path)?)?
        .ok_or("the file's node")?;
    transaction.commit()?;
    Ok(node)
}

/// The store of `repo` holding the three files HEAD has that the tests blame or name.
fn indexed(repo: &Repo) -> TestResult<(Writer, Nodes)> {
    let mut writer = writer(&repo.root())?;
    index(&mut writer, "src/parsing/parser.py", PARSER_HEAD, false)?;
    let deps = index(&mut writer, "deps.txt", DEPS_HEAD, false)?;
    let test = index(&mut writer, "tests/test_parser.py", TEST_FILE, true)?;
    Ok((writer, Nodes { deps, test }))
}

/// The settings with `history.why` on and a blame given `blame_timeout_ms`.
fn history_on(blame_timeout_ms: u32) -> Config {
    Config {
        history: HistoryConfig {
            is_why: true,
            blame_timeout_ms,
            ..HistoryConfig::default()
        },
        ..Config::default()
    }
}

/// A worker running on `repo` under `config` at day 10.
struct Running {
    /// The writer's inbox, which the test serves.
    inbox: Arc<Inbox>,
    /// The worker's jobs.
    jobs: Arc<Jobs>,
    /// The worker, stopped when dropped.
    _history: History,
}

/// Starts the worker on `repo` under `config`, its clock at day 10.
fn start(repo: &Repo, config: &Config) -> TestResult<Running> {
    start_carding(repo, config, MAX_CARDS_A_JOB)
}

/// Starts the worker as [`start`] does, making at most `max_cards` cards a job.
fn start_carding(repo: &Repo, config: &Config, max_cards: usize) -> TestResult<Running> {
    let inbox = Arc::new(Inbox::default());
    let jobs = Arc::new(Jobs::default());
    let history = History::start_with(
        &repo.root(),
        config,
        (&inbox, &jobs),
        (day_10_ms, max_cards),
    )
    .ok_or("the worker did not start")?;
    Ok(Running {
        inbox,
        jobs,
        _history: history,
    })
}

/// Plays the writer's part for `running` with `writer` until `is_done` says so, writing what the
/// worker hands over as the daemon's writer does.
fn serve_until(
    running: &Running,
    writer: &mut Writer,
    mut is_done: impl FnMut(&Writer, &Jobs) -> TestResult<bool>,
) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while !is_done(writer, &running.jobs)? {
        if Instant::now() > deadline {
            return Err("the worker did not finish in time".into());
        }
        match running
            .inbox
            .wait(Some(Instant::now() + Duration::from_millis(50)))
        {
            Work::History(job) => {
                let stored = store(writer, &job.write);
                job.answer(stored.map_err(|error| error.to_string()));
            }
            Work::Timeout => {}
            other => return Err(format!("the writer was asked {other:?}").into()),
        }
    }
    Ok(())
}

/// A blame of lines `first..=last` of the file at `path` holding `text`.
fn span(path: &str, text: &str, (first, last): (u32, u32)) -> TestResult<BlameJob> {
    Ok(BlameJob {
        path: RepoPath::new(path)?,
        file_hash: ContentHash::of(text.as_bytes()),
        first_line: first,
        last_line: last,
    })
}

/// The run of lines `start..=end`, from line `source_start` of `oid`'s file.
fn run(start: u32, end: u32, source_start: u32, oid: &str) -> BlameRunRecord {
    BlameRunRecord {
        start,
        end,
        source_start,
        oid: oid.to_owned(),
    }
}

/// Whether the store keeps a blame of exactly `job`'s span.
fn is_kept(writer: &Writer, job: &BlameJob) -> TestResult<bool> {
    Ok(writer
        .queries()
        .cached_blames((&job.path, &job.file_hash), job.first_line, job.last_line)?
        .iter()
        .any(|kept| kept.first_line == job.first_line && kept.last_line == job.last_line))
}

/// A card of `oid` made at day 10, committed on day `on`, after `parent`, of `kind`, naming
/// `issues`, with a reason when `has_reason`, changing `files` of which `tests` hold tests.
fn card(
    oid: &str,
    (on, parent): (i64, Option<&str>),
    (kind, issues, has_reason): (CardKind, &[&str], bool),
    (files, tests): (Vec<NodeId>, Vec<NodeId>),
) -> ChangeCardRecord {
    ChangeCardRecord {
        oid: oid.to_owned(),
        first_parent: parent.map(str::to_owned),
        committed_at_s: day(on),
        kind,
        pull_request: None,
        issues: issues.iter().map(|issue| (*issue).to_owned()).collect(),
        has_reason,
        files,
        symbols: Vec::new(),
        tests,
        carded_at_ms: day_10_ms(),
    }
}

/// The parser's lines 3 to 8 at HEAD: line 3 is the fix's (its 2), line 4 the formatting commit's,
/// which changed its quotes after a comment it put above moved it down a line, and 5 to 8 the first
/// commit's (its 2 to 5). The formatting commit is looked through: git maps line 4 through it to
/// the fix's line 3, which joins line 3; a walk to the commit before at the run's own line number
/// (4) would have found the first commit's `out = []`. The parser's line 1 is the move's, looked
/// through to the first commit's line 1. The dependencies' line 1 is the bot's, looked through to
/// the first commit's line 1, which joins its line 2.
///
/// Every commit the blames named is carded, with the kinds read off the history: the first commit
/// normal, a root, its only file still here `deps.txt` (its parser moved since); the fix normal,
/// closing `#12`, with a reason line, its test file both a file and a test; the formatting commit
/// trivial by its `style:` subject; the bump a bot's; the move a mass change with no file. The
/// modules' commit owns no line and has no card. The text cache holds the fix's subject and reason
/// line and the move's subject.
#[test]
fn a_blame_fills_the_cache_and_cards_every_commit_it_names() -> TestResult {
    let (repo, commits) = history()?;
    let (mut writer, nodes) = indexed(&repo)?;
    let running = start(&repo, &history_on(10_000))?;
    let parser = span("src/parsing/parser.py", PARSER_HEAD, (3, 8))?;
    let first_line = span("src/parsing/parser.py", PARSER_HEAD, (1, 1))?;
    let deps = span("deps.txt", DEPS_HEAD, (1, 2))?;
    for job in [&parser, &first_line, &deps] {
        assert!(running.jobs.ask_blame(job.clone()));
    }
    serve_until(&running, &mut writer, |writer, _| {
        Ok(is_kept(writer, &parser)? && is_kept(writer, &first_line)? && is_kept(writer, &deps)?)
    })?;

    let runs_of = |job: &BlameJob| -> TestResult<Vec<Vec<BlameRunRecord>>> {
        Ok(writer
            .queries()
            .cached_blames((&job.path, &job.file_hash), job.first_line, job.last_line)?
            .into_iter()
            .filter(|kept| kept.first_line == job.first_line && kept.last_line == job.last_line)
            .map(|kept| kept.runs)
            .collect())
    };
    assert_eq!(
        runs_of(&parser)?,
        [vec![
            run(3, 4, 2, &commits.fix),
            run(5, 8, 2, &commits.first)
        ]]
    );
    assert_eq!(runs_of(&first_line)?, [vec![run(1, 1, 1, &commits.first)]]);
    assert_eq!(runs_of(&deps)?, [vec![run(1, 2, 1, &commits.first)]]);

    let every = [
        &commits.first,
        &commits.modules,
        &commits.fix,
        &commits.style,
        &commits.bump,
        &commits.mass,
    ]
    .map(String::clone);
    let mut expected = vec![
        card(
            &commits.first,
            (1, None),
            (CardKind::Normal, &[], false),
            (vec![nodes.deps], Vec::new()),
        ),
        card(
            &commits.fix,
            (3, Some(&commits.modules)),
            (CardKind::Normal, &["#12"], true),
            (vec![nodes.test], vec![nodes.test]),
        ),
        card(
            &commits.style,
            (4, Some(&commits.fix)),
            (CardKind::Trivial, &[], false),
            (Vec::new(), Vec::new()),
        ),
        card(
            &commits.bump,
            (5, Some(&commits.style)),
            (CardKind::Bot, &[], false),
            (vec![nodes.deps], Vec::new()),
        ),
        card(
            &commits.mass,
            (6, Some(&commits.bump)),
            (CardKind::Mass, &[], false),
            (Vec::new(), Vec::new()),
        ),
    ];
    expected.sort_by(|left, right| left.oid.cmp(&right.oid));
    assert_eq!(writer.queries().change_cards(&every)?, expected);

    assert_eq!(
        running.jobs.text(&commits.fix),
        Some(CardText {
            subject: "Fix the crash on empty input".to_owned(),
            reason: Some("Parsing no items raised an error.".to_owned()),
        })
    );
    assert_eq!(
        running.jobs.text(&commits.mass),
        Some(CardText {
            subject: "move the parser and the modules".to_owned(),
            reason: None,
        })
    );
    let report = running.jobs.report().ok_or("no report")?;
    assert_eq!((report.cards, report.cached_blames), (5, 3));
    assert_eq!((report.failed_blames, report.refused), (0, None));
    Ok(())
}

/// Every text column of every card row, read from the database itself, holds ids, a kind's name
/// or JSON of numbers and references: none holds a word of a subject, a reason line or an
/// author's name.
#[test]
fn a_card_row_holds_no_text_a_person_wrote() -> TestResult {
    let (repo, _) = history()?;
    let (mut writer, _) = indexed(&repo)?;
    let running = start(&repo, &history_on(10_000))?;
    let parser = span("src/parsing/parser.py", PARSER_HEAD, (3, 8))?;
    assert!(running.jobs.ask_blame(parser.clone()));
    serve_until(&running, &mut writer, |writer, _| is_kept(writer, &parser))?;
    drop(running);

    let database = rusqlite::Connection::open(repo.root().join(".cortex").join("cortex.db"))?;
    let mut statement = database.prepare("SELECT * FROM change_cards")?;
    let columns = statement.column_count();
    let mut texts = Vec::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        for column in 0..columns {
            if let rusqlite::types::ValueRef::Text(text) = row.get_ref(column)? {
                texts.push(String::from_utf8(text.to_vec())?);
            }
        }
    }
    // Three cards (the fix, the formatting commit and the first commit) of seven text columns,
    // but the first commit's parent, which it has none of.
    assert_eq!(texts.len(), 3 * 7 - 1);
    let written = [
        "crash", "empty", "parsing", "parser", "quotes", "move", "modules", "cortex", "test",
    ];
    for text in &texts {
        let lower = text.to_lowercase();
        for word in written {
            assert!(!lower.contains(word), "{word} in {text}");
        }
    }
    Ok(())
}

/// A blame given no time is stopped: it leaves no blame and no card, is counted, and is not run
/// again when asked for again. A card asked for after it is made, so the worker took the blame
/// asked for again before it, and the failure count is still one.
#[test]
fn a_blame_past_its_time_leaves_nothing_and_is_not_retried() -> TestResult {
    let (repo, commits) = history()?;
    let (mut writer, _) = indexed(&repo)?;
    let running = start(&repo, &history_on(0))?;
    let parser = span("src/parsing/parser.py", PARSER_HEAD, (3, 8))?;
    assert!(running.jobs.ask_blame(parser.clone()));
    let failed = |jobs: &Jobs| jobs.report().map_or(0, |report| report.failed_blames);
    serve_until(&running, &mut writer, |_, jobs| Ok(failed(jobs) == 1))?;
    assert!(running.jobs.ask_blame(parser.clone()));
    assert_eq!(running.jobs.text(&commits.fix), None);
    running.jobs.ask_text(&commits.fix);
    let fix = [commits.fix.clone()];
    serve_until(&running, &mut writer, |writer, _| {
        Ok(!writer.queries().change_cards(&fix)?.is_empty())
    })?;
    assert_eq!(failed(&running.jobs), 1);
    assert_eq!(writer.queries().cached_blame_count()?, 0);
    assert_eq!(writer.queries().change_card_count()?, 1);
    Ok(())
}

/// A repository whose own configuration names a filter is refused, said in the status and kept
/// for the daemon's life: nothing is blamed or carded.
#[test]
fn a_refused_repository_is_said_and_left_alone() -> TestResult {
    let (repo, _) = history()?;
    repo.git(&["config", "filter.lfs.clean", "git-lfs clean -- %f"], None)?;
    let (mut writer, _) = indexed(&repo)?;
    let running = start(&repo, &history_on(10_000))?;
    let parser = span("src/parsing/parser.py", PARSER_HEAD, (3, 8))?;
    assert!(running.jobs.ask_blame(parser.clone()));
    serve_until(&running, &mut writer, |_, jobs| {
        Ok(jobs.report().is_some_and(|report| report.refused.is_some()))
    })?;
    let report = running.jobs.report().ok_or("no report")?;
    assert_eq!(
        report.refused.as_deref(),
        Some("the repository's own git configuration names a filter")
    );
    assert_eq!((report.cards, report.cached_blames), (0, 0));
    Ok(())
}

/// A blame is kept under the hash of the file on disk, and only while that file is HEAD's. With
/// `core.autocrlf` on, the file on disk has CRLF line ends and the commit LF: the file is HEAD's
/// all the same, and its blame is kept under the disk's hash, where a work order finds it. Once a
/// line is added on disk and not committed, the file is not HEAD's: nothing is kept, and the span
/// is remembered as not HEAD's at that HEAD. Committed, the same content is HEAD's at the new HEAD,
/// and its blame is kept: lines 1 and 2 the first commit's, line 3 the second's. Then a line
/// changed on disk, the line count kept, is not HEAD's either: no blame of it is kept.
#[test]
fn the_file_on_disk_is_blamed_only_while_it_is_head_s() -> TestResult {
    let repo = Repo::new()?;
    repo.git(&["config", "core.autocrlf", "true"], None)?;
    let first_text = "def a():\r\n    return 1\r\n";
    repo.write("a.py", first_text)?;
    let first = repo.commit(1, None, "add a")?;
    let blob = repo.git(&["cat-file", "-p", "HEAD:a.py"], None)?;
    assert!(!blob.contains('\r'), "the premise: git stores LF");
    let mut writer = writer(&repo.root())?;
    index(&mut writer, "a.py", first_text, false)?;
    let running = start(&repo, &history_on(10_000))?;
    let whole = span("a.py", first_text, (1, 2))?;
    assert!(running.jobs.ask_blame(whole.clone()));
    serve_until(&running, &mut writer, |writer, _| is_kept(writer, &whole))?;
    let kept = writer
        .queries()
        .cached_blames((&whole.path, &whole.file_hash), 1, 2)?;
    let runs: Vec<_> = kept.into_iter().map(|kept| kept.runs).collect();
    assert_eq!(runs, [vec![run(1, 2, 1, &first)]]);

    let edited_text = "def a():\r\n    return 1\r\nb = 2\r\n";
    repo.write("a.py", edited_text)?;
    index(&mut writer, "a.py", edited_text, false)?;
    let edited = span("a.py", edited_text, (1, 3))?;
    assert!(running.jobs.ask_blame(edited.clone()));
    let head = repo.git(&["rev-parse", "HEAD"], None)?;
    serve_until(&running, &mut writer, |_, jobs| {
        Ok(jobs.is_unblameable_at(&edited, &head))
    })?;
    assert!(!is_kept(&writer, &edited)?);

    let second = repo.commit(2, None, "add b")?;
    assert!(running.jobs.ask_blame(edited.clone()));
    serve_until(&running, &mut writer, |writer, _| is_kept(writer, &edited))?;
    let kept = writer
        .queries()
        .cached_blames((&edited.path, &edited.file_hash), 1, 3)?;
    let runs: Vec<_> = kept
        .into_iter()
        .filter(|kept| kept.last_line == 3)
        .map(|kept| kept.runs)
        .collect();
    assert_eq!(runs, [vec![run(1, 2, 1, &first), run(3, 3, 3, &second)]]);

    let same_length = "def a():\r\n    return 2\r\nb = 2\r\n";
    repo.write("a.py", same_length)?;
    index(&mut writer, "a.py", same_length, false)?;
    let changed = span("a.py", same_length, (1, 3))?;
    assert!(running.jobs.ask_blame(changed.clone()));
    serve_until(&running, &mut writer, |_, jobs| {
        Ok(jobs.is_unblameable_at(&changed, &second))
    })?;
    assert!(!is_kept(&writer, &changed)?);
    Ok(())
}

/// Two files of the same content are blamed apart: `a.py` and `b.py` hold the same line, written
/// by different commits, and each file's blame names its own commit, under the same hash.
#[test]
fn files_of_the_same_content_are_blamed_apart() -> TestResult {
    let repo = Repo::new()?;
    let text = "x = 1\n";
    repo.write("a.py", text)?;
    let first = repo.commit(1, None, "add a")?;
    repo.write("b.py", text)?;
    let second = repo.commit(2, None, "add b")?;
    let mut writer = writer(&repo.root())?;
    index(&mut writer, "a.py", text, false)?;
    index(&mut writer, "b.py", text, false)?;
    let running = start(&repo, &history_on(10_000))?;
    let of_a = span("a.py", text, (1, 1))?;
    let of_b = span("b.py", text, (1, 1))?;
    assert_eq!(of_a.file_hash, of_b.file_hash);
    assert!(running.jobs.ask_blame(of_a.clone()));
    assert!(running.jobs.ask_blame(of_b.clone()));
    serve_until(&running, &mut writer, |writer, _| {
        Ok(is_kept(writer, &of_a)? && is_kept(writer, &of_b)?)
    })?;
    for (job, oid) in [(&of_a, &first), (&of_b, &second)] {
        let kept = writer
            .queries()
            .cached_blames((&job.path, &job.file_hash), 1, 1)?;
        let runs: Vec<_> = kept.into_iter().map(|kept| kept.runs).collect();
        assert_eq!(runs, [vec![run(1, 1, 1, oid)]], "{}", job.path);
    }
    Ok(())
}

/// A card whose commit can no longer be read (gone from the repository, as after a rewritten
/// history is collected) has its text asked for once: the read fails, the commit is remembered,
/// and asking again queues nothing.
#[test]
fn a_text_that_cannot_be_read_is_not_asked_for_again() -> TestResult {
    let repo = Repo::new()?;
    repo.write("a.py", "a = 1\n")?;
    repo.commit(1, None, "add a")?;
    let mut writer = writer(&repo.root())?;
    let gone = "f".repeat(40);
    let mut transaction = writer.transaction()?;
    transaction.put_change_card(
        &card(
            &gone,
            (1, None),
            (CardKind::Normal, &[], true),
            (Vec::new(), Vec::new()),
        ),
        &[],
    )?;
    transaction.commit()?;
    let running = start(&repo, &history_on(10_000))?;
    running.jobs.ask_text(&gone);
    serve_until(&running, &mut writer, |_, jobs| {
        Ok(jobs.is_uncardable(&gone))
    })?;
    running.jobs.ask_text(&gone);
    assert_eq!(
        running.jobs.report().map(|report| report.jobs_waiting),
        Some(0)
    );
    Ok(())
}

/// A commit left uncarded by a job's cap is looked through all the same, once it is carded: with
/// one card a job, the blame of lines 1 to 4 cards the fix, which owns lines 1 to 3, and leaves
/// the formatting commit, which owns line 4, to a card job; the span is then looked at again, the
/// formatting commit now known as trivial and looked through to the first commit, which wrote line
/// 4. The blame kept ends with the first commit's line 4.
#[test]
fn a_commit_carded_later_is_looked_through_on_a_second_look() -> TestResult {
    let repo = Repo::new()?;
    repo.write("c.py", "a = 0\nb = 0\nc = 0\nd = 1\n")?;
    let first = repo.commit(1, None, "add c")?;
    repo.write("c.py", "a = 1\nb = 1\nc = 1\nd = 1\n")?;
    let fix = repo.commit(2, None, "Fix the counts\n\nThey started at zero.")?;
    let text = "a = 1\nb = 1\nc = 1\nd = (1)\n";
    repo.write("c.py", text)?;
    repo.commit(3, None, "style: parenthesise d")?;
    let mut writer = writer(&repo.root())?;
    index(&mut writer, "c.py", text, false)?;
    let running = start_carding(&repo, &history_on(10_000), 1)?;
    let whole = span("c.py", text, (1, 4))?;
    assert!(running.jobs.ask_blame(whole.clone()));
    let looked_through = vec![run(1, 3, 1, &fix), run(4, 4, 4, &first)];
    serve_until(&running, &mut writer, |writer, _| {
        let kept = writer
            .queries()
            .cached_blames((&whole.path, &whole.file_hash), 1, 4)?;
        Ok(kept.iter().any(|kept| kept.runs == looked_through))
    })?;
    Ok(())
}

/// With both switches off no worker starts, no job is taken and the status says nothing of
/// history; either switch alone starts one.
#[test]
fn with_both_switches_off_nothing_starts() -> TestResult {
    let repo = Repo::new()?;
    let inbox = Arc::new(Inbox::default());
    let jobs = Arc::new(Jobs::default());
    let off = Config::default();
    assert!(
        History::start_with(
            &repo.root(),
            &off,
            (&inbox, &jobs),
            (day_10_ms, MAX_CARDS_A_JOB)
        )
        .is_none()
    );
    assert!(!jobs.ask_blame(span("a.py", "a\n", (1, 1))?));
    assert_eq!(jobs.report(), None);
    let seeds = Config {
        history: HistoryConfig {
            is_seeds: true,
            ..HistoryConfig::default()
        },
        ..Config::default()
    };
    let started = History::start_with(
        &repo.root(),
        &seeds,
        (&inbox, &jobs),
        (day_10_ms, MAX_CARDS_A_JOB),
    );
    assert!(started.is_some());
    assert!(jobs.report().is_some());
    drop(started);
    assert_eq!(jobs.report(), None);
    Ok(())
}

/// The text of a two-line file named for `path`.
fn two_lines(path: &str) -> String {
    format!("{path} 1\n{path} 2\n")
}

/// Stores, in the store of the repository at `root`, the file at `path` holding
/// [`two_lines`], with `commits` commits of the mined history, and `tier` as its interest when
/// one is given: its node.
fn index_with_history(
    (writer, root): (&mut Writer, &std::path::Path),
    path: &str,
    (commits, tier): (u32, Option<InterestTier>),
) -> TestResult<NodeId> {
    let node = index(writer, path, &two_lines(path), false)?;
    let mut transaction = writer.transaction()?;
    if let Some(tier) = tier {
        transaction.replace_interest(&[InterestRow {
            node,
            score: 4.0,
            last_seq: 0,
            tier,
        }])?;
    }
    let file = transaction
        .queries()
        .file(&RepoPath::new(path)?)?
        .ok_or("the file's row")?;
    transaction.commit()?;
    let database = rusqlite::Connection::open(root.join(".cortex").join("cortex.db"))?;
    database.execute(
        "INSERT INTO file_commits (file_id, count, weighted) VALUES (?1, ?2, ?2)",
        (file.id.get(), commits),
    )?;
    Ok(node)
}

/// Upkeep's history step asks the running worker for the first lines of the files worth warming,
/// in node order. By hand, at day 10: `a.py`, a landmark with 3 commits, and `b.py`, which a
/// session edited an hour ago, with 5, are asked for. Not `c.py`, a landmark with 2 commits; not
/// `d.py`, a landmark whose two lines are kept already; not `e.py`, edited two days ago; not
/// `f.py`, interesting but no landmark.
#[test]
fn upkeep_warms_landmarks_and_hot_files_with_history() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = writer(root.path())?;
    let landmark = Some(InterestTier::Landmark);
    index_with_history((&mut writer, root.path()), "a.py", (3, landmark))?;
    let b = index_with_history((&mut writer, root.path()), "b.py", (5, None))?;
    index_with_history((&mut writer, root.path()), "c.py", (2, landmark))?;
    index_with_history((&mut writer, root.path()), "d.py", (3, landmark))?;
    let e = index_with_history((&mut writer, root.path()), "e.py", (4, None))?;
    let interesting = Some(InterestTier::Interesting);
    index_with_history((&mut writer, root.path()), "f.py", (3, interesting))?;
    let hour_ms = 60 * 60 * 1_000;
    let mut transaction = writer.transaction()?;
    for (node, last_ms) in [(b, day_10_ms() - hour_ms), (e, day_10_ms() - 48 * hour_ms)] {
        transaction.keep_position(&PositionRecord {
            session_id: Identifier::new("s1")?,
            node,
            mode: PositionMode::Edit,
            since_ms: last_ms,
            last_ms,
            worktree: None,
            branch: None,
        })?;
    }
    transaction.put_blame(&BlameRecord {
        path: RepoPath::new("d.py")?,
        file_hash: ContentHash::of(two_lines("d.py").as_bytes()),
        first_line: 1,
        last_line: 2,
        runs: Vec::new(),
        computed_at_ms: 1,
    })?;
    transaction.commit()?;

    let jobs = Arc::new(Jobs::default());
    jobs.turn_on();
    let mut upkeep = Upkeep::default();
    upkeep.warm_with(Arc::clone(&jobs));
    let settings = UpkeepSettings {
        link_decay: 0.5,
        interest_decay: 0.5,
        // Positions hold three days here, so that upkeep's own step keeps both.
        longest_hold_ms: 72 * hour_ms,
        log_limit_bytes: LOG_LIMIT_BYTES,
    };
    let far = Instant::now() + Duration::from_secs(60);
    let (swept, _) = upkeep.run(&mut writer, (day_10_ms(), &settings), (far, &|| false))?;
    assert_eq!(swept.warmed, 2);
    assert_eq!(jobs.report().map(|report| report.jobs_waiting), Some(2));
    for path in ["a.py", "b.py"] {
        let asked = jobs.next_job();
        assert_eq!(
            asked,
            Some(Job::Blame(span(path, &two_lines(path), (1, 2))?)),
            "{path}"
        );
    }
    Ok(())
}
