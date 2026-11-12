//! The hook benchmarks with the history features on (`history.why`, `history.seeds` and
//! `history.experience`) and their caches warm, so that the hook path reads what the history
//! worker kept and never waits for git: the prompt's hook, which gets a work order with why-lines,
//! and the edit's, which gets the note of where the lines it replaced came from. And the storage
//! the history tables take at 10,000 files.
//!
//! The repository is the index benchmark's 10,000 generated files with a scripted year of history
//! ([`repository`]). Its daemon indexes it and mines it; a session reads 200 of the files the
//! history changed, and upkeep, asked to run until its day's warming is spent, has the history
//! worker blame them and card the commits the blames name; the prompt's own work order has the
//! spans it shows blamed; three tasks, each a prompt, an edit and a stop, leave their records. Then,
//! the daemon stopped, the history tables' share of `.cortex/` is measured and must be under 2%
//! ([`storage`]). The daemon is started again and, before anything is measured, a prompt from a
//! new session is checked to carry a why-line, which also has the commits' texts read again, and
//! the edit, made once the index read the edited file, is checked to be told its origin.
//!
//! Each sample is a session of its own: a start, not measured, then the prompt, measured; or a
//! start and a prompt, not measured, then the edit, measured, so that each edit is its task's
//! first and looks its origin up. Every measured prompt must carry a why-line and every measured
//! edit its origin note, and the prompts must leave the number of blames kept as it was: the
//! cache was warm for every span they showed.

mod payloads;
mod repository;
mod storage;

use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, Instant};

use cortex_core::ipc::{HistoryReport, RequestBody, ResponseBody, Role, StatusReport};
use cortex_core::{ContentHash, RepoId, RepoPath, RunFiles, UserFolder, canonical_root};
use cortex_store::Reader;
use criterion::Criterion;
use serde_json::Value;

use self::payloads::Edit;
use self::repository::{Changed, changed, write_history};
use crate::common::git_of;
use crate::common::{Daemon, StopsDaemon, TestResult};
use crate::{payload, printed_in, single_runs, started};

/// The repository's configuration: the three history switches on, and no task held out, so every
/// prompt can get a work order.
const CONFIG: &str = "[history]\nwhy = true\nseeds = true\nexperience = true\n\n\
                      [holdout]\nshare = 0\n";

/// The files the reading session reads, of those the history changed: as many as upkeep warms in
/// a day (`MAX_WARMED_A_DAY` of the daemon's warming), so that a day's warming is spent.
const READ_FILES: u64 = 200;

/// Runs of upkeep asked for: each warms at most 32 files (`MAX_WARMED_A_PASS`), so seven reach
/// the day's 200.
const WARM_PASSES: usize = 7;

/// How long a run of upkeep is left alone before its status is asked: a status request is a
/// request, which stops upkeep, so it is asked rarely. A run stopped goes on at the next ask.
const UPKEEP_QUIET: Duration = Duration::from_secs(5);

/// The most runs of upkeep asked for one pass of warming before it is taken as stuck.
const UPKEEP_TRIES: usize = 24;

/// How long the history worker is watched with no change before it is taken as done with the
/// jobs asked of it: longer than one blame of a generated file takes.
const SETTLED_AFTER: Duration = Duration::from_secs(2);

/// How long the history worker has for what it was asked, and the daemon for the commits' texts
/// and a task's record: the worker waits while hooks are served, and blames one file at a time.
const WARM_WITHIN: Duration = Duration::from_secs(600);

/// How often a condition is looked at while waiting for it.
const POLL_EVERY: Duration = Duration::from_millis(500);

/// The tasks recorded before measuring: a few, as a repository in use holds.
const TASKS: usize = 3;

/// The largest share of `.cortex/` the history tables may take together, at 10,000 files.
const MAX_HISTORY_SHARE: f64 = 0.02;

/// The file the measured edit changes, and the file the recorded tasks change.
const EDITED: (&str, &str) = ("pkg00/m000.py", "pkg02/m003.py");

/// `helper_0_0_1`'s header before the measured edit, and after: the edit adds an argument. The
/// history's commit 100, which fixes an issue, wrote it.
const HEADERS: (&str, &str) = (
    "def helper_0_0_1(value):",
    "def helper_0_0_1(value, scale=1):",
);

/// What the edit's origin note says, whatever the commit.
const ORIGIN: &str = "this edit replaced in pkg00/m000.py came from commit ";

/// The line of `pkg02/m003.py` the recorded tasks change, each to a limit of its own.
const LIMIT_LINE: &str = "LIMIT_2_3 = 3";

