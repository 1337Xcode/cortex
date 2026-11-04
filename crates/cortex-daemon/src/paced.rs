//! Work a thread of its own does again each time it is asked, paced. However often it is asked, a
//! run starts at least a period after the one before, and at least a few times as long after it as
//! that one took, so that the work takes a bounded share of a core. A run that fails, or panics,
//! is handed back with why and tried again, a period later after one failure, twice as long after
//! each further one in a row, up to a ceiling. A stop is taken at once, between runs or while
//! waiting for the next. The graph is rebuilt this way.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How a paced job's runs are spaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pacing {
    /// The least time between the starts of two runs, which asks that come closer together share;
    /// also the wait after one failure.
    pub(crate) every: Duration,
    /// How many times as long as a run took the next waits, at least, from its start.
    pub(crate) spacing: u32,
    /// The longest a run that failed waits before it is tried again.
    pub(crate) max_retry_wait: Duration,
}

/// The asks a paced job waits for, and its stop.
#[derive(Debug, Default)]
pub(crate) struct Asks {
    /// Whether a run is asked for, and whether the thread is to stop.
    state: Mutex<AskState>,
    /// Wakes the thread when either changes.
    wake: Condvar,
}

/// What the thread of a paced job waits on.
#[derive(Debug, Default)]
struct AskState {
    /// A run was asked for since the latest began, or the latest failed.
    is_asked: bool,
    /// The thread is to stop.
    is_stopping: bool,
}

impl Asks {
    /// Asks for a run.
    pub(crate) fn ask(&self) {
        self.state().is_asked = true;
        self.wake.notify_all();
    }

    /// Whether a run is asked for and not yet begun. For tests.
    #[cfg(test)]
    pub(crate) fn is_asked(&self) -> bool {
        self.state().is_asked
    }

    /// Takes the ask waiting, as the thread does: whether one was there. For tests.
    #[cfg(test)]
    pub(crate) fn take(&self) -> bool {
        std::mem::take(&mut self.state().is_asked)
    }

    /// The state, locked.
    fn state(&self) -> MutexGuard<'_, AskState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the thread to stop.
    fn stop(&self) {
        self.state().is_stopping = true;
        self.wake.notify_all();
    }

    /// Waits until a run is asked for and `not_before` has come, or the thread is to stop, and
    /// takes the ask: `false` to stop. A stop is taken at once, however long the wait still had.
    fn wait(&self, not_before: Option<Instant>) -> bool {
        let mut state = self.state();
        loop {
            if state.is_stopping {
                return false;
            }
            if !state.is_asked {
                state = self
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let now = Instant::now();
            match not_before.filter(|at| *at > now) {
                Some(at) => {
                    state = self
                        .wake
                        .wait_timeout(state, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                None => {
                    state.is_asked = false;
                    return true;
                }
            }
        }
    }
}

/// How the latest run went, which paces the next.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Last {
    /// When it started.
    pub(crate) started: Instant,
    /// How long it took.
    pub(crate) took: Duration,
    /// How many runs in a row, it included, failed.
    pub(crate) failures: u32,
}

/// When the run after `last` may start under `pacing`: at least its period after it, at least its
/// spacing times as long as it took, and after failures in a row a period, twice as long for each
/// further one, at most its longest retry wait.
pub(crate) fn next_start(last: Last, pacing: Pacing) -> Instant {
    let paced = pacing.every.max(last.took.saturating_mul(pacing.spacing));
    let retry = match last.failures {
        0 => Duration::ZERO,
        failures => pacing
            .every
            .saturating_mul(2_u32.saturating_pow(failures))
            .min(pacing.max_retry_wait),
    };
    last.started + paced.max(retry)
}

/// A paced job's thread, stopped and waited for when dropped.
#[derive(Debug)]
pub(crate) struct Paced {
    /// What it waits for.
    asks: Arc<Asks>,
    /// The thread, when it started.
    thread: Option<JoinHandle<()>>,
}

impl Paced {
    /// Starts the thread `name`, which does `run` at once and then after each ask of `asks`, spaced
    /// by `pacing`, until stopped. A run that fails or panics is handed to `failed` with why, and
    /// asked for again. A thread that cannot start is logged, and the work is not done.
    pub(crate) fn start(
        name: &str,
        asks: &Arc<Asks>,
        pacing: Pacing,
        (run, failed): (
            impl FnMut() -> Result<(), String> + Send + 'static,
            impl Fn(&str) + Send + 'static,
        ),
    ) -> Self {
        let shared = Arc::clone(asks);
        asks.ask();
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run_until_stopped(&shared, pacing, (run, failed)));
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                tracing::warn!(thread = name, error = %error, "a paced thread did not start");
                None
            }
        };
        Self {
            asks: Arc::clone(asks),
            thread,
        }
    }
}

