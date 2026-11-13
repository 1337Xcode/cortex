//! What waits for the writer: hook events, the watcher's changes, a mining of the history, what
//! the history worker found, word that the watcher stopped, and a request to stop, with the counts, the miner's state and the
//! latest errors the daemon's status reports, and how many of the watcher's changes the writer has
//! indexed, which the miner waits on. One lock and one condition variable, so the writer sleeps
//! until something arrives or a deadline passes. It also holds the rules the writer keeps each
//! session's positions by, set once the configuration is read.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use cortex_coord::Timing;
use cortex_core::ipc::{GitReport, RecentError, UpkeepReport};
use cortex_core::{
    CheckCounts, CoordinationConfig, Event, HookKind, RepoPath, ShortHash, ToolKind,
};
use cortex_git::mine::Mining;
use cortex_index::Change;
use cortex_learn::{Commands, failing_signature, read_line_failure};
use cortex_store::CheckpointFailures;

use crate::asked::{Ask, AskJob};
use crate::given::Given;
use crate::history::{HistoryWrite, HistoryWriteJob};

use crate::recent_errors::RecentErrors;
use crate::troubles::{Step, Taken, Troubles};

/// How many hook events may wait for the writer. When the inbox is full, a read or a search gives
/// way first: an arriving one is dropped, or the oldest waiting one makes room for an edit, a
/// prompt or a session's event. Only when every waiting event is of those does an arriving one go.
/// Every drop is counted.
pub(crate) const MAX_EVENTS: usize = 10_000;

/// How many records of what was given may wait for the writer, each a primer's or a prompt's
/// injection or an edit's warnings: when full, the oldest gives way, counted. A writer that records
/// events keeps up with these.
pub(crate) const MAX_GIVEN: usize = 1_000;

/// How many changes may wait for the writer. A change that finds the inbox full is dropped and an
/// overflow marked, so that the changes drained next are indexed as a change to everything, the
/// dropped change's files included.
pub(crate) const MAX_CHANGES: usize = 10_000;

/// Most requests of people and agents waiting for the writer (notes, the user's word on facts):
/// people type them, so a handful at once is already many, and one more is refused rather than
/// kept.
const MAX_ASKS_WAITING: usize = 16;

/// The writer's inbox.
#[derive(Debug, Default)]
pub(crate) struct Inbox {
    /// What waits.
    state: Mutex<State>,
    /// Signalled whenever something is put in.
    arrived: Condvar,
    /// How long each kind of position holds, from the configuration: of a session's two positions
    /// on one file, the writer keeps the one that holds longer.
    position_timing: OnceLock<Timing>,
    /// The checks Cortex tells apart, from the configuration, to read a failed check's counts.
    commands: OnceLock<Arc<Commands>>,
}

/// What waits in the inbox.
#[derive(Debug, Default)]
struct State {
    /// Hook events, oldest first.
    events: VecDeque<Event>,
    /// Hook events dropped since the daemon started.
    events_dropped: u64,
    /// What was given to agents, or would have been, oldest first, each recorded after the events
    /// that came before it.
    given: VecDeque<Given>,
    /// Batches of events the writer could not record, since the daemon started.
    failed_batches: u64,
    /// The writer's count of the checkpoints that failed.
    failed_checkpoints: CheckpointFailures,
    /// The latest errors the writer and the connections met.
    errors: RecentErrors,
    /// The steps the daemon runs on its own that failed since the user was last told.
    troubles: Troubles,
    /// The watcher's changes, oldest first, each with when it arrived, in UTC milliseconds.
    changes: Vec<(Change, i64)>,
    /// Whether a change was dropped since the last drain.
    is_overflowed: bool,
    /// How many changes the watcher put in since the daemon started, the dropped ones included.
    changes_in: u64,
    /// How many of those the writer had taken when it last drained the changes.
    changes_taken: u64,
    /// How many of those the writer has indexed: the ones it had taken before it last came back
    /// for work.
    changes_indexed: u64,
    /// How many edits hooks reported since the daemon started, failed ones included, and shell
    /// commands that may have written a file: changes the daemon hears of before the watcher
    /// reports them, or that the watcher, which reports a path once it has been quiet a moment,
    /// has not reported yet.
    edits_in: u64,
    /// A mining waiting to be written: one at most, since the miner waits for the write.
    mining: Option<MiningJob>,
    /// What the history worker found, waiting to be written: one at most, since the worker waits
    /// for the write.
    history: Option<HistoryWriteJob>,
    /// Requests waiting for the writer, oldest first, at most [`MAX_ASKS_WAITING`].
    asked: VecDeque<AskJob>,
    /// Whether a person asked for upkeep and the writer has not been told yet.
    is_upkeep_asked: bool,
    /// What the last run of upkeep did.
    last_upkeep: Option<UpkeepReport>,
    /// What the co-change miner is doing.
    git: GitReport,
    /// Whether the daemon was asked to stop.
    is_stopping: bool,
    /// Whether the writer has stopped working and takes no more events.
    is_closed: bool,
    /// Whether the watcher stopped feeding the inbox.
    is_watcher_gone: bool,
    /// When the latest request of any kind arrived, if one did.
    last_request: Option<Instant>,
    /// Whether the daemon's first index is done.
    is_indexed: bool,
    /// Whether huge mode's crawl has files waiting to be parsed.
    is_crawling: bool,
    /// How many batches of hook events the writer has recorded since the daemon started: the map
    /// reads where agents are again when it changes.
    batches_recorded: u64,
}

