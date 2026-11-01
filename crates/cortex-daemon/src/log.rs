//! The daemon's log: `daemon-<repo id>.log` in the user's log folder, one line per event at info
//! level and above, written by a thread of its own and rotated at 10 MiB across five files.
//!
//! A line holds the time in UTC milliseconds, the level, the module the event came from, its
//! message and its fields, made one line of at most 1,000 characters (`one_line`) whatever
//! reached it. What the daemon logs is its own words, ids, counts, times and paths, never code,
//! prompt text or tool output. Lines wait for the thread in a bounded queue, so the daemon never
//! waits for the disk; a line that finds the queue full is dropped, and the log says how many were.

use std::fs::File;
use std::io::{self, Write as _};
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context as _;
use cortex_core::{
    RepoId, UserFolder, append_private_file, create_private_folder, one_line, rotate_log,
    unix_millis,
};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

/// The size past which the log starts a new file.
const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// How many files the log keeps: the current one and four older ones, `.1` the newest.
const KEPT_FILES: usize = 5;

/// Most characters a line keeps: far more than any line the daemon writes, so that only text that
/// reached it from elsewhere, such as a long path in an error, is ever cut.
const MAX_LINE_CHARS: usize = 1_000;

/// How many lines may wait for the log's thread. A burst of events that outruns the disk drops
/// lines rather than making the daemon wait or its memory grow.
const MAX_WAITING_LINES: usize = 1_000;

/// How long a stopping daemon waits for its log to be written out. A full queue takes
/// milliseconds; a disk that takes longer is stalled, and the daemon stops without the rest rather
/// than keep the repository from the next daemon.
const FLUSH_WITHIN: Duration = Duration::from_secs(2);

/// What the log's thread is sent.
enum Message {
    /// A line to write.
    Line(String),
    /// Write the lines sent before and stop.
    Stop,
}

/// The daemon's log while the daemon runs. Dropped, it writes the lines sent before, waiting for
/// them at most [`FLUSH_WITHIN`].
pub(crate) struct Log {
    /// The way to the thread.
    sender: SyncSender<Message>,
    /// Closed by the thread when it is done.
    done: Receiver<()>,
}

impl Drop for Log {
    /// Tells the thread to stop once it has written what came before, and waits for it, both at
    /// most [`FLUSH_WITHIN`]; a thread still busy then is left to the process's exit.
    fn drop(&mut self) {
        let deadline = Instant::now() + FLUSH_WITHIN;
        let mut stop = Message::Stop;
        loop {
            match self.sender.try_send(stop) {
                Err(TrySendError::Full(back)) if Instant::now() < deadline => {
                    stop = back;
                    thread::sleep(Duration::from_millis(1));
                }
                // Sent, out of time, or a thread already gone, which has nothing left to write.
                _ => break,
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        // The thread sends nothing: the wait ends when it closes its end, or at the deadline, and
        // either way there is nothing more to do.
        let _ = self.done.recv_timeout(left);
    }
}

/// Starts the log of the daemon of repository `repo_id` in `user`'s log folder as the process's
/// tracing subscriber: from now on, every event at info level and above, from any of the daemon's
/// threads, becomes a line. A panic becomes one too, naming where it happened and nothing of what
/// it said, before the panic is reported as usual.
///
/// # Errors
///
/// The log's folder or file cannot be created or opened, its thread cannot start, or the process
/// has a subscriber already.
pub(crate) fn start(user: &UserFolder, repo_id: &RepoId) -> anyhow::Result<Log> {
    let path = user.logs_folder().join(format!("daemon-{repo_id}.log"));
    let file = LogFile::open(&path, ROTATE_AT_BYTES)
        .with_context(|| format!("opening the log {}", path.display()))?;
    let (sender, receiver) = mpsc::sync_channel(MAX_WAITING_LINES);
    let (finished, done) = mpsc::channel::<()>();
    let dropped = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&dropped);
    thread::Builder::new()
        .name("cortex-log".to_owned())
        .spawn(move || {
            write_lines(&receiver, file, &counted);
            drop(finished);
        })
        .context("starting the log's thread")?;
    let lines = LineLayer {
        sender: sender.clone(),
        dropped,
    };
    let subscriber = tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(lines);
    tracing::subscriber::set_global_default(subscriber)
        .context("making the log the process's subscriber")?;
    let reported = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        log_panic(panic.location());
        reported(panic);
    }));
    Ok(Log { sender, done })
}

