//! Hook events reach the store within a second of arriving, while the daemon loads its grammars and
//! runs its first index too, and while it reads a long transcript, and what reached the store
//! survives a hard kill: a kill loses at most the last second of events.

mod common;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use common::{Daemon, READY_WITHIN, TestResult};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, Event, HookKind, Identifier, RepoId, RepoPath, RunFiles, ToolKind, ToolResult,
    ToolTarget, Ulid, UserFolder, canonical_root,
};
use cortex_hook::ask;
use cortex_store::Reader;

/// How late an event may reach the store.
const WITHIN: Duration = Duration::from_secs(1);

/// Held by each test whose daemon keeps the machine busy for seconds, so that none measures how late
/// events are while another loads the machine as well.
static BUSY: Mutex<()> = Mutex::new(());

/// The machine to itself among the busy tests; a test that failed holding it leaves it free.
fn alone() -> MutexGuard<'static, ()> {
    BUSY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How often the test sends an event, then looks at the store: often enough that a start a fast
/// machine gets through in a second or two still sees twenty events, as the test asks.
const EVERY: Duration = Duration::from_millis(25);

/// The session every event here comes from.
const SESSION: &str = "s1";

/// A package of `modules` Python modules, each importing the one before it and defining
/// `definitions` functions that each call the one before: work for the parser and the resolver in
/// every file, so that a first index takes seconds on any machine without thousands more files to
/// write.
fn package(root: &Path, modules: usize, definitions: usize) -> TestResult {
    let folder = root.join("pkg");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("__init__.py"), "")?;
    for number in 0..modules {
        let mut source = match number.checked_sub(1) {
            Some(previous) => format!("from pkg.m{previous} import f{previous}_0\n\n"),
            None => String::new(),
        };
        source.push_str(&format!("def f{number}_0(value):\n    return value\n\n"));
        for part in 1..definitions {
            let previous = part - 1;
            source.push_str(&format!(
                "def f{number}_{part}(value):\n    return f{number}_{previous}(value) + {part}\n\n"
            ));
        }
        std::fs::write(folder.join(format!("m{number}.py")), source)?;
    }
    Ok(())
}

/// A read of `pkg/m0.py` from the session, after it ran, at `ts_ms`, in the repository at `root`.
fn read_event(root: &Path, ts_ms: i64) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"durability"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new(SESSION)?,
        parent_session_id: None,
        cwd: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        worktree_root: None,
        hook: HookKind::PostTool,
        source: None,
        tool: Some(ToolKind::Read),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new("pkg/m0.py")?),
            ..ToolTarget::default()
        }),
        tool_result: Some(ToolResult {
            exit_code: None,
            is_success: true,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
            failing: None,
        }),
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// The times of the session's events the store holds, none while it cannot be read yet.
fn stored(root: &Path) -> TestResult<BTreeSet<i64>> {
    let Ok(reader) = Reader::open(&root.join(".cortex"), 0) else {
        return Ok(BTreeSet::new());
    };
    Ok(reader
        .queries()
        .events_of_session(&Identifier::new(SESSION)?)?
        .into_iter()
        .map(|row| row.record.ts_ms)
        .collect())
}

/// The events sent and when, and how late the latest look at the store found any.
struct Sender {
    /// The repository.
    root: PathBuf,
    /// Its daemon's run files.
    files: RunFiles,
    /// Each event the daemon took, with when it took it.
    sent: Vec<(Instant, i64)>,
    /// The next event's time.
    next_ts_ms: i64,
    /// When the daemon now running was started, for what a failure says.
    daemon_started: Instant,
}

impl Sender {
    /// A sender to the daemon of the repository at `root`, whose user's home is `home`.
    fn new(root: &Path, home: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            files: UserFolder::in_home(home).run_files(&RepoId::of(root)),
            sent: Vec::new(),
            next_ts_ms: 1_000,
            daemon_started: Instant::now(),
        }
    }

    /// Sends one event; it counts as sent once the daemon takes it.
    fn send(&mut self) -> TestResult {
        let event = read_event(&self.root, self.next_ts_ms)?;
        let deadline = Instant::now() + Duration::from_secs(1);
        let body = RequestBody::Event {
            event: Box::new(event),
        };
        if ask(&self.files, Role::Hook, body, deadline)
            .is_ok_and(|answer| answer.body == ResponseBody::Ack)
        {
            self.sent.push((Instant::now(), self.next_ts_ms));
            self.next_ts_ms += 1;
        }
        Ok(())
    }

    /// The events sent at or before `cutoff` that the store does not hold: each one's time, how
    /// long ago the daemon took it, and how long after its start, in milliseconds.
    fn missing_by(&self, cutoff: Instant) -> TestResult<Vec<(i64, u128, i128)>> {
        let have = stored(&self.root)?;
        let since_start = |taken: Instant| match taken.checked_duration_since(self.daemon_started) {
            Some(after) => i128::try_from(after.as_millis()).unwrap_or(i128::MAX),
            None => -i128::try_from(self.daemon_started.duration_since(taken).as_millis())
                .unwrap_or(i128::MAX),
        };
        Ok(self
            .sent
            .iter()
            .filter(|(taken, ts_ms)| *taken <= cutoff && !have.contains(ts_ms))
            .map(|(taken, ts_ms)| (*ts_ms, taken.elapsed().as_millis(), since_start(*taken)))
            .collect())
    }

    /// Sends an event every [`EVERY`] while `goes_on` holds, at most for `span`, and after each one
    /// checks that every event sent more than [`WITHIN`] before is in the store.
    fn send_while(&mut self, goes_on: impl Fn() -> TestResult<bool>, span: Duration) -> TestResult {
        let until = Instant::now() + span;
        while Instant::now() < until && goes_on()? {
            self.send()?;
            if let Some(cutoff) = Instant::now().checked_sub(WITHIN) {
                let missing = self.missing_by(cutoff)?;
                assert!(
                    missing.is_empty(),
                    "events a second old not stored: {missing:?}"
                );
            }
            std::thread::sleep(EVERY);
        }
        Ok(())
    }
}

