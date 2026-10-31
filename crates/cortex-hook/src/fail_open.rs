//! The hook's promise to the agent: whatever goes wrong inside it, the agent sees exit code 0 and
//! nothing printed, by the event's hard timeout. A panic anywhere in the hook, on any of its
//! threads, is silenced and caught, and work that stalls is given up at its deadline.

use std::panic::{self, AssertUnwindSafe, UnwindSafe};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

/// How work run [`within`] a deadline ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It returned.
    Finished,
    /// It panicked, and the panic was caught.
    Panicked,
    /// The deadline passed first; the work was left to the process's exit.
    OutOfTime,
}

/// Runs `body` with every panic silenced, and reports whether it finished without one.
pub(crate) fn guard(body: impl FnOnce() + UnwindSafe) -> bool {
    // INVARIANT: fail-open. The default panic hook would print the panic to stderr, which the
    // agent may show or treat as hook output; this one prints nothing, for every thread.
    panic::set_hook(Box::new(|_| {}));
    // INVARIANT: fail-open. A panic ends here instead of in an exit code the agent could act on.
    panic::catch_unwind(body).is_ok()
}

/// Runs `work` guarded on a thread of its own and waits for it no later than `deadline`. Work still
/// running then is left behind: the hook returns, and its process's exit ends the work.
pub(crate) fn within(deadline: Instant, work: impl FnOnce() + Send + 'static) -> Outcome {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let finished = guard(AssertUnwindSafe(work));
        // The hook may have stopped waiting: nobody is left to tell.
        let _ = sender.send(finished);
    });
    // INVARIANT: budgeted. Nothing that stalls (the payload, the file system, the daemon) holds the
    // agent past the deadline.
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(true) => Outcome::Finished,
        // A thread that ends without a word can only have panicked past its guard.
        Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => Outcome::Panicked,
        Err(mpsc::RecvTimeoutError::Timeout) => Outcome::OutOfTime,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A body that panics is caught and reported; one that returns is reported as finished. Work
    /// given a deadline finishes, panics, or is given up at the deadline, not when it ends.
    #[test]
    fn panics_are_caught_and_stalls_given_up() {
        #[allow(clippy::panic, reason = "the panic is what the guard must catch")]
        let panicked = !guard(|| panic!("the hook broke"));
        assert!(panicked);
        assert!(guard(|| {}));

        let soon = || Instant::now() + Duration::from_secs(10);
        assert_eq!(within(soon(), || {}), Outcome::Finished);
        #[allow(clippy::panic, reason = "the panic is what the guard must catch")]
        let outcome = within(soon(), || panic!("the work broke"));
        assert_eq!(outcome, Outcome::Panicked);
        let started = Instant::now();
        let stalled = within(started + Duration::from_millis(50), || {
            thread::sleep(Duration::from_secs(30));
        });
        assert_eq!(stalled, Outcome::OutOfTime);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        // Other tests in this binary report their own panics.
        drop(panic::take_hook());
    }
}