/// The repository with the history, the home folder, and the daemon on them. The daemon is
/// declared first so that it is dropped, and so stopped, first: a benchmark that fails while it
/// runs leaves no repository behind that the daemon still held open.
struct HistoryBench {
    /// The daemon, none while it is stopped for the store to be measured.
    daemon: Option<Daemon>,
    /// The repository.
    repo: tempfile::TempDir,
    /// The user's home folder.
    home: tempfile::TempDir,
    /// The number of the next session a sample starts.
    next_session: Cell<u64>,
}

/// What the hook gave the agent to add to its context: empty when it printed nothing or printed
/// no context.
fn context_of(printed: &str) -> String {
    serde_json::from_str::<Value>(printed.trim())
        .ok()
        .and_then(|value| {
            value
                .pointer("/hookSpecificOutput/additionalContext")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Whether a work order carries a why-line: the reason history gives for a line shown.
fn has_why_line(context: &str) -> bool {
    context.lines().any(|line| line.starts_with("  why "))
}

/// Waits until `condition` holds, for at most [`WARM_WITHIN`]; `what` names it in the error.
fn until(what: &str, mut condition: impl FnMut() -> TestResult<bool>) -> TestResult {
    let deadline = Instant::now() + WARM_WITHIN;
    while !condition()? {
        if Instant::now() > deadline {
            return Err(format!("{what}: not within {WARM_WITHIN:?}").into());
        }
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}

impl HistoryBench {
    /// The repository with its history and configuration, a home folder, and a daemon that has
    /// built its first graph and mined the history.
    fn start() -> TestResult<Self> {
        let home = tempfile::tempdir()?;
        let repo = tempfile::tempdir()?;
        write_history(repo.path(), home.path())?;
        std::fs::create_dir(repo.path().join(".cortex"))?;
        std::fs::write(repo.path().join(".cortex/config.toml"), CONFIG)?;
        let daemon = started(repo.path(), home.path());
        let bench = Self {
            repo,
            home,
            daemon: Some(daemon),
            next_session: Cell::new(100),
        };
        until("the history mined", || {
            let git = git_of(bench.repo.path(), bench.home.path())?;
            Ok(git.is_some_and(|git| git.state == cortex_core::ipc::GitState::Ready))
        })?;
        Ok(bench)
    }

    /// The repository's root.
    fn root(&self) -> &Path {
        self.repo.path()
    }

    /// A session id no sample used yet.
    fn session(&self) -> String {
        let number = self.next_session.get();
        self.next_session.set(number + 1);
        format!("00000000-0000-4000-8000-{number:012}")
    }

    /// Runs the hook on `value`, an agent's payload, and returns how long it took and the context
    /// it gave.
    fn send(&self, value: &Value) -> TestResult<(Duration, String)> {
        let event = value
            .get("hook_event_name")
            .and_then(Value::as_str)
            .ok_or("a payload names its event")?;
        let bytes = serde_json::to_vec(value)?;
        let (took, printed) = printed_in(self.root(), self.home.path(), event, &bytes);
        Ok((took, context_of(&printed)))
    }

    /// The recorded payload `fixture`, the prompt [`crate::PROMPT`] for a prompt's, sent as `session`'s.
    fn recorded(&self, fixture: &str, session: &str) -> TestResult<Value> {
        let mut value: Value = serde_json::from_slice(&payload(fixture, self.root()))?;
        if let Some(id) = value.get_mut("session_id") {
            *id = session.into();
        }
        Ok(value)
    }

    /// A new session's start, then its prompt: how long the prompt took and its work order.
    fn new_prompt(&self) -> TestResult<(String, Duration, String)> {
        let session = self.session();
        self.send(&self.recorded("session-start-startup.json", &session)?)?;
        let (took, context) = self.send(&self.recorded("user-prompt-submit.json", &session)?)?;
        Ok((session, took, context))
    }

    /// The daemon's status.
    fn status(&self) -> TestResult<StatusReport> {
        let deadline = Instant::now() + Duration::from_secs(10);
        match cortex_hook::ask(&self.files()?, Role::Cli, RequestBody::Status, deadline)?.body {
            ResponseBody::Status(report) => Ok(report),
            other => Err(format!("the daemon answered its status with {other:?}").into()),
        }
    }

    /// What the history worker holds and waits on.
    fn history(&self) -> TestResult<HistoryReport> {
        Ok(self.status()?.history.ok_or("no history worker runs")?)
    }

    /// The daemon's run files.
    fn files(&self) -> TestResult<RunFiles> {
        let user = UserFolder::in_home(self.home.path());
        Ok(user.run_files(&RepoId::of(&canonical_root(self.root())?)))
    }
}

/// The benchmarks this module describes, and the storage check.
pub(super) fn history(criterion: &mut Criterion) {
    // A benchmark without its warmed repository has nothing to measure.
    #[allow(clippy::expect_used)]
    let mut bench = warmed().expect("the warmed repository with history");
    let mut group = criterion.benchmark_group("hooks_history");
    single_runs(&mut group);
    #[allow(clippy::expect_used)]
    let blames = bench.history().expect("the history's status").cached_blames;
    group.bench_function("user_prompt_submit", |bencher| {
        bencher.iter_custom(|runs| (0..runs).map(|_| measured_prompt(&bench)).sum());
    });
    #[allow(clippy::expect_used)]
    let after = bench.history().expect("the history's status").cached_blames;
    assert_eq!(after, blames, "the measured prompts had spans blamed");
    #[allow(clippy::expect_used)]
    bench.edit_on_disk().expect("the edit made and indexed");
    group.bench_function("post_tool_use_edit", |bencher| {
        bencher.iter_custom(|runs| (0..runs).map(|_| measured_edit(&bench)).sum());
    });
    group.finish();
    #[allow(clippy::expect_used)]
    StopsDaemon::of(bench.root(), bench.home.path())
        .stop()
        .expect("stopping the daemon");
    drop(bench.daemon.take());
}

/// The repository warmed and measured as the module documentation describes, its daemon running
/// again and its premises checked.
fn warmed() -> TestResult<HistoryBench> {
    let mut bench = HistoryBench::start()?;
    bench.read_changed_files()?;
    for _ in 0..WARM_PASSES {
        bench.run_upkeep()?;
        until("the history worker done", || bench.is_settled())?;
    }
    let history = bench.history()?;
    assert!(
        history.cached_blames >= READ_FILES,
        "a day's warming kept {} blames: {history:?}",
        history.cached_blames
    );
    bench.why_lines_given()?;
    bench.record_tasks()?;
    bench.measure_storage()?;
    bench.why_lines_given()?;
    Ok(bench)
}

/// One measured prompt, from a session of its own, checked to carry a why-line.
fn measured_prompt(bench: &HistoryBench) -> Duration {
    // A sample that cannot be run has nothing to measure.
    #[allow(clippy::expect_used)]
    let (_, took, context) = bench.new_prompt().expect("a prompt from a new session");
    assert!(has_why_line(&context), "no why-line in: {context}");
    took
}

/// One measured edit, its task opened by a session's start and prompt, checked to be told its
/// origin.
fn measured_edit(bench: &HistoryBench) -> Duration {
    // A sample that cannot be run has nothing to measure.
    #[allow(clippy::expect_used)]
    let (took, context) = bench.new_edit().expect("an edit from a new session");
    assert!(context.contains(ORIGIN), "no origin note in: {context}");
    took
}

impl HistoryBench {
    /// A session reads [`READ_FILES`] of the files the history changed, so that upkeep warms them
    /// as files a session worked on in the last day.
    fn read_changed_files(&self) -> TestResult {
        let session = self.session();
        self.send(&self.recorded("session-start-startup.json", &session)?)?;
        for index in 0..usize::try_from(READ_FILES)? {
            let Changed { path, .. } = changed(index);
            self.send(&self.read_of(&session, &path)?)?;
        }
        Ok(())
    }

    /// Asks for a run of upkeep until one that ended after the first ask ran through all its jobs,
    /// each left alone for [`UPKEEP_QUIET`] before its status is asked.
    fn run_upkeep(&self) -> TestResult {
        let before = self.status()?.upkeep.map(|upkeep| upkeep.at_ms);
        for _ in 0..UPKEEP_TRIES {
            let deadline = Instant::now() + Duration::from_secs(10);
            let answer = cortex_hook::ask(
                &self.files()?,
                Role::Cli,
                RequestBody::Consolidate,
                deadline,
            )?;
            assert_eq!(answer.body, ResponseBody::Ack, "upkeep was not taken");
            std::thread::sleep(UPKEEP_QUIET);
            let last = self.status()?.upkeep;
            if last.is_some_and(|upkeep| upkeep.is_done && Some(upkeep.at_ms) != before) {
                return Ok(());
            }
        }
        Err(format!("upkeep did not finish in {UPKEEP_TRIES} runs").into())
    }

    /// Whether the history worker has no job waiting and kept nothing new for [`SETTLED_AFTER`].
    fn is_settled(&self) -> TestResult<bool> {
        let before = self.history()?;
        std::thread::sleep(SETTLED_AFTER);
        let after = self.history()?;
        Ok(before.jobs_waiting == 0
            && after.jobs_waiting == 0
            && (before.cached_blames, before.cards) == (after.cached_blames, after.cards))
    }

    /// Waits until a prompt from a new session carries a why-line: the spans its work order shows
    /// are blamed and the commits' texts read, both asked for by the prompts that found them
    /// missing.
    fn why_lines_given(&self) -> TestResult {
        until("a work order with a why-line", || {
            Ok(has_why_line(&self.new_prompt()?.2))
        })
    }

    /// Records [`TASKS`] tasks, each a session's prompt, an edit of [`LIMIT_LINE`] to a limit of its
    /// own, written to disk as an agent writes it, and a stop; and waits for their records.
    fn record_tasks(&self) -> TestResult {
        let path = self.root().join(EDITED.1);
        let mut limit = LIMIT_LINE.to_owned();
        for task in 0..TASKS {
            let (session, _, _) = self.new_prompt()?;
            let before = std::fs::read_to_string(&path)?;
            let next = format!("LIMIT_2_3 = {}", 4 + task);
            std::fs::write(&path, before.replacen(&limit, &next, 1))?;
            let edit = Edit {
                path: EDITED.1,
                before: &before,
                lines: (&limit, &next),
            };
            self.send(&self.edit_of(&session, &edit)?)?;
            self.send(&self.recorded("stop.json", &session)?)?;
            limit = next;
        }
        until("the tasks' records", || {
            let reader = Reader::open(&self.root().join(".cortex"), 0)?;
            Ok(reader.queries().episodes(TASKS)?.len() >= TASKS)
        })
    }

    /// Stops the daemon, measures the history tables' share of `.cortex/`, which must be under
    /// [`MAX_HISTORY_SHARE`], prints it, and starts the daemon again.
    fn measure_storage(&mut self) -> TestResult {
        let history = self.history()?;
        StopsDaemon::of(self.root(), self.home.path()).stop()?;
        drop(self.daemon.take());
        let shares = storage::measure(&self.root().join(".cortex"))?;
        // The shares are a measurement the benchmark reports, as criterion reports its times.
        #[allow(clippy::print_stdout)]
        {
            println!(
                "history storage at 10,000 files, {} blames and {} cards kept: {shares}",
                history.cached_blames, history.cards
            );
        }
        let together = shares.share_of(shares.history_bytes());
        assert!(
            together < MAX_HISTORY_SHARE,
            "the history tables take {:.3}% of .cortex/, not under {:.1}%",
            together * 100.0,
            MAX_HISTORY_SHARE * 100.0
        );
        self.daemon = Some(started(self.root(), self.home.path()));
        Ok(())
    }

    /// Makes the measured edit on disk, waits until the index read it, then until an edit from a
    /// new session is told its origin, the commit's text read again if the restart let it go.
    fn edit_on_disk(&self) -> TestResult {
        let path = self.root().join(EDITED.0);
        let before = std::fs::read_to_string(&path)?;
        let edited = before.replacen(HEADERS.0, HEADERS.1, 1);
        std::fs::write(&path, &edited)?;
        let hash = ContentHash::of(edited.as_bytes());
        until("the edit indexed", || {
            let reader = Reader::open(&self.root().join(".cortex"), 0)?;
            let row = reader.queries().file(&RepoPath::new(EDITED.0)?)?;
            Ok(row.and_then(|row| row.file.content_hash) == Some(hash))
        })?;
        until("an edit told its origin", || {
            Ok(self.new_edit()?.1.contains(ORIGIN))
        })
    }

    /// A new session's start and prompt, then the measured edit of `helper_0_0_1`'s header, the
    /// file before it as HEAD holds it: how long the edit took and what it was told.
    fn new_edit(&self) -> TestResult<(Duration, String)> {
        let (session, _, _) = self.new_prompt()?;
        let path = self.root().join(EDITED.0);
        let edited = std::fs::read_to_string(path)?;
        let before = edited.replacen(HEADERS.1, HEADERS.0, 1);
        let edit = Edit {
            path: EDITED.0,
            before: &before,
            lines: HEADERS,
        };
        self.send(&self.edit_of(&session, &edit)?)
    }
}