/// How many files the store holds, none while it cannot be read yet.
fn files_stored(root: &Path) -> TestResult<u64> {
    let Ok(reader) = Reader::open(&root.join(".cortex"), 0) else {
        return Ok(0);
    };
    Ok(reader.queries().counts()?.files)
}

/// From the moment the daemon listens, through its grammar loading and the first index of 2,000
/// modules of 24 functions each until half the files are stored, every event it took more than a
/// second earlier is in the store; after a hard kill then, every event it took more than a second
/// before the kill is still there. A daemon started again on the half-written store finishes the
/// index, its passes over every file included, and events reach the store within a second through
/// all of it.
#[test]
#[ignore = "timing under load: its premise (every event stored within a second while the first index runs) needs an idle machine; `cargo xtask verify --full` runs it"]
fn events_are_stored_within_a_second_while_the_daemon_starts() -> TestResult {
    let _alone = alone();
    let home = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    package(repo.path(), 2_000, 24)?;
    // The package's modules and its `__init__.py`.
    let (all, half) = (2_001, 1_000);
    let root = canonical_root(repo.path())?;
    let mut daemon = Daemon::start(&root, home.path())?;
    let mut sender = Sender::new(&root, home.path());
    sender.send_while(|| Ok(files_stored(&root)? < half), READY_WITHIN)?;
    let stored_at_kill = files_stored(&root)?;
    assert!(
        (half..all).contains(&stored_at_kill),
        "{stored_at_kill} files stored at the kill"
    );
    assert_eq!(
        daemon.line_by_now(),
        None,
        "the index ended before the kill"
    );
    daemon.child.kill()?;
    daemon.child.wait()?;
    let killed = Instant::now();
    let cutoff = killed
        .checked_sub(WITHIN)
        .ok_or("no time before the kill")?;
    let missing = sender.missing_by(cutoff)?;
    assert!(missing.is_empty(), "events lost to the kill: {missing:?}");
    assert!(sender.sent.len() >= 20, "{} events", sender.sent.len());
    // An event taken in the last second before the kill may be lost with it; the daemon started
    // next owes nothing for it.
    sender.sent.retain(|(taken, _)| *taken <= cutoff);
    let sent_before = sender.sent.len();

    let daemon = Daemon::start(&root, home.path())?;
    sender.daemon_started = Instant::now();
    sender.send_while(|| Ok(daemon.line_by_now().is_none()), READY_WITHIN)?;
    let sent_after = sender.sent.len() - sent_before;
    assert!(sent_after >= 10, "{sent_after} events after the restart");
    let cutoff = Instant::now()
        .checked_sub(WITHIN)
        .ok_or("no time before now")?;
    let missing = sender.missing_by(cutoff)?;
    assert!(
        missing.is_empty(),
        "events a second old not stored: {missing:?}"
    );
    Ok(())
}