/// What the writer is to do next.
#[derive(Debug)]
pub(crate) enum Work {
    /// Stop.
    Stop,
    /// Record these events, oldest first.
    Events(Vec<Event>),
    /// Record what was given, oldest first: the events before it are recorded.
    Given(Vec<Given>),
    /// Index what changed.
    Changed(Changed),
    /// Write what the miner found, and tell it how that went.
    Mining(Box<MiningJob>),
    /// Write what the history worker found, and tell it how that went.
    History(Box<HistoryWriteJob>),
    /// Answer a request of a person or an agent.
    Ask(Box<AskJob>),
    /// Run upkeep: a person asked for it.
    Upkeep,
    /// The watcher stopped: changes are no longer seen.
    WatcherStopped,
    /// Nothing arrived before the deadline.
    Timeout,
}

/// What every change waiting together changed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Changed {
    /// Everything is to be indexed; the paths named besides, in path order, are asked for first in
    /// huge mode.
    Everything(Vec<RepoPath>),
    /// Only these paths, in path order, each with when the first change naming it arrived, in UTC
    /// milliseconds: when the watcher saw it change.
    Paths(Vec<(RepoPath, i64)>),
}

/// A mining of the history for the writer to store, with where to say how that went.
#[derive(Debug)]
pub(crate) struct MiningJob {
    /// What was mined.
    pub(crate) mining: Mining,
    /// Where the writer says whether it was stored, or why not.
    written: SyncSender<Result<(), String>>,
}

impl MiningJob {
    /// Tells the miner whether the mining was stored, or why not. A miner that stopped waiting is
    /// told nothing.
    pub(crate) fn answer(self, written: Result<(), String>) {
        drop(self.written.send(written));
    }
}

/// The inbox's counts, for the daemon's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Hook events waiting.
    pub(crate) events_waiting: usize,
    /// Changes waiting.
    pub(crate) changes_waiting: usize,
    /// Hook events dropped since the daemon started.
    pub(crate) events_dropped: u64,
    /// Batches of events the writer could not record, since the daemon started.
    pub(crate) failed_batches: u64,
    /// Checkpoints of the store's log that failed, since the writer opened.
    pub(crate) failed_checkpoints: u64,
    /// Whether the daemon's first index is done.
    pub(crate) is_indexed: bool,
}

impl Inbox {
    /// Sets how long each kind of position holds, as the configuration says: once, before the
    /// writer opens, so every event it records keeps its positions by the configured holds. A
    /// second call changes nothing.
    pub(crate) fn keep_positions_by(&self, timing: Timing) {
        self.position_timing.get_or_init(|| timing);
    }

    /// Reads a failed check's output with `commands`, the checks the configuration tells apart,
    /// for the counts the writer keeps: set before the writer opens. A second call changes nothing;
    /// an inbox never set reads no counts.
    pub(crate) fn read_failures_with(&self, commands: Arc<Commands>) {
        self.commands.get_or_init(|| commands);
    }

    /// The counts `output`, what `event`'s failed shell line printed, says, and the signature of
    /// the failing tests it names (`failing_signature`): none when the inbox reads no failures, or
    /// no parser of the line's checks finds a failure in it.
    fn failure_of(&self, event: &Event, output: &str) -> (Option<CheckCounts>, Option<ShortHash>) {
        let read = self.commands.get().and_then(|commands| {
            let command = event.tool_target.as_ref()?.command.as_ref()?;
            read_line_failure(
                commands,
                command,
                (event.worktree_top(), &event.cwd),
                output,
            )
        });
        read.map_or((None, None), |read| {
            (read.failures.counts, failing_signature(&read.failures))
        })
    }

    /// Whether `event` is a tool's result that may have changed the repository's files: an edit's,
    /// failed or not, or a shell command's unless its line ran only checks, `cd` and programs
    /// known to write nothing (`Told::may_write`). A shell line the inbox cannot read, as when it
    /// reads no commands or the event names none, may have.
    fn may_have_changed_files(&self, event: &Event) -> bool {
        if event.hook != HookKind::PostTool {
            return false;
        }
        match event.tool {
            Some(tool) if tool.is_edit() => true,
            Some(ToolKind::Shell) => {
                let command = event
                    .tool_target
                    .as_ref()
                    .and_then(|target| target.command.as_ref());
                command
                    .zip(self.commands.get())
                    .is_none_or(|(command, commands)| {
                        commands
                            .tell(command, &event.repo_root, &event.cwd)
                            .may_write
                    })
            }
            _ => false,
        }
    }

    /// How long each kind of position holds: as set from the configuration, which the daemon
    /// does before its writer opens; the documented defaults for an inbox never set, as a test's.
    pub(crate) fn position_timing(&self) -> Timing {
        self.position_timing
            .get()
            .copied()
            .unwrap_or_else(|| Timing::of(&CoordinationConfig::default()))
    }

