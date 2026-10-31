//! What waits for the writer: hook events, the watcher's changes, word that the watcher stopped,
//! and a request to stop, with the counts the daemon's status reports. One lock and one condition
//! variable, so the writer sleeps until something arrives or a deadline passes.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use cortex_core::{Event, HookKind, RepoPath, ToolKind};
use cortex_index::Change;

/// How many hook events may wait for the writer. When the inbox is full, a read or a search gives
/// way first: an arriving one is dropped, or the oldest waiting one makes room for an edit, a
/// prompt or a session's event. Only when every waiting event is of those does an arriving one go.
/// Every drop is counted.
pub(crate) const MAX_EVENTS: usize = 10_000;

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
}

/// What waits in the inbox.
#[derive(Debug, Default)]
struct State {
    /// Hook events, oldest first.
    events: VecDeque<Event>,
    /// Hook events dropped since the daemon started.
    events_dropped: u64,
    /// Batches of events the writer could not record, since the daemon started.
    failed_batches: u64,
    /// The watcher's changes, oldest first.
    changes: Vec<Change>,
    /// Whether a change was dropped since the last drain.
    is_overflowed: bool,
    /// Whether the daemon was asked to stop.
    is_stopping: bool,
    /// Whether the watcher stopped feeding the inbox.
    is_watcher_gone: bool,
}

/// What the writer is to do next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Stop.
    Stop,
    /// Record these events, oldest first.
    Events(Vec<Event>),
    /// Index what changed.
    Changed(Changed),
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
    /// Only these paths, in path order.
    Paths(Vec<RepoPath>),
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
}

impl Inbox {
    /// Puts in a hook event, making room as [`MAX_EVENTS`] says when the inbox is full.
    pub(crate) fn push_event(&self, event: Event) {
        let mut state = self.lock();
        if state.events.len() >= MAX_EVENTS {
            state.events_dropped += 1;
            if is_droppable(&event) {
                return;
            }
            match state.events.iter().position(is_droppable) {
                Some(oldest) => drop(state.events.remove(oldest)),
                None => return,
            }
        }
        state.events.push_back(event);
        drop(state);
        self.arrived.notify_one();
    }

    /// Counts a batch of events the writer could not record.
    pub(crate) fn count_failed_batch(&self) {
        self.lock().failed_batches += 1;
    }

    /// What waits, and what was lost.
    pub(crate) fn counts(&self) -> Counts {
        let state = self.lock();
        Counts {
            events_waiting: state.events.len(),
            changes_waiting: state.changes.len(),
            events_dropped: state.events_dropped,
            failed_batches: state.failed_batches,
        }
    }

    /// Puts in a change from the watcher, or drops it and marks the overflow when the inbox holds
    /// [`MAX_CHANGES`].
    pub(crate) fn push_change(&self, change: Change) {
        let mut state = self.lock();
        if state.changes.len() >= MAX_CHANGES {
            state.is_overflowed = true;
        } else {
            state.changes.push(change);
        }
        drop(state);
        self.arrived.notify_one();
    }

    /// Asks the writer to stop.
    pub(crate) fn stop(&self) {
        self.lock().is_stopping = true;
        self.arrived.notify_one();
    }

    /// Says the watcher stopped feeding the inbox.
    pub(crate) fn watcher_stopped(&self) {
        self.lock().is_watcher_gone = true;
        self.arrived.notify_one();
    }

    /// Waits until an event, a change, a stop or word of the watcher arrives, or `until` passes
    /// (never, when `None`), and says what to do: a stop first, then every event waiting, then
    /// every change waiting, drained into one, then that the watcher stopped.
    pub(crate) fn wait(&self, until: Option<Instant>) -> Work {
        let mut state = self.lock();
        loop {
            if state.is_stopping {
                return Work::Stop;
            }
            if !state.events.is_empty() {
                return Work::Events(state.events.drain(..).collect());
            }
            if !state.changes.is_empty() || state.is_overflowed {
                return Work::Changed(drain(&mut state));
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
/// the paths they name. The overflow mark is cleared with the changes.
fn drain(state: &mut State) -> Changed {
    let mut paths = BTreeSet::new();
    let mut is_everything = std::mem::take(&mut state.is_overflowed);
    for change in state.changes.drain(..) {
        match change {
            Change::Paths(named) => paths.extend(named),
            Change::Everything => is_everything = true,
        }
    }
    let paths = paths.into_iter().collect();
    if is_everything {
        Changed::Everything(paths)
    } else {
        Changed::Paths(paths)
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
            Changed::Paths(paths) => (false, paths.iter().map(ToString::to_string).collect()),
        })
    }

    /// Changes waiting together are drained into one, in path order; one change to everything
    /// makes the whole batch one; a dropped change, marked as an overflow, does too, once.
    #[test]
    fn waiting_changes_drain_into_one() -> Result<(), cortex_core::PathError> {
        let inbox = Inbox::default();
        inbox.push_change(paths(&["b.py"])?);
        inbox.push_change(paths(&["a.py", "b.py"])?);
        let names = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            summary(inbox.wait(None)),
            Some((false, names(&["a.py", "b.py"])))
        );
        inbox.push_change(paths(&["c.py"])?);
        inbox.push_change(Change::Everything);
        assert_eq!(summary(inbox.wait(None)), Some((true, names(&["c.py"]))));
        for _ in 0..MAX_CHANGES {
            inbox.push_change(paths(&["d.py"])?);
        }
        inbox.push_change(paths(&["e.py"])?);
        assert_eq!(summary(inbox.wait(None)), Some((true, names(&["d.py"]))));
        inbox.push_change(paths(&["f.py"])?);
        assert_eq!(summary(inbox.wait(None)), Some((false, names(&["f.py"]))));
        Ok(())
    }

    /// With nothing waiting, a deadline in the past times out at once and one in the future after
    /// waiting for it; changes come before word that the watcher stopped, and a stop before any
    /// change, every time it is asked.
    #[test]
    fn a_stop_comes_first_and_deadlines_time_out() -> Result<(), cortex_core::PathError> {
        let inbox = Inbox::default();
        assert_eq!(inbox.wait(Some(Instant::now())), Work::Timeout);
        let start = Instant::now();
        assert_eq!(
            inbox.wait(Some(start + Duration::from_millis(50))),
            Work::Timeout
        );
        assert!(start.elapsed() >= Duration::from_millis(50));
        inbox.push_change(paths(&["w.py"])?);
        inbox.watcher_stopped();
        assert!(matches!(inbox.wait(None), Work::Changed(_)));
        assert_eq!(inbox.wait(None), Work::WatcherStopped);
        inbox.push_change(paths(&["a.py"])?);
        inbox.stop();
        assert_eq!(inbox.wait(None), Work::Stop);
        assert_eq!(inbox.wait(None), Work::Stop);
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
        inbox.push_change(paths(&["a.py"])?);
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
        let thread = std::thread::spawn(move || sender.push_change(change));
        assert_eq!(
            summary(inbox.wait(None)),
            Some((false, vec!["x.py".to_owned()]))
        );
        thread.join().map_err(|_| "the sending thread panicked")?;
        Ok(())
    }
}