/// Once the daemon is ready, events reach the store at once, and a hard kill keeps every event
/// taken more than a second before it.
#[test]
fn events_survive_a_kill_in_steady_state() -> TestResult {
    let home = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    package(repo.path(), 3, 4)?;
    let root = canonical_root(repo.path())?;
    let mut daemon = Daemon::start(&root, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let mut sender = Sender::new(&root, home.path());
    sender.send_while(|| Ok(true), Duration::from_millis(1_500))?;
    daemon.child.kill()?;
    daemon.child.wait()?;
    let cutoff = Instant::now()
        .checked_sub(WITHIN)
        .ok_or("no time before the kill")?;
    let missing = sender.missing_by(cutoff)?;
    assert!(missing.is_empty(), "events lost to the kill: {missing:?}");
    assert!(sender.sent.len() >= 10, "{} events", sender.sent.len());
    Ok(())
}

/// A store a person can no longer read: the database file damaged under the daemon, as a disk
/// fault or a file recovered badly would leave it. Cortex says it cannot read the store and exits,
/// and a daemon started on it does not stay up pretending otherwise. What it must never do is
/// panic, hang, or report a working index over a file it cannot read.
///
/// The damage is written over the middle of the file, past the header SQLite checks first, so that
/// the file still opens and the damage is met while reading pages.
#[test]
fn a_damaged_store_is_said_to_be_damaged() -> TestResult {
    let _busy = alone();
    let home = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    package(repo.path(), 3, 4)?;
    let root = canonical_root(repo.path())?;
    let mut daemon = Daemon::start(&root, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let mut sender = Sender::new(&root, home.path());
    sender.send_while(|| Ok(true), Duration::from_millis(500))?;
    daemon.child.kill()?;
    daemon.child.wait()?;

    let database = root.join(".cortex").join("cortex.db");
    let mut bytes = std::fs::read(&database)?;
    assert!(bytes.len() > 8192, "the store is {} bytes", bytes.len());
    // Past the first page, so the file opens and the damage is met while pages are read.
    let from = bytes.len() / 2;
    for byte in &mut bytes[from..] {
        *byte = b'X';
    }
    std::fs::write(&database, &bytes)?;

    let said = common::init::cortex(&["status"], &root, home.path())?;
    let printed = format!("{}{}", said.stdout, said.stderr);
    assert!(
        !said.is_success || printed.contains("store"),
        "status on a damaged store said: {printed}"
    );
    assert!(
        !printed.contains("index: ready"),
        "a damaged store was called ready: {printed}"
    );
    Ok(())
}

/// How many responses the long transcript holds: 24 MiB of the shortest lines a response can be
/// written on, about 190 bytes each.
const LONG_TRANSCRIPT_RESPONSES: usize = 130_000;

/// Writes at `path` a Claude Code transcript of [`LONG_TRANSCRIPT_RESPONSES`] responses, each on a
/// line holding only what a response is read from: its message id, model, time and counts.
fn long_transcript(path: &Path) -> TestResult {
    let mut text = String::new();
    for number in 0..LONG_TRANSCRIPT_RESPONSES {
        text.push_str(&format!(
            "{{\"timestamp\":\"2026-11-02T23:04:17.241Z\",\"message\":{{\"id\":\"msg_{number:07}\",\
             \"model\":\"claude-sonnet-5-5\",\"usage\":{{\"input_tokens\":1,\
             \"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":1}}}}}}\n"
        ));
    }
    std::fs::create_dir_all(path.parent().ok_or("a transcript has a folder")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// How many usage rows the session's transcript has given, none while the store cannot be read.
fn usage_stored(root: &Path) -> TestResult<usize> {
    let Ok(reader) = Reader::open(&root.join(".cortex"), 0) else {
        return Ok(0);
    };
    Ok(reader
        .queries()
        .usage_of_session(&Identifier::new(SESSION)?)?
        .len())
}

/// While the daemon reads a transcript of 24 MiB of short responses, which it reads in passes,
/// every event it took more than a second earlier is in the store; at the end every response of
/// the transcript has its row.
#[test]
#[ignore = "timing under load: its premise (twenty events sent while the transcript is read) needs an idle machine; `cargo xtask verify --full` runs it"]
fn events_are_stored_within_a_second_while_a_long_transcript_is_read() -> TestResult {
    let _alone = alone();
    let home = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    package(repo.path(), 1, 1)?;
    let root = canonical_root(repo.path())?;
    let transcript = home
        .path()
        .join(".claude")
        .join("projects")
        .join("C--repo")
        .join(format!("{SESSION}.jsonl"));
    long_transcript(&transcript)?;
    let daemon = Daemon::start(&root, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let mut sender = Sender::new(&root, home.path());
    let stop = Event {
        hook: HookKind::Stop,
        tool: None,
        tool_target: None,
        tool_result: None,
        transcript_path: Some(transcript),
        ..read_event(&root, 500)?
    };
    let body = RequestBody::Event {
        event: Box::new(stop),
    };
    let answer = ask(&sender.files, Role::Hook, body, Instant::now() + WITHIN)?;
    assert_eq!(answer.body, ResponseBody::Ack);
    // Counting the rows reads them all, so it is done every half second rather than after each
    // event, which would leave few events sent while the transcript is read.
    let counted = Cell::new((Instant::now(), 0));
    let is_reading = || -> TestResult<bool> {
        let (at, rows) = counted.get();
        if at.elapsed() < Duration::from_millis(500) {
            return Ok(rows < LONG_TRANSCRIPT_RESPONSES);
        }
        let rows = usage_stored(&root)?;
        counted.set((Instant::now(), rows));
        Ok(rows < LONG_TRANSCRIPT_RESPONSES)
    };
    sender.send_while(is_reading, Duration::from_secs(120))?;
    assert_eq!(usage_stored(&root)?, LONG_TRANSCRIPT_RESPONSES);
    assert!(sender.sent.len() >= 20, "{} events", sender.sent.len());
    Ok(())
}