    /// Puts in a hook event, making room as [`MAX_EVENTS`] says when the inbox is full, and says
    /// whether it was taken: an event dropped to make room is taken (and counted), but none is
    /// once the inbox is closed, since the writer records what waits when it closes it and nothing
    /// that comes later. A stop asked for does not close it: the writer may still record events
    /// before it next looks. A failed command's output, which only an answer reads, is dropped from
    /// the event first.
    pub(crate) fn push_event(&self, mut event: Event) -> bool {
        // INVARIANT: no-content-stored (the writer, which stores events, never gets a command's
        // output, only the counts read from it and the hash of the failing tests' names; and a
        // queue of events stays one of paths, counts and hashes)
        if let Some(output) = event
            .tool_target
            .as_mut()
            .and_then(|target| target.failed_output.take())
        {
            let (counts, failing) = self.failure_of(&event, output.as_str());
            if let Some(result) = event.tool_result.as_mut() {
                result.counts = counts;
                result.failing = failing;
            }
        } else if let Some(result) = event.tool_result.as_mut() {
            // Only the daemon's own reading of an output tells two failures apart.
            result.failing = None;
        }
        let is_change = self.may_have_changed_files(&event);
        let mut state = self.lock();
        // Counted before anything can turn the event away: the file changed all the same.
        if is_change {
            state.edits_in = state.edits_in.saturating_add(1);
        }
        if state.is_closed {
            return false;
        }
        if state.events.len() >= MAX_EVENTS {
            state.events_dropped += 1;
            // A hook event lost is storing that failed: the user is told at the next start.
            state.troubles.note(
                Step::Storing,
                "too many hook events waited to be recorded, and one was dropped",
                event.ts_ms,
            );
            if is_droppable(&event) {
                return true;
            }
            match state.events.iter().position(is_droppable) {
                Some(oldest) => drop(state.events.remove(oldest)),
                None => return true,
            }
        }
        state.events.push_back(event);
        drop(state);
        self.arrived.notify_one();
        true
    }

    /// Puts `given` in to be recorded after the events waiting; `false` once the writer has
    /// stopped. When [`MAX_GIVEN`] wait, the oldest gives way, kept among the latest errors with
    /// `at_ms`, the time in UTC milliseconds.
    pub(crate) fn push_given(&self, given: Given, at_ms: i64) -> bool {
        let mut state = self.lock();
        if state.is_closed {
            return false;
        }
        if state.given.len() >= MAX_GIVEN
            && let Some(oldest) = state.given.pop_front()
        {
            state.errors.note(oldest.dropped_message(), at_ms);
            state
                .troubles
                .note(Step::Storing, oldest.dropped_message(), at_ms);
            tracing::warn!("too many records of what was given waiting: the oldest dropped");
        }
        state.given.push_back(given);
        drop(state);
        self.arrived.notify_one();
        true
    }

    /// Takes every event and every record of what was given waiting, together, without waiting
    /// for either: what the writer records between the transactions of an index, and whenever it is
    /// between tasks. What was given comes in after its event, so the events taken with it hold its
    /// event, or it was taken before.
    pub(crate) fn take_waiting(&self) -> (Vec<Event>, Vec<Given>) {
        let mut state = self.lock();
        let events = state.events.drain(..).collect();
        let given = state.given.drain(..).collect();
        (events, given)
    }

    /// Counts a batch the writer could not record, and keeps why, `message`, as an error met at
    /// `at_ms` in UTC milliseconds.
    pub(crate) fn count_failed_batch(&self, message: &str, at_ms: i64) {
        let mut state = self.lock();
        state.failed_batches += 1;
        state.errors.note(message, at_ms);
        state.troubles.note(Step::Storing, message, at_ms);
    }

    /// Keeps `message` as an error met at `at_ms` in UTC milliseconds, and counts it as a
    /// failure of `step`, which the next session's start tells the user of.
    pub(crate) fn note_trouble(&self, step: Step, message: &str, at_ms: i64) {
        let mut state = self.lock();
        state.errors.note(message, at_ms);
        state.troubles.note(step, message, at_ms);
    }

    /// What the user is told at `now_ms` of the steps that failed since last told, with what was
    /// taken to say it, and from then on nothing of those failures unless they are put back;
    /// none when none failed.
    pub(crate) fn take_troubles(&self, now_ms: i64) -> Option<(String, Taken)> {
        self.lock().troubles.take_said(now_ms)
    }

    /// Puts back the failures `taken`, whose telling never reached the user.
    pub(crate) fn put_back_troubles(&self, taken: Taken) {
        self.lock().troubles.put_back(taken);
    }

    /// Keeps `message` as an error met at `at_ms` in UTC milliseconds.
    pub(crate) fn note_error(&self, message: &str, at_ms: i64) {
        self.lock().errors.note(message, at_ms);
    }

    /// The latest errors, oldest first.
    pub(crate) fn recent_errors(&self) -> Vec<RecentError> {
        self.lock().errors.list()
    }

    /// Reports the checkpoints `failures` counts from now on: the writer's, once it is open.
    pub(crate) fn watch_checkpoints(&self, failures: CheckpointFailures) {
        self.lock().failed_checkpoints = failures;
    }

    /// What waits, and what was lost.
    pub(crate) fn counts(&self) -> Counts {
        let state = self.lock();
        Counts {
            events_waiting: state.events.len(),
            changes_waiting: state.changes.len(),
            events_dropped: state.events_dropped,
            failed_batches: state.failed_batches,
            failed_checkpoints: state.failed_checkpoints.count(),
            is_indexed: state.is_indexed,
        }
    }

    /// Notes that the daemon's first index is done, for its status to say.
    pub(crate) fn mark_indexed(&self) {
        self.lock().is_indexed = true;
    }

    /// Puts in a change from the watcher that arrived at `seen_ms`, in UTC milliseconds, or drops it
    /// and marks the overflow when the inbox holds [`MAX_CHANGES`].
    pub(crate) fn push_change(&self, change: Change, seen_ms: i64) {
        let mut state = self.lock();
        state.changes_in = state.changes_in.saturating_add(1);
        if state.changes.len() >= MAX_CHANGES {
            state.is_overflowed = true;
        } else {
            state.changes.push((change, seen_ms));
        }
        drop(state);
        self.arrived.notify_one();
    }

