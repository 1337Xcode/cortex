//! What the history worker shares with the rest of the daemon: the jobs waiting, the text cache,
//! what the status reports, what the worker remembers not to try again, and the warming done
//! today. A retrieval asks for a blame or reads a commit's text here without waiting for git;
//! upkeep adds the files it warms. Nothing here runs git or touches the store. Until the worker
//! starts, which it does only when `history.why` or `history.seeds` is on, nothing is taken: an
//! ask is refused and the status says nothing.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cortex_core::ipc::HistoryReport;

use super::queue::{BlameJob, Job, Pushed, Queue};
use super::remembered::{Remembered, SpanKey};
use super::texts::{CardText, Texts};

/// A UTC day, in milliseconds: warming counts the files it asked for per day.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// The worker's shared state.
#[derive(Debug, Default)]
pub(crate) struct Jobs {
    /// What is shared.
    state: Mutex<State>,
    /// Signalled when a job arrives or the worker is told to stop.
    arrived: Condvar,
}

/// What is shared, behind one lock.
#[derive(Debug, Default)]
struct State {
    /// Whether the worker started, so that jobs are taken.
    is_on: bool,
    /// Whether the worker was told to stop.
    is_stopping: bool,
    /// The jobs waiting.
    queue: Queue,
    /// The texts of the commits carded or read lately.
    texts: Texts,
    /// What the status reports, the jobs waiting apart.
    report: HistoryReport,
    /// The spans whose blame failed or that the store refused.
    failed: Remembered<SpanKey, ()>,
    /// The spans whose file on disk was not the file at HEAD, with the HEAD it was not.
    unblameable: Remembered<SpanKey, String>,
    /// The commits that could not be carded.
    uncardable: Remembered<String, ()>,
    /// The UTC day warming last asked for files on, and how many it asked for that day.
    warmed: (i64, u64),
}

/// The key of `job`'s span.
fn span_key(job: &BlameJob) -> SpanKey {
    (
        job.path.clone(),
        job.file_hash,
        job.first_line,
        job.last_line,
    )
}

impl Jobs {
    /// Asks for a blame of `job`'s span, kept for the next prompt that shows it; never waits for
    /// git. True when it was added to the queue; false when the worker is off or stopped, or the
    /// same job waits already.
    pub(crate) fn ask_blame(&self, job: BlameJob) -> bool {
        self.push(Job::Blame(job))
    }

    /// The subject and reason line of the commit `oid`, when the text cache holds them; reading
    /// them counts as a use. Never runs git.
    pub(crate) fn text(&self, oid: &str) -> Option<CardText> {
        self.lock().texts.get(oid)
    }

    /// Asks the worker to read the commit `oid` again, its card when the store has none and its
    /// text, so that a later prompt finds them; not for a commit that could not be read before.
    pub(crate) fn ask_text(&self, oid: &str) {
        if !self.is_uncardable(oid) {
            self.push(Job::Card(oid.to_owned()));
        }
    }

    /// What the status reports: none while the worker is off.
    pub(crate) fn report(&self) -> Option<HistoryReport> {
        let state = self.lock();
        state.is_on.then(|| HistoryReport {
            jobs_waiting: u64::try_from(state.queue.len()).unwrap_or(u64::MAX),
            ..state.report.clone()
        })
    }

    /// How many more files warming may ask for on the UTC day of `now_ms`, of at most `max_a_day`.
    pub(crate) fn warming_room(&self, now_ms: i64, max_a_day: u64) -> u64 {
        let day = now_ms.div_euclid(DAY_MS);
        let state = self.lock();
        let asked = if state.warmed.0 == day {
            state.warmed.1
        } else {
            0
        };
        max_a_day.saturating_sub(asked)
    }

    /// Counts `files` warming added to the queue on the UTC day of `now_ms`.
    pub(crate) fn count_warmed(&self, now_ms: i64, files: u64) {
        let day = now_ms.div_euclid(DAY_MS);
        let mut state = self.lock();
        if state.warmed.0 != day {
            state.warmed = (day, 0);
        }
        state.warmed.1 = state.warmed.1.saturating_add(files);
    }

