//! The history worker: a thread of its own that reads, in the background, who last changed the
//! lines Cortex may show and what those commits were, so that a later prompt finds the answer in
//! the store and in memory without running git. It starts only when `history.why` or
//! `history.seeds` is on, once the daemon's first index is done; with both off there is no thread
//! and no git is run.
//!
//! - A retrieval asks for a blame of a span as HEAD holds it ([`Jobs::ask_blame`]) and reads a
//!   commit's subject and reason line from the text cache ([`Jobs::text`]); neither waits for git.
//!   The blames kept are read from the store (`Queries::cached_blames`).
//! - Upkeep's history step prunes the blames kept and asks for blames of the files worth warming
//!   ([`warm`]).
//! - The worker takes one job at a time from a queue of at most 64, the oldest dropped when it is
//!   full ([`queue`]); waits while hook requests are served or an index runs ([`worker`]); blames
//!   through one `Blamer` a repository, opened again when HEAD or the repository's configuration
//!   may have changed ([`repository`]); cards every commit a blame names that has no card
//!   ([`carding`]); walks one step past the runs of trivial, mass and bot commits ([`walk`]); and
//!   hands what it found to the writer through the inbox, never writing the store itself
//!   ([`write`]).
//! - The text of a commit, its subject and reason line, is held in memory for at most 2,000
//!   commits ([`texts`]), never stored and never logged.
//!
//! Dropping the [`History`] tells the thread to stop and waits for it, at most the longest limit
//! of one of its git processes (a blame's `history.blame_timeout_ms`, a commit's record and its
//! white-space check, `cortex_git::card::RECORD_WITHIN` and `COSMETIC_WITHIN`) and [`STOP_MARGIN`]
//! more: a job under way ends before its next git process, and the one running ends at its limit,
//! so that none outlives the daemon.

#![warn(clippy::pedantic)]

mod blaming;
mod carding;
mod for_work_orders;
mod jobs;
mod queue;
mod remembered;
mod repository;
#[cfg(test)]
mod tests;
mod texts;
mod walk;
mod warm;
mod worker;
mod write;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use cortex_core::Config;
use cortex_git::card::{COSMETIC_WITHIN, RECORD_WITHIN};

pub(crate) use self::jobs::Jobs;
pub(crate) use self::warm::warm;
pub(crate) use self::write::{HistoryWrite, HistoryWriteJob, store};

use self::blaming::{JobSettings, MAX_CARDS_A_JOB};
use self::carding::CardSettings;
use self::worker::Worker;
use crate::host::now_ms;
use crate::inbox::Inbox;
use crate::troubles::Step;

/// How much longer than a blame's time limit a stop waits for the worker: time for the blame's
/// process to be stopped and the thread to end.
const STOP_MARGIN: Duration = Duration::from_secs(1);

/// The history worker's thread, told to stop when this is dropped.
#[derive(Debug)]
pub(crate) struct History {
    /// The worker's shared state, told to stop on drop.
    jobs: Arc<Jobs>,
    /// Closed when the worker's thread ends.
    ended: Receiver<()>,
    /// How long a stop waits for the thread to end.
    stop_within: Duration,
}

impl History {
    /// Starts the worker for the repository at `root` under `config`, its jobs taken from `jobs`
    /// and its writes handed to `inbox`, when `history.why` or `history.seeds` is on; none when
    /// both are off. A thread that cannot be started is logged and told to the user as a failure
    /// to read the git history, and the daemon goes on without it.
    pub(crate) fn start(
        root: &Path,
        config: &Config,
        inbox: &Arc<Inbox>,
        jobs: &Arc<Jobs>,
    ) -> Option<Self> {
        Self::start_with(root, config, (inbox, jobs), (now_ms, MAX_CARDS_A_JOB))
    }

    /// Starts the worker as [`History::start`] does, with `clock` giving the time in UTC
    /// milliseconds and at most `max_cards` cards made a job.
    fn start_with(
        root: &Path,
        config: &Config,
        (inbox, jobs): (&Arc<Inbox>, &Arc<Jobs>),
        (clock, max_cards): (fn() -> i64, usize),
    ) -> Option<Self> {
        if !config.history.is_why && !config.history.is_seeds {
            return None;
        }
        let place = (root.to_path_buf(), config.mmap_bytes);
        let shared = (Arc::clone(jobs), Arc::clone(inbox));
        let settings = JobSettings {
            max_cards,
            ..settings(config)
        };
        // The longest one git process of the worker may run: a blame, a commit's record or its
        // white-space check, each stopped at its own limit.
        let longest = Duration::from_millis(settings.blame_timeout_ms)
            .max(RECORD_WITHIN)
            .max(COSMETIC_WITHIN);
        let stop_within = longest + STOP_MARGIN;
        let (ending, ended) = mpsc::channel::<()>();
        jobs.turn_on();
        // The worker is made on its own thread: the history it reads in-process is not `Send`.
        // The thread holds `ending` until it ends, which closes the channel.
        let started = std::thread::Builder::new()
            .name("cortex-history".to_owned())
            .spawn(move || {
                Worker::new(place, shared, settings, clock).run();
                drop(ending);
            });
        match started {
            Ok(_) => Some(Self {
                jobs: Arc::clone(jobs),
                ended,
                stop_within,
            }),
            Err(error) => {
                jobs.stop();
                let why = format!("the history worker could not start: {error}");
                tracing::warn!(error = %why, "no history is read");
                inbox.note_trouble(Step::Mining, &why, now_ms());
                None
            }
        }
    }
}

impl Drop for History {
    /// Tells the worker to stop, and waits for its thread to end, at most the time a blame may
    /// take and a margin.
    fn drop(&mut self) {
        self.jobs.stop();
        if self.ended.recv_timeout(self.stop_within) == Err(RecvTimeoutError::Timeout) {
            tracing::warn!("the history worker did not end in time");
        }
    }
}

/// The settings jobs run with, from `config`.
fn settings(config: &Config) -> JobSettings {
    JobSettings {
        blame_timeout_ms: u64::from(config.history.blame_timeout_ms),
        max_age_days: config.git_max_age_days,
        max_file_bytes: config.max_file_bytes,
        max_cards: MAX_CARDS_A_JOB,
        card: CardSettings {
            max_files: usize::try_from(config.git_max_files_per_commit).unwrap_or(usize::MAX),
            bot_patterns: config.history.bot_patterns.clone(),
        },
    }
}