    /// Notes that a request arrived now: a command's status, a hook's event or anything else is
    /// use, and puts off the idle stop. The writer is not woken: it looks when its wait ends.
    pub(crate) fn note_request(&self) {
        self.lock().last_request = Some(Instant::now());
    }

    /// When the latest request arrived, if one did.
    pub(crate) fn last_request(&self) -> Option<Instant> {
        self.lock().last_request
    }

    /// Notes that the writer recorded a batch of hook events, which may have moved a session.
    pub(crate) fn note_recorded(&self) {
        let mut state = self.lock();
        state.batches_recorded = state.batches_recorded.wrapping_add(1);
    }

    /// How many batches of hook events the writer has recorded since the daemon started.
    pub(crate) fn batches_recorded(&self) -> u64 {
        self.lock().batches_recorded
    }

    /// Tells the writer a person asked for upkeep, to run as soon as it is free.
    pub(crate) fn ask_upkeep(&self) {
        self.lock().is_upkeep_asked = true;
        self.arrived.notify_one();
    }

    /// Keeps what a run of upkeep did, for the daemon's status.
    pub(crate) fn note_upkeep(&self, report: UpkeepReport) {
        self.lock().last_upkeep = Some(report);
    }

    /// What the last run of upkeep did, if one ran.
    pub(crate) fn upkeep(&self) -> Option<UpkeepReport> {
        self.lock().last_upkeep
    }

    /// Whether upkeep, which began when the latest request was `asked_before`, should stop for
    /// other work: work waits for the writer, or a request arrived since, a read's hint among them,
    /// which a reader answers beside the writer and upkeep should not slow.
    pub(crate) fn should_upkeep_stop(&self, asked_before: Option<Instant>) -> bool {
        self.has_work_waiting() || self.last_request() != asked_before
    }

    /// Whether work the writer should do before upkeep waits: hook events to record, what was
    /// given to record, a request asked of it, or a stop.
    pub(crate) fn has_work_waiting(&self) -> bool {
        let state = self.lock();
        state.is_stopping
            || !state.events.is_empty()
            || !state.given.is_empty()
            || !state.asked.is_empty()
    }

    /// Puts in a mining for the writer, and returns where the writer will say whether it stored
    /// it; `None` once the inbox is closed, when nothing would store it.
    pub(crate) fn push_mining(&self, mining: Mining) -> Option<Receiver<Result<(), String>>> {
        let mut state = self.lock();
        if state.is_closed {
            return None;
        }
        let (written, answer) = mpsc::sync_channel(1);
        state.mining = Some(MiningJob { mining, written });
        drop(state);
        self.arrived.notify_one();
        Some(answer)
    }

    /// Puts in what the history worker found for the writer, and returns where the writer will
    /// say whether it stored it; `None` once the inbox is closed, when nothing would store it.
    pub(crate) fn push_history(&self, write: HistoryWrite) -> Option<Receiver<Result<(), String>>> {
        let mut state = self.lock();
        if state.is_closed {
            return None;
        }
        let (job, answer) = HistoryWriteJob::new(write);
        state.history = Some(job);
        drop(state);
        self.arrived.notify_one();
        Some(answer)
    }

    /// Puts in a request for the writer, and returns where the writer will answer it; none when
    /// the inbox is closed or [`MAX_ASKS_WAITING`] requests wait already.
    pub(crate) fn push_ask(&self, request: Ask) -> Option<Receiver<crate::asked::Answered>> {
        let mut state = self.lock();
        if state.is_closed || state.asked.len() >= MAX_ASKS_WAITING {
            return None;
        }
        let (job, answer) = AskJob::new(request);
        state.asked.push_back(job);
        drop(state);
        self.arrived.notify_one();
        Some(answer)
    }

    /// How many changes the watcher put in since the daemon started.
    pub(crate) fn changes_in(&self) -> u64 {
        self.lock().changes_in
    }

    /// How many changes to the repository's files the daemon has heard of since it started: the
    /// watcher's and the edits hooks reported. None once the watcher stopped, when a change could
    /// go unheard.
    pub(crate) fn source_changes(&self) -> Option<u64> {
        let state = self.lock();
        (!state.is_watcher_gone).then(|| state.changes_in.saturating_add(state.edits_in))
    }

    /// Whether an index is under way or due: changes wait for the writer, it took some and has
    /// not come back for more work since, which it does once they are indexed, or huge mode's
    /// crawl has files waiting to be parsed.
    pub(crate) fn is_indexing(&self) -> bool {
        let state = self.lock();
        !state.changes.is_empty()
            || state.is_overflowed
            || state.changes_taken > state.changes_indexed
            || state.is_crawling
    }

    /// Notes whether huge mode's crawl has files waiting to be parsed: the writer says so as it
    /// goes.
    pub(crate) fn set_crawling(&self, is_crawling: bool) {
        self.lock().is_crawling = is_crawling;
    }

    /// Whether the writer has indexed the first `count` changes the watcher put in: it took them,
    /// then came back for more work.
    pub(crate) fn has_indexed(&self, count: u64) -> bool {
        self.lock().changes_indexed >= count
    }

    /// Reports what the co-change miner is doing, for the daemon's status.
    pub(crate) fn set_git(&self, report: GitReport) {
        self.lock().git = report;
    }

    /// What the co-change miner is doing.
    pub(crate) fn git(&self) -> GitReport {
        self.lock().git.clone()
    }

    /// Asks the writer to stop.
    pub(crate) fn stop(&self) {
        self.lock().is_stopping = true;
        self.arrived.notify_one();
    }

