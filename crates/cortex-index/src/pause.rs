//! What the caller of an index does between two of the index's write transactions. The daemon
//! commits the hook events waiting for it there, so that an index taking minutes never holds them
//! back past the second a hard kill may lose.

use std::fmt;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use cortex_store::Writer;

/// The longest an index goes without letting its caller in while it waits for files to be read
/// and parsed: a quarter of the second of hook events a hard kill may lose.
pub const PAUSE_EVERY: Duration = Duration::from_millis(250);

/// A callback an index calls with its writer and no transaction open: at least every
/// [`PAUSE_EVERY`] while it waits for a batch of files to be prepared, after each batch it writes,
/// and between its passes. It never calls the index back.
#[derive(Clone, Copy)]
pub struct Pause<'a>(&'a dyn Fn(&mut Writer));

impl<'a> Pause<'a> {
    /// A pause that calls `callback`.
    #[must_use]
    pub fn new(callback: &'a dyn Fn(&mut Writer)) -> Self {
        Self(callback)
    }
}

impl fmt::Debug for Pause<'_> {
    /// A callback has nothing to show.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Pause")
    }
}

/// Lets the caller in, when it gave a pause.
pub(crate) fn between(pause: Option<Pause<'_>>, writer: &mut Writer) {
    if let Some(Pause(callback)) = pause {
        callback(writer);
    }
}

/// The time since an index last let its caller in, for a pass that offers to at safe points: no
/// transaction open, nothing of the store borrowed.
pub(crate) struct Pacer<'a> {
    /// The caller's pause, if it gave one.
    pause: Option<Pause<'a>>,
    /// When the caller was last let in.
    since: Instant,
}

impl<'a> Pacer<'a> {
    /// A pacer for `pause`, counting from now.
    pub(crate) fn new(pause: Option<Pause<'a>>) -> Self {
        Self {
            pause,
            since: Instant::now(),
        }
    }

    /// Whether [`PAUSE_EVERY`] has passed since the caller was last let in: a transaction open
    /// that long is due to be committed.
    pub(crate) fn is_due(&self) -> bool {
        self.since.elapsed() >= PAUSE_EVERY
    }

    /// Lets the caller in when it is due.
    pub(crate) fn offer(&mut self, writer: &mut Writer) {
        if self.is_due() {
            between(self.pause, writer);
            self.since = Instant::now();
        }
    }
}

/// Waits for the one message `receiver` will get, letting the caller in every [`PAUSE_EVERY`]
/// meanwhile. `None` when its sender went without a word.
pub(crate) fn wait_for<T>(
    receiver: &Receiver<T>,
    pause: Option<Pause<'_>>,
    writer: &mut Writer,
) -> Option<T> {
    loop {
        match receiver.recv_timeout(PAUSE_EVERY) {
            Ok(message) => return Some(message),
            Err(RecvTimeoutError::Timeout) => between(pause, writer),
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
    use cortex_store::WriterOptions;

    use super::*;

    /// While a message is late, the caller is let in every [`PAUSE_EVERY`]: a message 600 ms late
    /// lets it in at least twice, at about 250 and 500 ms. A sender that goes without a word ends
    /// the wait with nothing.
    #[test]
    fn a_late_message_lets_the_caller_in() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let mut writer = Writer::open(&WriterOptions {
            folder: &folder.path().join(".cortex"),
            repo_id: &RepoId::of(folder.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        let calls = Cell::new(0);
        let count = |_: &mut Writer| calls.set(calls.get() + 1);
        let pause = Pause::new(&count);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(600));
            // The test may have stopped waiting: nothing is left to tell.
            let _ = sender.send(7);
        });
        assert_eq!(wait_for(&receiver, Some(pause), &mut writer), Some(7));
        assert!(calls.get() >= 2, "{} calls", calls.get());
        let (silent, nothing) = mpsc::channel::<u8>();
        drop(silent);
        assert_eq!(wait_for(&nothing, Some(pause), &mut writer), None);
        Ok(())
    }
}
