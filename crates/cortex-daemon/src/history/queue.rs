//! The jobs waiting for the history worker: at most [`MAX_JOBS`], oldest first. A job asked for
//! while the same job waits is not added twice; one that finds the queue full makes room by
//! dropping the oldest, which the next prompt that needs it asks for again.

use std::collections::VecDeque;

use cortex_core::{ContentHash, RepoPath};

/// The most jobs that wait: one prompt's work order asks for a blame of each span it shows (two by
/// default, five at most) and a card for each commit whose text is not held, and upkeep's warming
/// adds at most [`super::warm::MAX_WARMED_A_PASS`] at a time, so 64 holds several prompts' worth
/// while a slow blame runs.
pub(crate) const MAX_JOBS: usize = 64;

/// A span of a file as HEAD holds it, to be blamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlameJob {
    /// The file's path.
    pub(crate) path: RepoPath,
    /// The hash of the file's content at HEAD, which the blame is kept under. A file whose content
    /// at HEAD hashes otherwise when the job runs is not blamed.
    pub(crate) file_hash: ContentHash,
    /// The span's first line, counted from 1.
    pub(crate) first_line: u32,
    /// The span's last line, inclusive.
    pub(crate) last_line: u32,
}

/// What the history worker is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Job {
    /// Blame a span, and card every commit the blame names that has no card.
    Blame(BlameJob),
    /// Read the commit with this id again: its card when it has none, and the two lines of its
    /// message worth showing, which the text cache lost.
    Card(String),
    /// Blame the span again, once, now that the commits a first blame left uncarded are carded,
    /// so that the ones not worth showing are looked through.
    LookAgain(BlameJob),
}

/// What became of a job asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pushed {
    /// It waits, last.
    Queued,
    /// The same job waits already.
    Waiting,
    /// It waits, last, and the oldest job was dropped to make room.
    DroppedOldest,
}

/// The jobs waiting, oldest first.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    /// The jobs, oldest first, at most [`MAX_JOBS`].
    jobs: VecDeque<Job>,
}

impl Queue {
    /// Adds `job` last, unless the same job waits; when [`MAX_JOBS`] wait, the oldest is dropped.
    pub(crate) fn push(&mut self, job: Job) -> Pushed {
        if self.jobs.contains(&job) {
            return Pushed::Waiting;
        }
        let pushed = if self.jobs.len() >= MAX_JOBS {
            self.jobs.pop_front();
            Pushed::DroppedOldest
        } else {
            Pushed::Queued
        };
        self.jobs.push_back(job);
        pushed
    }

    /// Takes the oldest job.
    pub(crate) fn pop(&mut self) -> Option<Job> {
        self.jobs.pop_front()
    }

    /// How many jobs wait.
    pub(crate) fn len(&self) -> usize {
        self.jobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card job for the commit numbered `number`, an id of 40 digits.
    fn card(number: usize) -> Job {
        Job::Card(format!("{number:040}"))
    }

    /// Worked by hand: 70 jobs pushed into a queue of 64. The first 64 queue; each of the last six
    /// drops the oldest, so jobs 0 to 5 are gone and 6 to 69 wait, in the order they came.
    #[test]
    fn a_full_queue_drops_the_oldest() {
        let mut queue = Queue::default();
        let pushed: Vec<Pushed> = (0..70).map(|number| queue.push(card(number))).collect();
        assert_eq!(pushed[..64], [Pushed::Queued; 64]);
        assert_eq!(pushed[64..], [Pushed::DroppedOldest; 6]);
        assert_eq!(queue.len(), 64);
        let waiting: Vec<Job> = std::iter::from_fn(|| queue.pop()).collect();
        assert_eq!(waiting, (6..70).map(card).collect::<Vec<_>>());
        assert_eq!(queue.len(), 0);
    }

    /// A job asked for again while it waits is not added again, and keeps its place; once taken,
    /// it can be asked for again.
    #[test]
    fn a_job_waits_once() {
        let mut queue = Queue::default();
        assert_eq!(queue.push(card(1)), Pushed::Queued);
        assert_eq!(queue.push(card(2)), Pushed::Queued);
        assert_eq!(queue.push(card(1)), Pushed::Waiting);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop(), Some(card(1)));
        assert_eq!(queue.push(card(1)), Pushed::Queued);
        assert_eq!(queue.pop(), Some(card(2)));
        assert_eq!(queue.pop(), Some(card(1)));
        assert_eq!(queue.pop(), None);
    }
}