    /// Closes the inbox to events and minings: what the writer does once it has stopped working,
    /// whatever stopped it, before it records the last events waiting. A mining or the history
    /// worker's findings still waiting are dropped unwritten, which ends the miner's and the
    /// worker's wait for them; the next daemon reads them again.
    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.is_closed = true;
        state.mining = None;
        state.history = None;
        state.asked.clear();
    }

    /// Says the watcher stopped feeding the inbox.
    pub(crate) fn watcher_stopped(&self) {
        self.lock().is_watcher_gone = true;
        self.arrived.notify_one();
    }

    /// Waits until an event, a change, a mining, the history worker's findings, a stop or word of
    /// the watcher arrives, or `until` passes (never, when `None`), and says what to do: a stop
    /// first, then every event waiting, then every change waiting, drained into one, then the
    /// oldest request asked, so that a note finds the files changed before it indexed, then the
    /// mining, then the history worker's findings, then that the watcher stopped.
    /// Only the writer waits, so the changes it took before are indexed by now.
    pub(crate) fn wait(&self, until: Option<Instant>) -> Work {
        let mut state = self.lock();
        state.changes_indexed = state.changes_taken;
        loop {
            if state.is_stopping {
                return Work::Stop;
            }
            if !state.events.is_empty() {
                return Work::Events(state.events.drain(..).collect());
            }
            if !state.given.is_empty() {
                return Work::Given(state.given.drain(..).collect());
            }
            if !state.changes.is_empty() || state.is_overflowed {
                return Work::Changed(drain(&mut state));
            }
            if let Some(job) = state.asked.pop_front() {
                return Work::Ask(Box::new(job));
            }
            if state.is_upkeep_asked {
                state.is_upkeep_asked = false;
                return Work::Upkeep;
            }
            if let Some(job) = state.mining.take() {
                return Work::Mining(Box::new(job));
            }
            if let Some(job) = state.history.take() {
                return Work::History(Box::new(job));
            }
            if state.is_watcher_gone {
                return Work::WatcherStopped;
            }
            state = match until {
                None => self
                    .arrived
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(until) => {
                    let Some(left) = until.checked_duration_since(Instant::now()) else {
                        return Work::Timeout;
                    };
                    self.arrived
                        .wait_timeout(state, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
    }

    /// The state, locked. A panic while it was held leaves it consistent (each change is one push),
    /// so a poisoned lock is taken as it is.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether an event may give way when the inbox is full: a read or a search, recorded after it
/// ran.
fn is_droppable(event: &Event) -> bool {
    event.hook == HookKind::PostTool
        && matches!(
            event.tool,
            Some(ToolKind::Read | ToolKind::Grep | ToolKind::Glob)
        )
}

/// Every change waiting, as one: everything when any of them is, or when one was dropped, else
/// the paths they name, each with when the first change naming it arrived. The overflow mark is
/// cleared with the changes.
fn drain(state: &mut State) -> Changed {
    state.changes_taken = state.changes_in;
    let mut paths: BTreeMap<RepoPath, i64> = BTreeMap::new();
    let mut is_everything = std::mem::take(&mut state.is_overflowed);
    for (change, seen_ms) in state.changes.drain(..) {
        match change {
            Change::Paths(named) => {
                for path in named {
                    let first_ms = paths.entry(path).or_insert(seen_ms);
                    *first_ms = (*first_ms).min(seen_ms);
                }
            }
            Change::Everything => is_everything = true,
        }
    }
    if is_everything {
        Changed::Everything(paths.into_keys().collect())
    } else {
        Changed::Paths(paths.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A change naming `names`.
    fn paths(names: &[&str]) -> Result<Change, cortex_core::PathError> {
        Ok(Change::Paths(
            names
                .iter()
                .map(|name| RepoPath::new(name))
                .collect::<Result<_, _>>()?,
        ))
    }

    /// Upkeep stops for a request that came after it began, a read's hint included, and for work
    /// waiting for the writer; not for the request it began after.
    #[test]
    fn upkeep_stops_for_a_request_that_came_after_it_began() {
        let inbox = Inbox::default();
        let asked_before = inbox.last_request();
        assert!(!inbox.should_upkeep_stop(asked_before));
        inbox.note_request();
        assert!(inbox.should_upkeep_stop(asked_before));
        assert!(!inbox.should_upkeep_stop(inbox.last_request()));
    }

    /// The paths of `changed`, and whether it is a change to everything.
    fn summary(work: Work) -> Option<(bool, Vec<String>)> {
        let Work::Changed(changed) = work else {
            return None;
        };
        Some(match changed {
            Changed::Everything(paths) => (true, paths.iter().map(ToString::to_string).collect()),
            Changed::Paths(paths) => (
                false,
                paths.iter().map(|(path, _)| path.to_string()).collect(),
            ),
        })
    }

    /// Changes waiting together are drained into one, in path order, each path seen when the first
    /// change naming it arrived: `b.py` at 20, though named again at 30, and `c.py` at 40 whatever
    /// the others' times. One change to everything makes the whole batch one; a dropped change,
    /// marked as an overflow, does too, once.
    #[test]
    fn waiting_changes_drain_into_one() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        inbox.push_change(paths(&["b.py"])?, 30);
        inbox.push_change(paths(&["a.py", "b.py"])?, 20);
        inbox.push_change(paths(&["c.py"])?, 40);
        let names = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();
        let drained = match inbox.wait(None) {
            Work::Changed(Changed::Paths(drained)) => drained,
            other => return Err(format!("drained {other:?}").into()),
        };
        let times: Vec<(String, i64)> = drained
            .iter()
            .map(|(path, seen_ms)| (path.to_string(), *seen_ms))
            .collect();
        assert_eq!(
            times,
            [
                ("a.py".to_owned(), 20),
                ("b.py".to_owned(), 20),
                ("c.py".to_owned(), 40)
            ]
        );
        inbox.push_change(paths(&["c.py"])?, 1);
        inbox.push_change(Change::Everything, 1);
        assert_eq!(summary(inbox.wait(None)), Some((true, names(&["c.py"]))));
        for _ in 0..MAX_CHANGES {
            inbox.push_change(paths(&["d.py"])?, 1);
        }
        inbox.push_change(paths(&["e.py"])?, 1);
        assert_eq!(summary(inbox.wait(None)), Some((true, names(&["d.py"]))));
        inbox.push_change(paths(&["f.py"])?, 1);
        assert_eq!(summary(inbox.wait(None)), Some((false, names(&["f.py"]))));
        Ok(())
    }

    /// With nothing waiting, a deadline in the past times out at once and one in the future after
    /// waiting for it; changes come before word that the watcher stopped, and a stop before any
    /// change, every time it is asked.
    #[test]
    fn a_stop_comes_first_and_deadlines_time_out() -> Result<(), cortex_core::PathError> {
        let inbox = Inbox::default();
        assert!(matches!(inbox.wait(Some(Instant::now())), Work::Timeout));
        let start = Instant::now();
        assert!(matches!(
            inbox.wait(Some(start + Duration::from_millis(50))),
            Work::Timeout
        ));
        assert!(start.elapsed() >= Duration::from_millis(50));
        inbox.push_change(paths(&["w.py"])?, 1);
        inbox.watcher_stopped();
        assert!(matches!(inbox.wait(None), Work::Changed(_)));
        assert!(matches!(inbox.wait(None), Work::WatcherStopped));
        inbox.push_change(paths(&["a.py"])?, 1);
        inbox.stop();
        assert!(matches!(inbox.wait(None), Work::Stop));
        assert!(matches!(inbox.wait(None), Work::Stop));
        Ok(())
    }

    /// An event of `hook`, with `tool` for a tool event, at `ts_ms`.
    fn event(
        hook: HookKind,
        tool: Option<ToolKind>,
        ts_ms: i64,
    ) -> Result<Event, Box<dyn std::error::Error>> {
        Ok(Event {
            event_id: cortex_core::Ulid::from_seed(u64::try_from(ts_ms)?, b"inbox"),
            ts_ms,
            agent: cortex_core::Agent::Claude,
            agent_version: None,
            session_id: cortex_core::Identifier::new("s1")?,
            parent_session_id: None,
            cwd: std::path::PathBuf::from("/repo"),
            repo_root: std::path::PathBuf::from("/repo"),
            worktree_root: None,
            hook,
            source: None,
            tool,
            tool_target: None,
            tool_result: None,
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        })
    }

    /// The source changes heard of count each change the watcher put in, each edit a hook
    /// reported (a failed one and one the closed inbox turns away included) and each shell line
    /// that may have written a file (`sed -i`, a redirection to a file, one that names no command),
    /// and nothing else: a check, a line of programs that only read, a read, an edit before it
    /// runs. Once the watcher stops, the count is unknown.
    #[test]
    fn the_source_changes_heard_of_are_counted() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        inbox.read_failures_with(Arc::new(Commands::new(&[])?));
        assert_eq!(inbox.source_changes(), Some(0));
        inbox.push_change(paths(&["a.py"])?, 1);
        inbox.push_change(Change::Everything, 2);
        assert_eq!(inbox.source_changes(), Some(2));
        inbox.push_event(event(HookKind::PostTool, Some(ToolKind::Edit), 3)?);
        let failed_patch = Event {
            tool_result: Some(cortex_core::ToolResult {
                exit_code: None,
                is_success: false,
                is_interrupted: false,
                output_bytes: None,
                counts: None,
                failing: None,
            }),
            ..event(HookKind::PostTool, Some(ToolKind::ApplyPatch), 4)?
        };
        inbox.push_event(failed_patch);
        assert_eq!(inbox.source_changes(), Some(4));
        let shell = |line: &str, ts_ms| -> Result<Event, Box<dyn std::error::Error>> {
            Ok(Event {
                tool_target: Some(cortex_core::ToolTarget {
                    command: Some(cortex_core::ShellCommand::cut(line)),
                    ..cortex_core::ToolTarget::default()
                }),
                ..event(HookKind::PostTool, Some(ToolKind::Shell), ts_ms)?
            })
        };
        inbox.push_event(shell("python -m pytest -q 2>&1 | tail -20", 5)?);
        inbox.push_event(shell("git status && cat calc/ops.py", 6)?);
        inbox.push_event(event(HookKind::PostTool, Some(ToolKind::Read), 7)?);
        inbox.push_event(event(HookKind::PreTool, Some(ToolKind::Write), 8)?);
        inbox.push_event(Event {
            hook: HookKind::PreTool,
            ..shell("sed -i s/a/b/ calc/ops.py", 9)?
        });
        assert_eq!(inbox.source_changes(), Some(4));
        inbox.push_event(shell("sed -i s/a/b/ calc/ops.py", 10)?);
        inbox.push_event(shell("pytest -q > out.txt", 11)?);
        inbox.push_event(event(HookKind::PostTool, Some(ToolKind::Shell), 12)?);
        assert_eq!(inbox.source_changes(), Some(7));
        inbox.close();
        assert!(!inbox.push_event(event(HookKind::PostTool, Some(ToolKind::Write), 13)?));
        assert_eq!(inbox.source_changes(), Some(8));
        inbox.watcher_stopped();
        assert_eq!(inbox.source_changes(), None);
        Ok(())
    }

    /// The times of the events a wait drains.
    fn times(work: Work) -> Vec<i64> {
        match work {
            Work::Events(events) => events.iter().map(|event| event.ts_ms).collect(),
            _ => Vec::new(),
        }
    }

    /// A failed command's event reaches the writer without its output, its command kept and the
    /// counts its output printed read: pytest's `1 failed, 2 passed`, with the short hash of the
    /// one failing test's name. A signature an event brings without an output is not kept.
    #[test]
    fn the_writer_gets_no_command_s_output() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        inbox.read_failures_with(Arc::new(Commands::new(&[])?));
        let mut failed = event(HookKind::PostTool, Some(ToolKind::Shell), 1)?;
        failed.tool_target = Some(cortex_core::ToolTarget {
            command: Some(cortex_core::ShellCommand::cut("pytest")),
            failed_output: cortex_core::FailedOutput::tail_of(
                "FAILED test_ops.py::test_a - AssertionError\n1 failed, 2 passed in 0.01s\n",
            ),
            ..cortex_core::ToolTarget::default()
        });
        failed.tool_result = Some(cortex_core::ToolResult {
            exit_code: Some(1),
            is_success: false,
            is_interrupted: false,
            output_bytes: Some(70),
            counts: None,
            failing: None,
        });
        assert!(failed.failed_output().is_some());
        inbox.push_event(failed);
        let Work::Events(events) = inbox.wait(None) else {
            return Err("no events".into());
        };
        let target = events
            .first()
            .and_then(|event| event.tool_target.as_ref())
            .ok_or("no target")?;
        assert_eq!(target.failed_output, None);
        assert_eq!(
            target
                .command
                .as_ref()
                .map(cortex_core::ShellCommand::as_str),
            Some("pytest")
        );
        let counts = events
            .first()
            .and_then(|event| event.tool_result)
            .and_then(|result| result.counts);
        assert_eq!(
            counts,
            Some(CheckCounts {
                failed: 1,
                passed: Some(2),
                errors: 0,
                skipped: 0,
            })
        );
        let failing = events
            .first()
            .and_then(|event| event.tool_result)
            .and_then(|result| result.failing);
        assert_eq!(failing, Some(ShortHash::of(b"test_ops.py::test_a")));
        let mut brought = event(HookKind::PostTool, Some(ToolKind::Shell), 2)?;
        brought.tool_result = Some(cortex_core::ToolResult {
            failing: Some(ShortHash::of(b"x")),
            ..failed_result()
        });
        inbox.push_event(brought);
        let Work::Events(events) = inbox.wait(None) else {
            return Err("no events".into());
        };
        assert_eq!(
            events.first().and_then(|event| event.tool_result?.failing),
            None
        );
        Ok(())
    }

    /// A shell line's result that exited 1, with nothing read from its output.
    fn failed_result() -> cortex_core::ToolResult {
        cortex_core::ToolResult {
            exit_code: Some(1),
            is_success: false,
            is_interrupted: false,
            output_bytes: Some(70),
            counts: None,
            failing: None,
        }
    }

    /// Events come out oldest first, before any change waiting with them.
    #[test]
    fn events_come_first_and_in_order() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        inbox.push_change(paths(&["a.py"])?, 1);
        inbox.push_event(event(HookKind::Prompt, None, 1)?);
        inbox.push_event(event(HookKind::Stop, None, 2)?);
        assert_eq!(times(inbox.wait(None)), [1, 2]);
        assert!(matches!(inbox.wait(None), Work::Changed(_)));
        Ok(())
    }

    /// In a full inbox an arriving read is dropped; an arriving edit takes the place of the oldest
    /// read waiting; with only edits, prompts and session events waiting, an arriving edit is
    /// dropped. Each drop is counted, and nothing waits past the cap.
    #[test]
    fn reads_and_searches_give_way_when_full() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        let read = |ts_ms| event(HookKind::PostTool, Some(ToolKind::Read), ts_ms);
        let edit = |ts_ms| event(HookKind::PostTool, Some(ToolKind::Edit), ts_ms);
        inbox.push_event(edit(0)?);
        inbox.push_event(read(1)?);
        inbox.push_event(read(2)?);
        for ts_ms in 3..i64::try_from(MAX_EVENTS)? {
            inbox.push_event(event(HookKind::Prompt, None, ts_ms)?);
        }
        assert_eq!(inbox.counts().events_waiting, MAX_EVENTS);
        inbox.push_event(read(20_000)?);
        inbox.push_event(edit(20_001)?);
        let counts = inbox.counts();
        assert_eq!(
            (counts.events_waiting, counts.events_dropped),
            (MAX_EVENTS, 2)
        );
        let drained = times(inbox.wait(None));
        assert_eq!(drained.get(..3), Some(&[0, 2, 3][..]));
        assert_eq!(drained.last(), Some(&20_001));
        for ts_ms in 0..i64::try_from(MAX_EVENTS)? {
            inbox.push_event(edit(ts_ms)?);
        }
        inbox.push_event(edit(30_000)?);
        let counts = inbox.counts();
        assert_eq!(
            (counts.events_waiting, counts.events_dropped),
            (MAX_EVENTS, 3)
        );
        assert!(!times(inbox.wait(None)).contains(&30_000));
        Ok(())
    }

    /// A writer waiting with no deadline wakes when a change arrives from another thread.
    #[test]
    fn a_change_wakes_the_waiting_writer() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = std::sync::Arc::new(Inbox::default());
        let sender = std::sync::Arc::clone(&inbox);
        let change = paths(&["x.py"])?;
        let thread = std::thread::spawn(move || sender.push_change(change, 1));
        assert_eq!(
            summary(inbox.wait(None)),
            Some((false, vec!["x.py".to_owned()]))
        );
        thread.join().map_err(|_| "the sending thread panicked")?;
        Ok(())
    }

    /// A mining waits behind the events and the changes waiting with it, and its answer reaches
    /// the miner. A closed inbox takes no mining, and one waiting when it closes is dropped: the
    /// miner's wait for its answer ends.
    #[test]
    fn a_mining_comes_after_events_and_changes() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        let answer = inbox
            .push_mining(Mining::Unchanged)
            .ok_or("an open inbox takes a mining")?;
        inbox.push_change(paths(&["a.py"])?, 1);
        inbox.push_event(event(HookKind::Prompt, None, 1)?);
        assert!(matches!(inbox.wait(None), Work::Events(_)));
        assert!(matches!(inbox.wait(None), Work::Changed(_)));
        let Work::Mining(job) = inbox.wait(None) else {
            return Err("the mining did not come third".into());
        };
        job.answer(Err("full disk".to_owned()));
        assert_eq!(answer.recv()?, Err("full disk".to_owned()));
        let waiting = inbox
            .push_mining(Mining::Unchanged)
            .ok_or("an open inbox takes a mining")?;
        inbox.close();
        assert_eq!(
            waiting.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        assert!(inbox.push_mining(Mining::Unchanged).is_none());
        Ok(())
    }

    /// The changes put in are counted, a dropped one too, and count as indexed only once the
    /// writer, having taken them, comes back for work: taking them is not indexing them. A change
    /// that comes while the writer works on the ones it took waits for the next round.
    #[test]
    fn changes_are_indexed_when_the_writer_comes_back() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        assert!(inbox.has_indexed(0));
        for number in 0..MAX_CHANGES {
            inbox.push_change(paths(&[&format!("f{number}.py")])?, 1);
        }
        inbox.push_change(paths(&["dropped.py"])?, 1);
        let count = u64::try_from(MAX_CHANGES)? + 1;
        assert_eq!(inbox.changes_in(), count);
        assert!(!inbox.has_indexed(1));
        assert!(matches!(
            inbox.wait(None),
            Work::Changed(Changed::Everything(_))
        ));
        assert!(!inbox.has_indexed(count));
        inbox.push_change(paths(&["late.py"])?, 2);
        assert!(matches!(inbox.wait(None), Work::Changed(_)));
        assert!(inbox.has_indexed(count));
        assert!(!inbox.has_indexed(count + 1));
        assert!(matches!(inbox.wait(Some(Instant::now())), Work::Timeout));
        assert!(inbox.has_indexed(count + 1));
        Ok(())
    }

    /// An injection of a prompt at `at_ms`, as typed here.
    fn injection(at_ms: i64) -> Result<Given, Box<dyn std::error::Error>> {
        Ok(Given::Injection(cortex_store::InjectionRecord {
            task_id: None,
            session_id: cortex_core::Identifier::new("s1")?,
            hook: HookKind::Prompt,
            at_ms,
            files: Vec::new(),
            tokens: 0,
            confidence: None,
            is_gated_out: true,
            is_holdout: false,
            text_hash: None,
            features: None,
        }))
    }

    /// When `given` holds an injection, its time.
    fn injected_at(given: &Given) -> Option<i64> {
        match given {
            Given::Injection(injection) => Some(injection.at_ms),
            Given::Warnings(..) | Given::Notices(_) | Given::Handoff(..) | Given::Facts(..) => None,
        }
    }

    /// The events and injections waiting are taken together, so that an injection is never taken
    /// without the event it followed; a wait takes the events first, then the injections.
    #[test]
    fn injections_come_out_after_their_events() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        inbox.push_event(event(HookKind::Prompt, None, 10)?);
        inbox.push_given(injection(10)?, 11);
        let (events, given) = inbox.take_waiting();
        let event_times: Vec<i64> = events.iter().map(|event| event.ts_ms).collect();
        let injection_times: Vec<Option<i64>> = given.iter().map(injected_at).collect();
        assert_eq!((event_times, injection_times), (vec![10], vec![Some(10)]));
        inbox.push_event(event(HookKind::Prompt, None, 20)?);
        inbox.push_given(injection(20)?, 21);
        assert_eq!(times(inbox.wait(None)), [20]);
        let Work::Given(given) = inbox.wait(None) else {
            return Err("the injection after its event".into());
        };
        assert_eq!(
            given.iter().map(injected_at).collect::<Vec<_>>(),
            [Some(20)]
        );
        Ok(())
    }

    /// When too many injections wait, the oldest gives way, and the daemon's latest errors say so;
    /// a closed inbox takes none.
    #[test]
    fn the_oldest_injection_gives_way() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Inbox::default();
        for at_ms in 0..=i64::try_from(MAX_GIVEN)? {
            assert!(inbox.push_given(injection(at_ms)?, 500));
        }
        let (_, given) = inbox.take_waiting();
        assert_eq!(given.len(), MAX_GIVEN);
        assert_eq!(given.first().and_then(injected_at), Some(1));
        let errors: Vec<String> = inbox
            .recent_errors()
            .into_iter()
            .map(|error| error.message)
            .collect();
        assert_eq!(errors, ["an injection was not recorded: too many waited"]);
        inbox.close();
        assert!(!inbox.push_given(injection(0)?, 600));
        Ok(())
    }
}
