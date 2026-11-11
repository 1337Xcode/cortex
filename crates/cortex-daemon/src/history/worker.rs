//! The worker's thread: takes the oldest job, waits for its turn, carries the job out, hands what
//! it found to the writer through the inbox and waits for the write, then reports the store's
//! counts. One job at a time, one git process at a time.
//!
//! Its turn comes when nothing waits for the writer, no index or crawl is under way or due, and no
//! hook request arrived in the last [`HOOK_QUIET`] ([`is_turn`]): the worker waits, as upkeep
//! does, rather than run git beside a hook. It asks again before every git process of a job, so a
//! request arriving during a job is waited for between two processes; a process already running
//! is not cut short, and is bounded by its own limit: `history.blame_timeout_ms` for a blame, 10 s
//! for a commit's record and 30 s for the comparison that says whether a commit only moved white
//! space.
//!
//! A span whose blame failed (git did not end in time, say) or that the store refused, a span
//! whose file on disk is not the file at HEAD, and a commit that cannot be carded are remembered
//! (`super::remembered`) and not tried again: a span that times out once would time out on every
//! prompt that shows it.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cortex_store::Reader;

use super::blaming::{self, Context, Halt, JobSettings};
use super::jobs::Jobs;
use super::queue::Job;
use super::repository::{Repository, Unavailable};
use super::write::HistoryWrite;
use crate::inbox::Inbox;

/// How long after a hook request arrives the worker starts no git process: a hook's handling
/// takes 150 ms at the 95th percentile at most, so 300 ms leaves it the machine.
pub(super) const HOOK_QUIET: Duration = Duration::from_millis(300);

/// How often the worker looks again whether its turn has come while it waits.
const TURN_POLL: Duration = Duration::from_millis(100);

/// The worker between jobs.
pub(super) struct Worker {
    /// The repository's top.
    root: PathBuf,
    /// The jobs, the text cache, what is remembered and the status.
    jobs: Arc<Jobs>,
    /// Where writes go, and what says whether hooks or the writer are busy.
    inbox: Arc<Inbox>,
    /// The repository's git handles.
    repository: Repository,
    /// The store, read-only, once opened.
    reader: Option<Reader>,
    /// The store's memory map size, to open it with.
    mmap_bytes: u64,
    /// How jobs run.
    settings: JobSettings,
    /// The clock, in UTC milliseconds.
    clock: fn() -> i64,
}

impl Worker {
    /// A worker for the repository at `root`, its store opened with `mmap_bytes`, taking its jobs
    /// from `jobs`, handing writes to `inbox`, running jobs with `settings` at the times `clock`
    /// gives.
    pub(super) fn new(
        (root, mmap_bytes): (PathBuf, u64),
        (jobs, inbox): (Arc<Jobs>, Arc<Inbox>),
        settings: JobSettings,
        clock: fn() -> i64,
    ) -> Self {
        Self {
            repository: Repository::new(root.clone()),
            root,
            jobs,
            inbox,
            reader: None,
            mmap_bytes,
            settings,
            clock,
        }
    }

    /// Takes jobs until told to stop or the writer takes no more. A panic in a job, as a library
    /// reading the repository's untrusted content may meet, ends that job only.
    pub(super) fn run(mut self) {
        self.report_counts();
        while let Some(job) = self.jobs.next_job() {
            let carried = std::panic::catch_unwind(AssertUnwindSafe(|| self.carry_out(&job)));
            match carried {
                Ok(true) => self.report_counts(),
                Ok(false) => return,
                Err(_) => tracing::warn!("a history job stopped on an internal error"),
            }
        }
    }

    /// Carries out `job` and hands what it found to the writer: false when the worker is to stop.
    fn carry_out(&mut self, job: &Job) -> bool {
        if let Job::Blame(blame) | Job::LookAgain(blame) = job
            && self.jobs.has_failed(blame)
        {
            return true;
        }
        if !wait_for_turn(&self.jobs, &self.inbox) {
            return false;
        }
        match self.found(job) {
            Ok(Some(write)) => self.hand_over(job, write),
            Ok(None) => true,
            Err(Halt::Stopped) => false,
            Err(Halt::Blame(error)) => {
                if let Job::Blame(blame) | Job::LookAgain(blame) = job {
                    self.jobs.remember_failed(blame);
                }
                tracing::warn!(error = %error, "a blame failed and is not tried again");
                true
            }
            Err(Halt::Failed(why)) => {
                tracing::warn!(error = %why, "a history job failed");
                true
            }
        }
    }

