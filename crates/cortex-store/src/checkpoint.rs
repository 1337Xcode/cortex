//! Moving what the writer committed from the write-ahead log into the database file, on a thread
//! of its own. SQLite would otherwise do it inside the commit that takes the log past 1,000 pages,
//! copying the log and syncing the disk while the writer waits, and on a slow disk that wait passes
//! the second a hook event may wait for the writer. The writer still checkpoints in a commit once
//! the log passes 64 MiB, so that the log starts afresh however busy it is; the thread leaves that
//! checkpoint little to copy.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;

use crate::connection::{self, Access};
use crate::error::{StoreError, io};

/// How often the log is checkpointed: it then holds about a second of the writer's commits, and an
/// idle store costs one look a second.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(1);

/// How many checkpoints failed, readable from any thread while the writer works.
#[derive(Debug, Clone, Default)]
pub struct CheckpointFailures(Arc<AtomicU64>);

impl CheckpointFailures {
    /// How many checkpoints failed since the writer opened.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// Counts one more.
    fn add(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// The thread that checkpoints the log, stopped and waited for when dropped.
#[derive(Debug)]
pub(crate) struct Checkpointer {
    /// Dropped to tell the thread to stop.
    stop: Option<Sender<()>>,
    /// The thread.
    thread: Option<JoinHandle<()>>,
    /// The checkpoints that failed.
    failures: CheckpointFailures,
}

impl Checkpointer {
    /// Starts checkpointing the database at `database` every [`CHECKPOINT_EVERY`], on a
    /// connection of its own with the writer's settings.
    ///
    /// # Errors
    ///
    /// SQLite errors opening or configuring the connection, and the I/O error of a thread that
    /// could not be started.
    pub(crate) fn start(database: &Path, mmap_bytes: u64) -> Result<Self, StoreError> {
        let connection = connection::connect(database, Access::ReadWrite)?;
        connection::configure(&connection, Access::ReadWrite, mmap_bytes)?;
        Self::over(connection).map_err(io("starting the checkpoint thread", database))
    }

    /// Starts checkpointing through `connection` every [`CHECKPOINT_EVERY`].
    fn over(connection: Connection) -> std::io::Result<Self> {
        let (stop, stopped) = mpsc::channel();
        let failures = CheckpointFailures::default();
        let counted = failures.clone();
        let thread = thread::Builder::new()
            .name("cortex-checkpoint".to_owned())
            .spawn(move || run(&connection, &stopped, &counted))?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
            failures,
        })
    }

    /// The count of the checkpoints that failed.
    pub(crate) fn failures(&self) -> CheckpointFailures {
        self.failures.clone()
    }
}

impl Drop for Checkpointer {
    /// Stops the thread and waits for it, so that its connection is closed before the writer's.
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            // A thread that panicked has nothing left to undo: its connection is closed either way.
            let _ = thread.join();
        }
    }
}

/// Checkpoints through `connection` every [`CHECKPOINT_EVERY`] until told to stop, counting each
/// checkpoint that fails; the next one tries again.
fn run(connection: &Connection, stopped: &Receiver<()>, failures: &CheckpointFailures) {
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(CHECKPOINT_EVERY) {
        if checkpoint(connection).is_err() {
            failures.add();
        }
    }
}

/// One passive checkpoint: copies every committed page no reader still needs into the database
/// file, waiting for no reader or writer and calling no busy handler. One that finds another
/// checkpoint running reports it in its row and copies nothing, which is not a failure.
fn checkpoint(connection: &Connection) -> rusqlite::Result<()> {
    connection.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{migrate, schema};

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A connection that cannot write cannot copy the log into the database: each checkpoint it
    /// tries fails and is counted, and the thread goes on trying. The schema, written through a
    /// writer's connection that stays open and leaves a log that small alone, keeps pages in the
    /// log to copy.
    #[test]
    fn a_checkpoint_that_fails_is_counted() -> TestResult {
        let folder = tempfile::tempdir()?;
        let database = folder.path().join("cortex.db");
        let mut writing = connection::connect(&database, Access::ReadWrite)?;
        connection::configure(&writing, Access::ReadWrite, 0)?;
        migrate::migrate(
            &mut writing,
            schema::MIGRATIONS,
            &folder.path().join("backups"),
        )?;
        let read_only = connection::connect(&database, Access::ReadOnly)?;
        let checkpointer = Checkpointer::over(read_only)?;
        let failures = checkpointer.failures();
        // On Windows SQLite tries a failing write again for about 1.5 s before it gives up.
        let deadline = Instant::now() + 10 * CHECKPOINT_EVERY;
        while failures.count() < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        drop(checkpointer);
        assert!(failures.count() >= 2, "{} failures", failures.count());
        Ok(())
    }
}
