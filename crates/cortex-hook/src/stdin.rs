//! The hook's payload, read from standard input within a size cap and a time limit, to the end of
//! the JSON value it holds. The read runs on a thread of its own, since standard input takes no
//! timeout: past the limit the hook goes on without it, and the thread ends with the process.

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
    /// The thread reading the payload stopped without a result.
    #[error("the payload's reader stopped")]
    ReaderStopped,
}

/// Bytes asked of the input at each read.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// Reads `input` before `deadline`: to the end of the JSON object or array it starts with, or to
/// its end when it starts with anything else. An agent may keep its end of the pipe open after
/// writing the payload, so the end of the value, not of the input, ends the read.
///
/// # Errors
///
/// [`StdinError::TooLong`] past [`MAX_PAYLOAD_BYTES`], [`StdinError::TimedOut`] when the payload
/// has not ended by `deadline`, [`StdinError::Empty`] for no bytes at all, and the read's own
/// error.
pub(crate) fn read_payload(
    input: impl Read + Send + 'static,
    deadline: Instant,
) -> Result<Vec<u8>, StdinError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        // One byte past the cap tells a payload of exactly the cap from a longer one.
        let read = read_value(input.take(MAX_PAYLOAD_BYTES as u64 + 1));
        // The hook may have gone on without the payload; nobody is left to tell.
        drop(sender.send(read));
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    let payload = receiver
        .recv_timeout(remaining)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => StdinError::TimedOut,
            mpsc::RecvTimeoutError::Disconnected => StdinError::ReaderStopped,
        })?
        .map_err(StdinError::Io)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(StdinError::TooLong);
    }
    if payload.is_empty() {
        return Err(StdinError::Empty);
    }
    Ok(payload)
}