    /// What `job` found to write, if anything.
    fn found(&mut self, job: &Job) -> Result<Option<HistoryWrite>, Halt> {
        if self.reader.is_none() {
            let folder = self.root.join(".cortex");
            self.reader = Some(Reader::open(&folder, self.mmap_bytes)?);
        }
        let (jobs, inbox) = (&self.jobs, &self.inbox);
        let may_go_on = || wait_for_turn(jobs, inbox);
        let context = Context {
            settings: &self.settings,
            now_ms: (self.clock)(),
            may_go_on: &may_go_on,
            jobs,
        };
        let queries = self
            .reader
            .as_ref()
            .ok_or_else(|| Halt::Failed("no store is open".to_owned()))?
            .queries();
        let git = match self.repository.at_head() {
            Ok(git) => git,
            Err(Unavailable::Refused(why)) => {
                self.jobs.refuse(&why);
                return Ok(None);
            }
            Err(Unavailable::NoCommit) => return Ok(None),
            Err(Unavailable::Failed(why)) => return Err(Halt::Failed(why)),
        };
        match job {
            Job::Blame(blame) => blaming::blame(&git, queries, (blame, false), &context),
            Job::LookAgain(blame) => blaming::blame(&git, queries, (blame, true), &context),
            Job::Card(oid) => blaming::card(&git, queries, oid, &context),
        }
    }

    /// Hands `write`, what `job` found, to the writer and waits for it: false when the writer
    /// takes no more. A blame the store refuses is remembered, so that it is not made again.
    fn hand_over(&self, job: &Job, write: HistoryWrite) -> bool {
        let Some(answer) = self.inbox.push_history(write) else {
            return false;
        };
        match answer.recv() {
            Ok(Ok(())) => true,
            Ok(Err(why)) => {
                if let Job::Blame(blame) | Job::LookAgain(blame) = job {
                    self.jobs.remember_failed(blame);
                }
                tracing::warn!(error = %why, "what the history worker found was not stored");
                true
            }
            Err(_) => false,
        }
    }

    /// Reports the store's counts of cards and cached blames; a store that cannot be read leaves
    /// the last counts.
    fn report_counts(&mut self) {
        if self.reader.is_none() {
            self.reader = Reader::open(&self.root.join(".cortex"), self.mmap_bytes).ok();
        }
        let Some(reader) = &self.reader else {
            return;
        };
        let queries = reader.queries();
        if let (Ok(cards), Ok(blames)) = (queries.change_card_count(), queries.cached_blame_count())
        {
            self.jobs.set_counts(cards, blames);
        }
    }
}

/// Whether the worker may start a git process now, by what `inbox` says: nothing waits for the
/// writer, no index or crawl is under way or due, and no hook request arrived in the last
/// [`HOOK_QUIET`].
pub(super) fn is_turn(inbox: &Inbox) -> bool {
    let is_quiet = inbox
        .last_request()
        .is_none_or(|at| at.elapsed() >= HOOK_QUIET);
    // INVARIANT: budgeted (the worker runs git only while no hook request is being served and the
    // writer has nothing waiting, so a hook never waits behind it)
    is_quiet && !inbox.has_work_waiting() && !inbox.is_indexing()
}

/// Waits until [`is_turn`]: false when the worker is told to stop meanwhile.
fn wait_for_turn(jobs: &Jobs, inbox: &Inbox) -> bool {
    loop {
        if jobs.is_stopping() {
            return false;
        }
        if is_turn(inbox) {
            return true;
        }
        if jobs.pause(TURN_POLL) {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::RepoPath;
    use cortex_index::Change;

    use super::*;

    /// The turn: a fresh inbox gives it; a hook request noted now takes it; so do changes
    /// waiting, and a crawl under way; an inbox with neither gives it.
    #[test]
    fn git_waits_for_hooks_and_the_index() -> Result<(), cortex_core::PathError> {
        assert!(is_turn(&Inbox::default()));
        let asked = Inbox::default();
        asked.note_request();
        assert!(!is_turn(&asked));
        let changed = Inbox::default();
        changed.push_change(Change::Paths([RepoPath::new("a.py")?].into()), 1);
        assert!(!is_turn(&changed));
        let crawling = Inbox::default();
        crawling.set_crawling(true);
        assert!(!is_turn(&crawling));
        crawling.set_crawling(false);
        assert!(is_turn(&crawling));
        Ok(())
    }
}
