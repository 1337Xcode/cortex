//! What the caller of a long write does between two of its write transactions: an index of the
//! repository, a mining of its history. The daemon commits the hook events waiting for it there, so
//! that a write taking minutes never holds them back past the second a hard kill may lose.

use std::fmt;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::Writer;

/// How often the daemon's pause comes: a quarter of the second of hook events a hard kill may
/// lose. Fixed by that bound, as the batch size is by the work in a batch, rather than tuned.
pub const PAUSE_EVERY: Duration = Duration::from_millis(250);

/// The shortest wait between two looks for a message that is not ready: a pause that comes every
/// zero seconds, as a test asks for, must not turn the wait into a spin.
const SHORTEST_WAIT: Duration = Duration::from_millis(1);

/// A callback a long write calls with its writer and no transaction open: between each two of its
/// stretches, and inside the long ones at the first safe point once `every` has passed since the
/// last call. It never calls the write back.
#[derive(Clone, Copy)]
pub struct Pause<'a> {
    /// What the caller does.
    callback: &'a dyn Fn(&mut Writer),
    /// How long the write may go without calling it.
    every: Duration,
}

impl<'a> Pause<'a> {
    /// A pause that calls `callback` between a write's stretches and, inside them, once `every`
    /// has passed; the daemon gives [`PAUSE_EVERY`].
    #[must_use]
    pub fn new(callback: &'a dyn Fn(&mut Writer), every: Duration) -> Self {
        Self { callback, every }
    }
}

impl fmt::Debug for Pause<'_> {
    /// A callback has nothing to show but its period.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pause")
            .field("every", &self.every)
            .finish_non_exhaustive()
    }
}

/// The time since a long write last let its caller in, for a write that offers to at safe points:
/// no transaction open, nothing of the store borrowed. One pacer serves a whole write.
#[derive(Debug)]
pub struct Pacer<'a> {
    /// The caller's pause, if it gave one.
    pause: Option<Pause<'a>>,
    /// When the caller was last let in.
    since: Instant,
}

impl<'a> Pacer<'a> {
    /// A pacer for `pause`, counting from now.
    #[must_use]
    pub fn new(pause: Option<Pause<'a>>) -> Self {
        Self {
            pause,
            since: Instant::now(),
        }
    }

    /// Whether the caller's period has passed since it was last let in: a transaction open that
    /// long is due to be committed. Never, without a pause.
    #[must_use]
    pub fn is_due(&self) -> bool {
        self.pause
            .is_some_and(|pause| self.since.elapsed() >= pause.every)
    }

    /// Lets the caller in when it is due: what a long stretch of work does at each safe point.
    pub fn offer(&mut self, writer: &mut Writer) {
        if self.is_due() {
            self.let_in(writer);
        }
    }

    /// Lets the caller in now: what a write does between two of its stretches, however short they
    /// were, so that a write that ends quickly lets its caller in too.
    pub fn let_in(&mut self, writer: &mut Writer) {
        if let Some(pause) = self.pause {
            (pause.callback)(writer);
            self.since = Instant::now();
        }
    }

    /// How long a wait goes before the caller is offered a turn.
    fn period(&self) -> Duration {
        self.pause
            .map_or(PAUSE_EVERY, |pause| pause.every)
            .max(SHORTEST_WAIT)
    }
}

/// Waits for the one message `receiver` will get, offering the caller a turn every period
/// meanwhile. `None` when its sender went without a word.
pub fn wait_for<T>(
    receiver: &Receiver<T>,
    pacer: &mut Pacer<'_>,
    writer: &mut Writer,
) -> Option<T> {
    loop {
        match receiver.recv_timeout(pacer.period()) {
            Ok(message) => return Some(message),
            Err(RecvTimeoutError::Timeout) => pacer.offer(writer),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::sync::mpsc;
    use std::thread;

    use cortex_core::RepoId;

    use crate::WriterOptions;

    use super::*;

    /// A writer for a store of its own in `folder`.
    fn writer(folder: &tempfile::TempDir) -> Result<Writer, Box<dyn Error>> {
        Ok(Writer::open(&WriterOptions {
            folder: &folder.path().join(".cortex"),
            repo_id: &RepoId::of(folder.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?)
    }

    /// A pacer lets the caller in once [`PAUSE_EVERY`] has passed since it last did, and then
    /// counts afresh: after a wait of 270 ms, the first offer lets it in and an offer right after
    /// does not. Between two stretches it lets the caller in whatever the time. Without a pause
    /// it is never due and calls nothing.
    #[test]
    fn a_pacer_lets_the_caller_in_once_per_turn() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let mut writer = writer(&folder)?;
        let calls = Cell::new(0);
        let count = |_: &mut Writer| calls.set(calls.get() + 1);
        let mut pacer = Pacer::new(Some(Pause::new(&count, PAUSE_EVERY)));
        let mut silent = Pacer::new(None);
        thread::sleep(PAUSE_EVERY + Duration::from_millis(20));
        assert!(pacer.is_due());
        assert!(!silent.is_due());
        pacer.offer(&mut writer);
        assert_eq!(calls.get(), 1);
        pacer.offer(&mut writer);
        assert_eq!(calls.get(), 1);
        pacer.let_in(&mut writer);
        assert_eq!(calls.get(), 2);
        silent.offer(&mut writer);
        silent.let_in(&mut writer);
        assert_eq!(calls.get(), 2);
        Ok(())
    }

    /// While a message is late, the caller is let in every period: a message 600 ms late lets it in
    /// at least twice, at about 250 and 500 ms. A sender that goes without a word ends the wait
    /// with nothing.
    #[test]
    fn a_late_message_lets_the_caller_in() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let mut writer = writer(&folder)?;
        let calls = Cell::new(0);
        let count = |_: &mut Writer| calls.set(calls.get() + 1);
        let mut pacer = Pacer::new(Some(Pause::new(&count, PAUSE_EVERY)));
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(600));
            // The test may have stopped waiting: nothing is left to tell.
            let _ = sender.send(7);
        });
        assert_eq!(wait_for(&receiver, &mut pacer, &mut writer), Some(7));
        assert!(calls.get() >= 2, "{} calls", calls.get());
        let (silent, nothing) = mpsc::channel::<u8>();
        drop(silent);
        assert_eq!(wait_for(&nothing, &mut pacer, &mut writer), None);
        Ok(())
    }
}