/// Logs a panic at `location`: where it happened, never what it said.
fn log_panic(location: Option<&Location<'_>>) {
    let place = location.map_or_else(String::new, |location| {
        format!("{}:{}", location.file(), location.line())
    });
    // INVARIANT: no-content-stored (a panic's message may quote a value it was handed, such as a
    // line of a file being parsed, so only its place is logged)
    tracing::error!(location = %place, "the daemon panicked");
}

/// The layer that makes each event a line and hands it to the log's thread.
struct LineLayer {
    /// The way to the thread.
    sender: SyncSender<Message>,
    /// How many lines found the queue full since the thread last said so.
    dropped: Arc<AtomicU64>,
}

impl<S: Subscriber> Layer<S> for LineLayer {
    /// Hands `event`, as a line stamped now, to the thread, or counts it dropped when the queue is
    /// full.
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let line = line_of(event, unix_millis(SystemTime::now()));
        if let Err(TrySendError::Full(_)) = self.sender.try_send(Message::Line(line)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// `event` as one line at `at_ms`: `<at_ms> <LEVEL> <module>: <message> <name>=<value>...`, cut to
/// [`MAX_LINE_CHARS`] with control and hidden characters made spaces.
fn line_of(event: &Event<'_>, at_ms: i64) -> String {
    let metadata = event.metadata();
    let mut fields = Fields::default();
    event.record(&mut fields);
    let text = format!(
        "{at_ms} {} {}: {}{}",
        metadata.level(),
        metadata.target(),
        fields.message,
        fields.others
    );
    one_line(&text, MAX_LINE_CHARS)
}

/// An event's message and its other fields, as text.
#[derive(Default)]
struct Fields {
    /// The message.
    message: String,
    /// Every other field, each as ` name=value`.
    others: String,
}

impl Fields {
    /// Adds the field `field` of value `value`.
    fn add(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.others.push_str(&format!(" {}={value}", field.name()));
        }
    }
}

impl Visit for Fields {
    /// A text field, as it is.
    fn record_str(&mut self, field: &Field, value: &str) {
        self.add(field, value);
    }

    /// Any other field: a number, a flag, or a value given with `%`, which shows as its own
    /// text, or with `?`.
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.add(field, &format!("{value:?}"));
    }
}

/// Writes the lines `receiver` brings to `file` until told to stop or every sender is gone, saying
/// how many lines were dropped, `dropped`, whenever some were: before the next line, or before
/// stopping. A line that cannot be written is lost: the log has nowhere else to report it.
fn write_lines(receiver: &Receiver<Message>, mut file: LogFile, dropped: &AtomicU64) {
    while let Ok(Message::Line(line)) = receiver.recv() {
        note_dropped(&mut file, dropped);
        drop(file.append(&line));
    }
    note_dropped(&mut file, dropped);
}

/// Writes to `file` how many lines were dropped since it last said so, if any.
fn note_dropped(file: &mut LogFile, dropped: &AtomicU64) {
    let lost = dropped.swap(0, Ordering::Relaxed);
    if lost > 0 {
        let at_ms = unix_millis(SystemTime::now());
        let notice = format!(
            "{at_ms} WARN {}: {lost} lines were dropped while the log fell behind",
            module_path!()
        );
        drop(file.append(&notice));
    }
}

