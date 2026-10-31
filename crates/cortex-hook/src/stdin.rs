//! The hook's payload, read from standard input within a size cap and a time limit. The read runs
//! on a thread of its own, since standard input takes no timeout: past the limit the hook goes on
//! without it, and the thread ends with the process.

use std::io::{self, Read};
use std::sync::mpsc;
use std::time::Instant;

/// Most bytes of payload the hook reads, one mebibyte: a frame's size, so that no payload the hook
/// keeps is too long to forward. A longer payload is not read to its end.
pub(crate) const MAX_PAYLOAD_BYTES: usize = 1 << 20;

/// Why the hook has no payload.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StdinError {
    /// The payload is longer than [`MAX_PAYLOAD_BYTES`].
    #[error("the payload is longer than {MAX_PAYLOAD_BYTES} bytes")]
    TooLong,
    /// The payload did not end in time.
    #[error("the payload did not arrive in time")]
    TimedOut,
    /// Standard input closed with nothing on it.
    #[error("the payload is empty")]
    Empty,
    /// Standard input could not be read.
    #[error("reading the payload")]
    Io(#[source] io::Error),
}

/// Reads `input` to its end before `deadline`.
///
/// # Errors
///
/// [`StdinError::TooLong`] past [`MAX_PAYLOAD_BYTES`], [`StdinError::TimedOut`] when the input has
/// not ended by `deadline`, [`StdinError::Empty`] for no bytes at all, and the read's own error.
pub(crate) fn read_payload(
    input: impl Read + Send + 'static,
    deadline: Instant,
) -> Result<Vec<u8>, StdinError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut payload = Vec::new();
        // One byte past the cap tells a payload of exactly the cap from a longer one.
        let read = input
            .take(MAX_PAYLOAD_BYTES as u64 + 1)
            .read_to_end(&mut payload)
            .map(|_| payload);
        // The hook may have gone on without the payload; nobody is left to tell.
        drop(sender.send(read));
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    let payload = receiver
        .recv_timeout(remaining)
        .map_err(|_| StdinError::TimedOut)?
        .map_err(StdinError::Io)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(StdinError::TooLong);
    }
    if payload.is_empty() {
        return Err(StdinError::Empty);
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Duration;

    use super::*;

    /// A reader that blocks until its sender is dropped.
    struct Blocked(mpsc::Receiver<()>);

    impl Read for Blocked {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            // The sender is dropped when the test is done: either way, the read ends.
            let _ = self.0.recv();
            Ok(0)
        }
    }

    /// A payload of exactly the cap is read whole; one byte more is too long; none is empty.
    #[test]
    fn the_cap_and_an_empty_payload() {
        let soon = || Instant::now() + Duration::from_secs(10);
        let whole = read_payload(Cursor::new(vec![b'x'; MAX_PAYLOAD_BYTES]), soon());
        assert_eq!(
            whole.ok().map(|payload| payload.len()),
            Some(MAX_PAYLOAD_BYTES)
        );
        let long = read_payload(Cursor::new(vec![b'x'; MAX_PAYLOAD_BYTES + 1]), soon());
        assert!(matches!(long, Err(StdinError::TooLong)), "{long:?}");
        let empty = read_payload(Cursor::new(Vec::new()), soon());
        assert!(matches!(empty, Err(StdinError::Empty)), "{empty:?}");
    }

    /// Input that never ends is given up at the deadline, not waited for.
    #[test]
    fn a_silent_input_times_out() {
        let (hold, blocked) = mpsc::channel();
        let started = Instant::now();
        let silent = read_payload(Blocked(blocked), started + Duration::from_millis(50));
        let waited = started.elapsed();
        assert!(matches!(silent, Err(StdinError::TimedOut)), "{silent:?}");
        assert!(waited >= Duration::from_millis(50), "{waited:?}");
        assert!(waited < Duration::from_secs(2), "{waited:?}");
        drop(hold);
    }
}
