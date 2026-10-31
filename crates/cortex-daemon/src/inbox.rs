//! What waits for the writer: the watcher's changes, word that the watcher stopped, and a request
//! to stop. One lock and one condition variable, so the writer sleeps until something arrives or a
//! deadline passes.

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use cortex_core::RepoPath;
use cortex_index::Change;

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

impl Inbox {
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

    /// Waits until a change, a stop or word of the watcher arrives, or `until` passes (never,
    /// when `None`), and says what to do: a stop first, then every change waiting, drained into
    /// one, then that the watcher stopped.
    pub(crate) fn wait(&self, until: Option<Instant>) -> Work {
        let mut state = self.lock();
        loop {
            if state.is_stopping {
                return Work::Stop;
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