/// The log's file, and how much it holds.
struct LogFile {
    /// Where it is.
    path: PathBuf,
    /// The file, opened for appending.
    file: File,
    /// How many bytes it holds, or have been written since a rotation that failed.
    size_bytes: u64,
    /// The size past which it starts a new file.
    rotate_at_bytes: u64,
}

impl LogFile {
    /// The log at `path`, created private, with its folder, when missing, and appended to when
    /// not; it starts a new file once it holds `rotate_at_bytes`.
    fn open(path: &Path, rotate_at_bytes: u64) -> io::Result<Self> {
        if let Some(folder) = path.parent() {
            create_private_folder(folder)?;
        }
        let file = append_private_file(path)?;
        let size_bytes = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file,
            size_bytes,
            rotate_at_bytes,
        })
    }

    /// Writes `line` and a line break, first starting a new file when this one holds
    /// `rotate_at_bytes` or more, [`KEPT_FILES`] kept. When the log cannot be moved aside (on
    /// Windows, a reader that does not share deletion holds it), the line goes on the end of this
    /// file, and the next try waits until it holds `rotate_at_bytes` more: trying on every line
    /// would write none.
    fn append(&mut self, line: &str) -> io::Result<()> {
        if self.size_bytes >= self.rotate_at_bytes {
            self.size_bytes = 0;
            if rotate_log(&self.path, KEPT_FILES).is_ok() {
                self.file = append_private_file(&self.path)?;
            }
        }
        let text = format!("{line}\n");
        self.file.write_all(text.as_bytes())?;
        self.size_bytes = self
            .size_bytes
            .saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn Error>>;

    /// The lines a layer over a queue of `capacity` makes of the events `emit` sends, with this
    /// thread's subscriber, and how many it dropped.
    fn lines_of(capacity: usize, emit: impl FnOnce()) -> (Vec<String>, u64) {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let layer = LineLayer {
            sender,
            dropped: Arc::clone(&dropped),
        };
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::INFO)
            .with(layer);
        tracing::subscriber::with_default(subscriber, emit);
        let lines = receiver
            .try_iter()
            .filter_map(|message| match message {
                Message::Line(line) => Some(line),
                Message::Stop => None,
            })
            .collect();
        (lines, dropped.load(Ordering::Relaxed))
    }

    /// Each of `lines` without its time, which is checked to be a number of milliseconds.
    fn without_times(lines: &[String]) -> Result<Vec<&str>, Box<dyn Error>> {
        let mut texts = Vec::new();
        for line in lines {
            let (time, text) = line.split_once(' ').ok_or("a line without a time")?;
            time.parse::<i64>()?;
            texts.push(text);
        }
        Ok(texts)
    }

    /// An event is the time, the level, the module, the message and the fields: a hook handled
    /// with request id 2, its hook named `post_tool`; a warning whose error holds a line break,
    /// which becomes a space. A debug event is not written. The time is the clock's when the event
    /// was made.
    #[test]
    fn an_event_is_one_line() -> TestResult {
        let before_ms = unix_millis(SystemTime::now());
        let (lines, dropped) = lines_of(10, || {
            tracing::info!(request_id = 2_u64, hook = "post_tool", "hook handled");
            tracing::debug!("not written");
            tracing::warn!(error = %"one\ntwo", "a connection broke");
        });
        let after_ms = unix_millis(SystemTime::now());
        assert_eq!(
            without_times(&lines)?,
            [
                "INFO cortex_daemon::log::tests: hook handled request_id=2 hook=post_tool",
                "WARN cortex_daemon::log::tests: a connection broke error=one two",
            ]
        );
        let times: Vec<i64> = lines
            .iter()
            .filter_map(|line| line.split_once(' ')?.0.parse().ok())
            .collect();
        assert!(
            times
                .iter()
                .all(|at_ms| (before_ms..=after_ms).contains(at_ms)),
            "{times:?}"
        );
        assert_eq!(dropped, 0);
        Ok(())
    }

    /// A panic is logged by where it happened, here this test's own line, and nothing else.
    #[test]
    fn a_panic_is_logged_by_its_place() -> TestResult {
        let here = Location::caller();
        let (lines, _) = lines_of(10, || log_panic(Some(here)));
        let expected = format!(
            "ERROR cortex_daemon::log: the daemon panicked location={}:{}",
            here.file(),
            here.line()
        );
        assert_eq!(without_times(&lines)?, [expected.as_str()]);
        Ok(())
    }

    /// Three events into a queue of one: the first waits, the other two are dropped and counted.
    /// The thread then writes that two were dropped, then the first line. Lines dropped after the
    /// last line written are told before the thread stops.
    #[test]
    fn a_full_queue_drops_and_says_so() -> TestResult {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let layer = LineLayer {
            sender: sender.clone(),
            dropped: Arc::clone(&dropped),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            for number in 0..3 {
                tracing::info!(number, "event");
            }
        });
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("daemon.log");
        let file = LogFile::open(&path, ROTATE_AT_BYTES)?;
        let counted = Arc::clone(&dropped);
        let thread = thread::spawn(move || write_lines(&receiver, file, &counted));
        sender.send(Message::Stop)?;
        thread.join().map_err(|_| "the log's thread panicked")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(Message::Stop)?;
        dropped.store(3, Ordering::Relaxed);
        write_lines(&receiver, LogFile::open(&path, ROTATE_AT_BYTES)?, &dropped);
        let written: Vec<String> = std::fs::read_to_string(&path)?
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            without_times(&written)?,
            [
                "WARN cortex_daemon::log: 2 lines were dropped while the log fell behind",
                "INFO cortex_daemon::log::tests: event number=0",
                "WARN cortex_daemon::log: 3 lines were dropped while the log fell behind",
            ]
        );
        Ok(())
    }

    /// A log that rotates past 100 bytes, given three lines of 59 characters (60 bytes with the
    /// break): the first two go to the file (120 bytes), the third finds it past 100 and starts a
    /// new one, the first two now in `.1`. A log opened again counts what it already holds.
    #[test]
    fn the_log_rotates_at_its_size() -> TestResult {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("logs").join("daemon.log");
        let older = folder.path().join("logs").join("daemon.log.1");
        let lines: Vec<String> = ["a", "b", "c"].map(|letter| letter.repeat(59)).into();
        let mut log = LogFile::open(&path, 100)?;
        for line in &lines {
            log.append(line)?;
        }
        drop(log);
        assert_eq!(read(&path)?, lines.get(2..).ok_or("three lines")?);
        assert_eq!(read(&older)?, lines.get(..2).ok_or("three lines")?);
        let reopened = LogFile::open(&path, 100)?;
        assert_eq!(reopened.size_bytes, 60);
        Ok(())
    }

    /// When the log cannot be moved aside (here a folder holds the name it moves to first), the
    /// line that found it full still goes on its end, the older files stay as they were, and the
    /// next line does not try again: five lines of 60 bytes past a limit of 100 are all in the
    /// log, `.1` untouched.
    #[test]
    fn a_rotation_that_fails_loses_no_line() -> TestResult {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("daemon.log");
        let older = folder.path().join("daemon.log.1");
        std::fs::write(&older, "kept\n")?;
        let blocker = folder.path().join("daemon.log.5");
        std::fs::create_dir(&blocker)?;
        std::fs::write(blocker.join("inside"), "x")?;
        let lines: Vec<String> = ["a", "b", "c", "d", "e"]
            .map(|letter| letter.repeat(59))
            .into();
        let mut log = LogFile::open(&path, 100)?;
        for line in &lines {
            log.append(line)?;
        }
        drop(log);
        assert_eq!(read(&path)?, lines);
        assert_eq!(read(&older)?, ["kept"]);
        Ok(())
    }

    /// The lines of the file at `path`.
    fn read(path: &Path) -> io::Result<Vec<String>> {
        Ok(std::fs::read_to_string(path)?
            .lines()
            .map(str::to_owned)
            .collect())
    }
}