    /// Whether `job`'s span is remembered as one not to blame: its blame failed, the store refused
    /// it, or its file on disk was not the file at a HEAD.
    pub(crate) fn is_remembered(&self, job: &BlameJob) -> bool {
        let key = span_key(job);
        let state = self.lock();
        state.failed.get(&key).is_some() || state.unblameable.get(&key).is_some()
    }

    /// Starts taking jobs: the worker started.
    pub(super) fn turn_on(&self) {
        self.lock().is_on = true;
    }

    /// Tells the worker to stop; no job is taken after.
    pub(super) fn stop(&self) {
        let mut state = self.lock();
        state.is_stopping = true;
        state.is_on = false;
        drop(state);
        self.arrived.notify_all();
    }

    /// Whether the worker was told to stop.
    pub(super) fn is_stopping(&self) -> bool {
        self.lock().is_stopping
    }

    /// Waits for the oldest job and takes it; none once the worker is told to stop.
    pub(super) fn next_job(&self) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if state.is_stopping {
                return None;
            }
            if let Some(job) = state.queue.pop() {
                return Some(job);
            }
            state = self
                .arrived
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Waits `pause`, or less when the worker is told to stop: whether it was.
    pub(super) fn pause(&self, pause: Duration) -> bool {
        let state = self.lock();
        if state.is_stopping {
            return true;
        }
        let (state, _) = self
            .arrived
            .wait_timeout(state, pause)
            .unwrap_or_else(PoisonError::into_inner);
        state.is_stopping
    }

    /// Asks the worker to card the commit `oid`, which a job named and left for later.
    pub(super) fn ask_card(&self, oid: &str) {
        self.push(Job::Card(oid.to_owned()));
    }

    /// Asks the worker to look through `job`'s span once more, after the cards asked for before it:
    /// a blame that left commits uncarded could not tell which of them to look through.
    pub(super) fn ask_look_again(&self, job: BlameJob) {
        self.push(Job::LookAgain(job));
    }

    /// Holds `text` as the commit `oid`'s.
    pub(super) fn put_text(&self, oid: &str, text: CardText) {
        self.lock().texts.put(oid, text);
    }

    /// Keeps the store's counts of cards and cached blames for the status.
    pub(super) fn set_counts(&self, cards: u64, cached_blames: u64) {
        let mut state = self.lock();
        state.report.cards = cards;
        state.report.cached_blames = cached_blames;
    }

    /// Remembers that `job`'s span failed, or that the store refused it, and counts a failure.
    pub(super) fn remember_failed(&self, job: &BlameJob) {
        let mut state = self.lock();
        state.failed.insert(span_key(job), ());
        state.report.failed_blames = state.report.failed_blames.saturating_add(1);
    }

    /// Whether `job`'s span failed before.
    pub(super) fn has_failed(&self, job: &BlameJob) -> bool {
        self.lock().failed.get(&span_key(job)).is_some()
    }

    /// Remembers that `job`'s file on disk is not the file at `head`.
    pub(super) fn remember_unblameable(&self, job: &BlameJob, head: &str) {
        self.lock()
            .unblameable
            .insert(span_key(job), head.to_owned());
    }

    /// Whether `job`'s file on disk was found not to be the file at `head`.
    pub(super) fn is_unblameable_at(&self, job: &BlameJob, head: &str) -> bool {
        self.lock()
            .unblameable
            .get(&span_key(job))
            .is_some_and(|seen| seen == head)
    }

    /// Remembers that the commit `oid` could not be carded.
    pub(super) fn remember_uncardable(&self, oid: &str) {
        self.lock().uncardable.insert(oid.to_owned(), ());
    }

    /// Whether the commit `oid` could not be carded before.
    pub(super) fn is_uncardable(&self, oid: &str) -> bool {
        self.lock().uncardable.get(&oid.to_owned()).is_some()
    }

    /// Keeps why git is not run in the repository, for the daemon's life.
    pub(super) fn refuse(&self, why: &str) {
        self.lock().report.refused = Some(why.to_owned());
    }