impl Drop for Paced {
    /// Stops the thread and waits for it: a run under way finishes first.
    fn drop(&mut self) {
        self.asks.stop();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!("a paced thread panicked");
        }
    }
}

/// Does `run` after each ask of `asks` until told to stop, each start as [`next_start`] paces it
/// under `pacing`. A run that fails or panics goes to `failed`, and is asked for again.
fn run_until_stopped(
    asks: &Asks,
    pacing: Pacing,
    (mut run, failed): (impl FnMut() -> Result<(), String>, impl Fn(&str)),
) {
    let mut last: Option<Last> = None;
    while asks.wait(last.map(|last| next_start(last, pacing))) {
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(&mut run))
            .unwrap_or_else(|_| Err("the work panicked".to_owned()));
        let took = started.elapsed();
        let failures = match outcome {
            Ok(()) => 0,
            Err(why) => {
                failed(&why);
                asks.ask();
                last.map_or(0, |last| last.failures).saturating_add(1)
            }
        };
        last = Some(Last {
            started,
            took,
            failures,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The graph's pacing, which the cases below are worked for: half a second, four times as
    /// long as a run took, at most a minute after failures.
    const PACING: Pacing = Pacing {
        every: Duration::from_millis(500),
        spacing: 4,
        max_retry_wait: Duration::from_secs(60),
    };

    /// A run starts half a second after the one before, or four times as long as that one took
    /// when that is longer: 100 ms of work waits 0.5 s, 200 ms 0.8 s, and 2 s 8 s. After failures
    /// in a row it waits a second for one, two for two, four for three, and at most a minute: 64 s
    /// for seven is held to 60.
    #[test]
    fn runs_are_paced_by_how_long_they_take_and_how_many_failed() {
        let started = Instant::now();
        let cases = [
            ((100, 0), 500),
            ((125, 0), 500),
            ((200, 0), 800),
            ((2_000, 0), 8_000),
            ((10, 1), 1_000),
            ((10, 2), 2_000),
            ((10, 3), 4_000),
            ((10, 7), 60_000),
            ((10, 40), 60_000),
            ((1_000, 1), 4_000),
        ];
        for ((took_ms, failures), wait_ms) in cases {
            let last = Last {
                started,
                took: Duration::from_millis(took_ms),
                failures,
            };
            assert_eq!(
                next_start(last, PACING),
                started + Duration::from_millis(wait_ms),
                "{took_ms} ms, {failures} failures"
            );
        }
    }

    /// A run that fails is handed back with why and asked for again, a period later: two failures,
    /// then a run that succeeds, and no more without an ask.
    #[test]
    fn a_failed_run_is_handed_back_and_tried_again() -> TestResult {
        let quick = Pacing {
            every: Duration::from_millis(10),
            ..PACING
        };
        let asks = Arc::new(Asks::default());
        let runs = Arc::new(AtomicU32::new(0));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let (counted, noted) = (Arc::clone(&runs), Arc::clone(&failures));
        let paced = Paced::start(
            "cortex-test",
            &asks,
            quick,
            (
                move || {
                    if counted.fetch_add(1, Ordering::SeqCst) < 2 {
                        return Err("the disk is busy".to_owned());
                    }
                    Ok(())
                },
                move |why: &str| {
                    noted
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(why.to_owned());
                },
            ),
        );
        let until = Instant::now() + Duration::from_secs(10);
        while runs.load(Ordering::SeqCst) < 3 {
            if Instant::now() >= until {
                return Err("three runs never came".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // A run asked for no more is not done again.
        std::thread::sleep(quick.every * 10);
        drop(paced);
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        let failures = failures.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(*failures, ["the disk is busy", "the disk is busy"]);
        Ok(())
    }

    /// A run that panics is handed back as a failure, and the thread goes on to the next.
    #[test]
    #[allow(clippy::panic, reason = "the run under test panics on purpose")]
    fn a_panicking_run_is_handed_back_and_tried_again() -> TestResult {
        let quick = Pacing {
            every: Duration::from_millis(10),
            ..PACING
        };
        let asks = Arc::new(Asks::default());
        let runs = Arc::new(AtomicU32::new(0));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let (counted, noted) = (Arc::clone(&runs), Arc::clone(&failures));
        let _paced = Paced::start(
            "cortex-test",
            &asks,
            quick,
            (
                move || {
                    if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                        panic!("a run that panics");
                    }
                    Ok(())
                },
                move |why: &str| {
                    noted
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(why.to_owned());
                },
            ),
        );
        let until = Instant::now() + Duration::from_secs(10);
        while runs.load(Ordering::SeqCst) < 2 {
            if Instant::now() >= until {
                return Err("the second run never came".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let failures = failures.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(*failures, ["the work panicked"]);
        Ok(())
    }
}