/// Reads `input` to the end of the JSON object or array it starts with, cut there, or to its end.
fn read_value(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let mut end = ValueEnd::default();
    let mut chunk = vec![0; READ_CHUNK_BYTES];
    loop {
        let read = match input.read(&mut chunk) {
            Ok(0) => return Ok(payload),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let bytes = chunk.get(..read).unwrap_or_default();
        payload.extend_from_slice(bytes);
        if let Some(length) = end.feed(bytes) {
            payload.truncate(length);
            return Ok(payload);
        }
    }
}

/// Finds where the JSON object or array a payload starts with ends, from the payload's bytes in
/// order: a byte-order mark and white space before it are passed over, brackets are counted
/// outside strings, and a string's escaped characters are passed over. A payload that starts with
/// anything else has no end it can find.
#[derive(Debug, Default)]
struct ValueEnd {
    /// How many bytes it has seen.
    seen: usize,
    /// How many objects and arrays are open; none before the first.
    depth: usize,
    /// Whether the payload starts with something other than an object or an array.
    is_other: bool,
    /// Whether the bytes are inside a string.
    is_in_string: bool,
    /// Whether the last byte, inside a string, was a backslash that escapes this one.
    is_escaped: bool,
}

impl ValueEnd {
    /// Takes the payload's next `bytes`: the payload's length up to the end of its value, once
    /// that has passed.
    fn feed(&mut self, bytes: &[u8]) -> Option<usize> {
        for &byte in bytes {
            self.seen += 1;
            if self.is_other {
                return None;
            }
            if self.is_in_string {
                match (self.is_escaped, byte) {
                    (true, _) => self.is_escaped = false,
                    (false, b'\\') => self.is_escaped = true,
                    (false, b'"') => self.is_in_string = false,
                    (false, _) => {}
                }
                continue;
            }
            match (self.depth, byte) {
                (0, b' ' | b'\t' | b'\r' | b'\n' | 0xEF | 0xBB | 0xBF) => {}
                (_, b'{' | b'[') => self.depth += 1,
                (0, _) => self.is_other = true,
                (_, b'"') => self.is_in_string = true,
                (1, b'}' | b']') => return Some(self.seen),
                (_, b'}' | b']') => self.depth -= 1,
                (_, _) => {}
            }
        }
        None
    }
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

    /// A reader that panics.
    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            #[allow(clippy::panic, reason = "a reader that breaks is what is tested")]
            {
                panic!("the reader broke")
            }
        }
    }

    /// A reader that gives `chunks` one read each, then blocks until its sender is dropped, as an
    /// agent that keeps its end of the pipe open after writing the payload.
    struct OpenAfter {
        /// What is left to give, in order.
        chunks: std::collections::VecDeque<Vec<u8>>,
        /// Dropped by the test when it is done.
        hold: mpsc::Receiver<()>,
    }

    impl Read for OpenAfter {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(mut chunk) = self.chunks.pop_front() else {
                let _ = self.hold.recv();
                return Ok(0);
            };
            // A buffer smaller than the chunk takes its start; the rest is the next read's.
            if chunk.len() > buffer.len() {
                self.chunks.push_front(chunk.split_off(buffer.len()));
            }
            buffer[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    /// A payload whose JSON object has ended is read at once, though the input stays open: its
    /// braces counted outside strings, a string's escaped quote and its brackets inside it passed
    /// over, the object cut where it ends, its line break and what follows left unread. The read is
    /// done well before the deadline, ten seconds off.
    #[test]
    fn an_object_is_read_once_it_ends_though_the_input_stays_open() {
        let object = br#"{"a":"} \" ]","b":[1,{"c":2}],"d":"\\"}"#;
        let (hold, waiting) = mpsc::channel();
        let started = Instant::now();
        let mut whole = object.to_vec();
        whole.extend_from_slice(b"\n{}");
        let read = read_payload(
            OpenAfter {
                chunks: [whole].into(),
                hold: waiting,
            },
            started + Duration::from_secs(10),
        );
        assert_eq!(read.ok().as_deref(), Some(&object[..]));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        drop(hold);
    }

    /// The end of an object is found across reads of one byte each, after a byte-order mark and
    /// spaces, and an array's end as an object's; text that is no object or array is read to the
    /// input's end, as is an object that never ends.
    #[test]
    fn a_value_s_end_is_found_across_reads() {
        let soon = || Instant::now() + Duration::from_secs(10);
        for value in [&b"\xEF\xBB\xBF \n{\"a\":[{}]}"[..], b"[{\"]\":1}]"] {
            let (hold, waiting) = mpsc::channel();
            let chunks = value.iter().map(|byte| vec![*byte]).collect();
            let read = read_payload(
                OpenAfter {
                    chunks,
                    hold: waiting,
                },
                soon(),
            );
            assert_eq!(read.ok().as_deref(), Some(value));
            drop(hold);
        }
        for text in [&b"\"a string\" 1"[..], b"{\"open\":", b"42"] {
            let read = read_payload(Cursor::new(text.to_vec()), soon());
            assert_eq!(read.ok().as_deref(), Some(text));
        }
    }

    /// A reader that dies is told apart from one that is slow: the hook knows at once.
    #[test]
    fn a_dead_reader_is_not_a_slow_one() {
        let started = Instant::now();
        let dead = read_payload(Broken, started + Duration::from_secs(10));
        assert!(matches!(dead, Err(StdinError::ReaderStopped)), "{dead:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    /// A reader that gives the bytes of `text` cut at `cuts`, one piece a read.
    struct Pieces(std::collections::VecDeque<Vec<u8>>);

    impl Read for Pieces {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(mut piece) = self.0.pop_front() else {
                return Ok(0);
            };
            if piece.len() > buffer.len() {
                self.0.push_front(piece.split_off(buffer.len()));
            }
            buffer[..piece.len()].copy_from_slice(&piece);
            Ok(piece.len())
        }
    }

    /// Any JSON value a few levels deep, its strings drawn from the characters that steer the
    /// reader: quotes, backslashes, brackets of both kinds, a line break and a letter past ASCII.
    fn json_value() -> impl proptest::strategy::Strategy<Value = serde_json::Value> {
        use proptest::prelude::*;
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<i32>().prop_map(serde_json::Value::from),
            "[\"\\\\{}\\[\\] a\né]{0,6}".prop_map(serde_json::Value::from),
        ];
        leaf.prop_recursive(4, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::from),
                prop::collection::btree_map("[\"\\\\{}a]{0,4}", inner, 0..4)
                    .prop_map(|fields| serde_json::Value::Object(fields.into_iter().collect())),
            ]
        })
    }

    proptest::proptest! {
        /// Any JSON object or array, as serde_json writes it, compact or pretty, in pieces of any
        /// size and with anything after it, is read to its last byte and no further.
        #[test]
        fn any_object_or_array_is_read_to_its_end(
            value in json_value(),
            is_pretty in proptest::prelude::any::<bool>(),
            cuts in proptest::collection::vec(0usize..512, 0..8),
            after in "[ -~]{0,8}",
        ) {
            let value = match value {
                serde_json::Value::Object(_) | serde_json::Value::Array(_) => value,
                scalar => serde_json::Value::Array(vec![scalar]),
            };
            let written = if is_pretty {
                serde_json::to_vec_pretty(&value)
            } else {
                serde_json::to_vec(&value)
            }
            .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
            let mut text = written.clone();
            text.extend_from_slice(after.as_bytes());
            let mut pieces = std::collections::VecDeque::new();
            let mut cuts: Vec<usize> = cuts.into_iter().map(|cut| cut % (text.len() + 1)).collect();
            cuts.sort_unstable();
            let mut from = 0;
            // An empty piece would read as the input's end.
            for cut in cuts.into_iter().chain([text.len()]).filter(|cut| *cut > 0) {
                if cut > from {
                    pieces.push_back(text[from..cut].to_vec());
                    from = cut;
                }
            }
            let read = read_value(Pieces(pieces))
                .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
            proptest::prop_assert_eq!(read, written);
        }
    }
}