    /// Adds `job` last, when the worker takes jobs: whether it was added.
    fn push(&self, job: Job) -> bool {
        let mut state = self.lock();
        if !state.is_on {
            return false;
        }
        let pushed = state.queue.push(job);
        drop(state);
        match pushed {
            Pushed::Waiting => false,
            Pushed::DroppedOldest => {
                tracing::debug!("a history job was dropped: too many waited");
                self.arrived.notify_all();
                true
            }
            Pushed::Queued => {
                self.arrived.notify_all();
                true
            }
        }
    }

    /// The state, locked. A panic while it was held leaves it consistent (each change is one
    /// step), so a poisoned lock is taken as it is.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{ContentHash, RepoPath};

    use super::*;

    /// A blame of lines 1 to 3 of `a.py`.
    fn blame() -> Result<BlameJob, cortex_core::PathError> {
        Ok(BlameJob {
            path: RepoPath::new("a.py")?,
            file_hash: ContentHash::of(b"a"),
            first_line: 1,
            last_line: 3,
        })
    }

    /// Before the worker starts, and after it stops, nothing is taken and the status says
    /// nothing; while it runs, a blame asked for waits (once), reading a text not held asks for
    /// nothing, and asking for it asks for a card, but not of a commit that could not be read.
    #[test]
    fn jobs_are_taken_only_while_the_worker_runs() -> Result<(), cortex_core::PathError> {
        let jobs = Jobs::default();
        assert!(!jobs.ask_blame(blame()?));
        jobs.ask_text(&"a".repeat(40));
        assert_eq!(jobs.report(), None);
        jobs.turn_on();
        assert!(jobs.ask_blame(blame()?));
        assert!(!jobs.ask_blame(blame()?));
        assert_eq!(jobs.text(&"a".repeat(40)), None);
        jobs.ask_text(&"a".repeat(40));
        jobs.remember_uncardable(&"b".repeat(40));
        jobs.ask_text(&"b".repeat(40));
        assert_eq!(jobs.report().map(|report| report.jobs_waiting), Some(2));
        assert_eq!(jobs.next_job(), Some(Job::Blame(blame()?)));
        assert_eq!(jobs.next_job(), Some(Job::Card("a".repeat(40))));
        jobs.stop();
        assert!(!jobs.ask_blame(blame()?));
        assert_eq!(jobs.next_job(), None);
        assert_eq!(jobs.report(), None);
        Ok(())
    }

    /// Worked by hand, at most 200 files a day: 150 counted leave room for 50; the same day
    /// another 50 leave none; the next day there is room for 200 again.
    #[test]
    fn warming_counts_what_it_queued_each_day() {
        let jobs = Jobs::default();
        let day = DAY_MS * 20_000;
        assert_eq!(jobs.warming_room(day + 1, 200), 200);
        jobs.count_warmed(day + 1, 150);
        assert_eq!(jobs.warming_room(day + 2, 200), 50);
        jobs.count_warmed(day + 2, 50);
        assert_eq!(jobs.warming_room(day + DAY_MS - 1, 200), 0);
        assert_eq!(jobs.warming_room(day + DAY_MS, 200), 200);
    }

    /// A span remembered as failed is remembered at any HEAD and counted; one whose file was not
    /// the file at a HEAD is so at that HEAD only, though warming skips it at any.
    #[test]
    fn spans_are_remembered_as_they_failed() -> Result<(), cortex_core::PathError> {
        let jobs = Jobs::default();
        jobs.turn_on();
        let failed = blame()?;
        let changed = BlameJob {
            last_line: 9,
            ..blame()?
        };
        jobs.remember_failed(&failed);
        jobs.remember_unblameable(&changed, "h1");
        assert!(jobs.has_failed(&failed) && !jobs.has_failed(&changed));
        assert!(jobs.is_unblameable_at(&changed, "h1"));
        assert!(!jobs.is_unblameable_at(&changed, "h2"));
        assert!(jobs.is_remembered(&failed) && jobs.is_remembered(&changed));
        assert_eq!(jobs.report().map(|report| report.failed_blames), Some(1));
        Ok(())
    }
}
