//! What waits for the writer: hook events, the watcher's changes, a mining of the history, word
//! that the watcher stopped, and a request to stop, with the counts, the miner's state and the
//! latest errors the daemon's status reports, and how many of the watcher's changes the writer has
//! indexed, which the miner waits on. One lock and one condition variable, so the writer sleeps
//! until something arrives or a deadline passes. It also holds the rules the writer keeps each
//! session's positions by, set once the configuration is read.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use cortex_coord::Timing;
use cortex_core::ipc::{GitReport, RecentError};
use cortex_core::{CoordinationConfig, Event, HookKind, RepoPath, ToolKind};
use cortex_git::mine::Mining;
use cortex_index::Change;
use cortex_store::CheckpointFailures;

use crate::given::Given;

use crate::recent_errors::RecentErrors;
use crate::troubles::{Step, Troubles};

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
    /// A mining waiting to be written: one at most, since the miner waits for the write.
    mining: Option<MiningJob>,
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
    /// before it next looks.
    pub(crate) fn push_event(&self, event: Event) -> bool {
        let mut state = self.lock();
        if state.is_closed {
            return false;
        }
        if state.events.len() >= MAX_EVENTS {
            state.events_dropped += 1;
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

    /// What the user is told at `now_ms` of the steps that failed since last told, and from
    /// then on nothing of those failures; none when none failed.
    pub(crate) fn take_troubles(&self, now_ms: i64) -> Option<String> {
        self.lock().troubles.take_said(now_ms)
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

    /// How many changes the watcher put in since the daemon started.
    pub(crate) fn changes_in(&self) -> u64 {
        self.lock().changes_in
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
    /// whatever stopped it, before it records the last events waiting. A mining still waiting is
    /// dropped unwritten, which ends the miner's wait for it; the next daemon mines again.
    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.is_closed = true;
        state.mining = None;
    }

    /// Says the watcher stopped feeding the inbox.
    pub(crate) fn watcher_stopped(&self) {
        self.lock().is_watcher_gone = true;
        self.arrived.notify_one();
    }

    /// Waits until an event, a change, a mining, a stop or word of the watcher arrives, or `until`
    /// passes (never, when `None`), and says what to do: a stop first, then every event waiting,
    /// then every change waiting, drained into one, then the mining, then that the watcher
    /// stopped. Only the writer waits, so the changes it took before are indexed by now.
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
            if let Some(job) = state.mining.take() {
                return Work::Mining(Box::new(job));
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

    /// The times of the events a wait drains.
    fn times(work: Work) -> Vec<i64> {
        match work {
            Work::Events(events) => events.iter().map(|event| event.ts_ms).collect(),
            _ => Vec::new(),
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
            Given::Warnings(..) | Given::Notices(_) | Given::Handoff(..) => None,
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
