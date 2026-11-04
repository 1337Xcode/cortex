//! What a task's end teaches, written as the task ends: what became of it, the interest its work
//! earned, the links between what it used together and the record of what it did. Also whether
//! the task before it in the same session was put back, which only this end can show.
//!
//! What each edited file holds now is read from the store, as the index last saw it, and counts
//! only when the index read the file after the task last changed it. When the index has not caught
//! up by the task's end, nothing shows the file back as it was, so no revert by contents is seen
//! then; the next task's end looks again, once the index has. A git restore needs no reading and
//! is seen at once.
//!
//! A task found put back only at the next end loses what its own end gave its links, as written
//! within their bounds and brought forward to the next end by the links' decay, as well as a
//! reverted task's share: the links are then what they would be had the task's work drawn nothing,
//! less that share.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Config, FileContent, Identifier, RepoPath, TaskOutcome, Ulid};
use cortex_store::{Queries, StoreError, WriteTx};

use crate::archetype::{MAX_CLUSTERED_TASKS, archetypes_of};
use crate::decay::decayed;
use crate::episode::episode_of;
use crate::interest::{add_interest, interest_of};
use crate::labels::label_injections;
use crate::links::{EarnedLinks, Linking, add_links, links_of};
use crate::outcome::{FileNow, judge, reverted_after};
use crate::task_state::TaskState;

/// Most sessions whose last ended task is kept for the look the session's next task's end takes
/// at it: the sessions at work in one repository at once, with room to spare. The session whose
/// last task ended longest ago is let go first.
pub const MAX_SESSIONS_KEPT: usize = 32;

/// What one task's end found.
#[derive(Debug, Clone, PartialEq)]
pub struct Learned {
    /// What became of the task.
    pub outcome: TaskOutcome,
    /// The task before it in its session, when this end showed that task was put back.
    pub reverted_before: Option<Ulid>,
    /// What the task's end changed the links by, pair by pair, as written within the bounds a link
    /// is held within: what a later end takes back should it find the task put back. Empty for a
    /// task that failed or was left; for a task put back at its own end, what weakened the links.
    pub linked: EarnedLinks,
}

/// A session's last ended task, as the learner keeps it for the session's next end.
#[derive(Debug)]
struct Kept {
    /// When the task ended.
    ended_at_ms: i64,
    /// The task's number in the repository.
    seq: u64,
    /// What the task did.
    state: TaskState,
    /// What its end added to the links.
    linked: EarnedLinks,
}

/// What the ends of tasks learn, holding each session's last ended task until its next one ends.
///
/// Learning writes only through the transaction it is given; what the learner itself remembers
/// changes only when the caller says the transaction was committed ([`Learner::keep`]), so a
/// transaction rolled back leaves the learner as it was.
#[derive(Debug, Default)]
pub struct Learner {
    /// Each session's last ended task.
    last: BTreeMap<Identifier, Kept>,
}

impl Learner {
    /// A learner that has seen no task end.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Learns from `state`, a task that has just ended and whose row says so, in `transaction`,
    /// with the settings of `config`: judges it and stores the judgement, adds the interest its
    /// work earned, changes the links between what it used, labels the injections its prompt got,
    /// and writes its record. When the task
    /// before it in the same session, kept by [`Learner::keep`], now shows as put back, that task
    /// is judged reverted, the links lose what its end gave them and the links between the files it
    /// edited are weakened, and its record says so. `None` when the store holds no ended row for
    /// the task. Once the caller has committed, it passes the state and what was learned to
    /// [`Learner::keep`].
    ///
    /// # Errors
    ///
    /// The store's errors; the transaction then holds part of the work and must not be committed.
    pub fn learn(
        &self,
        transaction: &mut WriteTx<'_>,
        state: &TaskState,
        config: &Config,
    ) -> Result<Option<Learned>, StoreError> {
        let Some(row) = transaction.queries().task(state.task_id())? else {
            return Ok(None);
        };
        let Some(ended_at_ms) = row.ended_at_ms else {
            return Ok(None);
        };
        let previous = self.last.get(&row.record.session_id);
        let paths: BTreeSet<RepoPath> = previous
            .iter()
            .flat_map(|earlier| earlier.state.before().keys())
            .chain(state.before().keys())
            .cloned()
            .collect();
        let now = files_now(transaction.queries(), &paths)?;
        let judgement = judge(state, &now);
        transaction.set_task_outcome(
            state.task_id(),
            judgement.outcome,
            judgement.evidence.as_deref(),
        )?;
        let earned = interest_of(transaction.queries(), state)?;
        add_interest(transaction, (row.seq, config.interest_decay), &earned)?;
        let linking = linking(config, row.seq, judgement.outcome);
        let links = links_of(transaction.queries(), state, &linking)?;
        let linked = add_links(transaction, &linking, &links)?;
        label_injections(transaction, state)?;
        let record = episode_of(
            transaction.queries(),
            state,
            (judgement.outcome, ended_at_ms),
        )?;
        transaction.replace_episode(&record)?;
        let reverted_before = match previous {
            Some(earlier) => revert_before(transaction, (earlier, state), &now, (config, row.seq))?,
            None => None,
        };
        Ok(Some(Learned {
            outcome: judgement.outcome,
            reverted_before,
            linked,
        }))
    }

    /// Keeps `state`, a task whose learning the caller has committed, with `linked`, what its end
    /// added to the links ([`Learned::linked`]), as its session's last ended task, read with
    /// `queries`, for the look the session's next task's end takes at it. When more than
    /// [`MAX_SESSIONS_KEPT`] sessions are kept, the one whose last task ended longest ago is let
    /// go, the smaller task number first on a tie. A task the store holds no ended row for is not
    /// kept.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub fn keep(
        &mut self,
        queries: Queries<'_>,
        state: TaskState,
        linked: EarnedLinks,
    ) -> Result<(), StoreError> {
        let Some(row) = queries.task(state.task_id())? else {
            return Ok(());
        };
        let Some(ended_at_ms) = row.ended_at_ms else {
            return Ok(());
        };
        let kept = Kept {
            ended_at_ms,
            seq: row.seq,
            state,
            linked,
        };
        self.last.insert(row.record.session_id, kept);
        // INVARIANT: budgeted (at most MAX_SESSIONS_KEPT sessions' last tasks are held)
        while self.last.len() > MAX_SESSIONS_KEPT {
            let oldest = self
                .last
                .iter()
                .min_by_key(|(_, kept)| (kept.ended_at_ms, kept.seq))
                .map(|(session, _)| session.clone());
            let Some(oldest) = oldest else {
                break;
            };
            self.last.remove(&oldest);
        }
        Ok(())
    }
}

/// Clusters the newest task records again and writes the clusters in `transaction`, as of
/// `at_ms`.
///
/// # Errors
///
/// The store's errors.
pub fn cluster_tasks(transaction: &mut WriteTx<'_>, at_ms: i64) -> Result<(), StoreError> {
    let clusters = archetypes_of(transaction.queries(), (MAX_CLUSTERED_TASKS, at_ms))?;
    transaction.replace_archetypes(&clusters)
}

/// What a task's end brings to the links, for the task numbered `seq` that ended as `outcome`.
fn linking(config: &Config, seq: u64, outcome: TaskOutcome) -> Linking {
    Linking {
        seq,
        outcome,
        eta: config.learning_eta,
        link_decay: config.link_decay,
        interest_decay: config.interest_decay,
    }
}

/// What each of `paths` holds now, as the store holds it, with when the index wrote that: a file
/// the store holds no row for is left out, since nothing says when it went or whether the index
/// has seen it come back.
fn files_now(
    queries: Queries<'_>,
    paths: &BTreeSet<RepoPath>,
) -> Result<BTreeMap<RepoPath, FileNow>, StoreError> {
    let mut now = BTreeMap::new();
    for path in paths {
        if let Some(row) = queries.file(path)? {
            let file_now = FileNow {
                content: FileContent::of_stored(row.file.content_hash),
                read_at_ms: row.file.indexed_at_ms,
            };
            now.insert(path.clone(), file_now);
        }
    }
    Ok(now)
}

/// Whether the task `earlier` was put back, as the end of `next`, the task after it in the same
/// session, shows with `now`; when it was, and was not judged so already, judges it reverted in
/// `transaction`, takes from the links what its end gave them and weakens the links between the
/// files it edited, both as of task `seq`, and writes its record's outcome again. The task's id
/// when it was.
fn revert_before(
    transaction: &mut WriteTx<'_>,
    (earlier, next): (&Kept, &TaskState),
    now: &BTreeMap<RepoPath, FileNow>,
    (config, seq): (&Config, u64),
) -> Result<Option<Ulid>, StoreError> {
    let Kept {
        state: earlier,
        seq: earlier_seq,
        linked,
        ..
    } = earlier;
    let task_id = earlier.task_id();
    let was = transaction
        .queries()
        .task(task_id)?
        .and_then(|row| row.outcome);
    if was == Some(TaskOutcome::Reverted) {
        return Ok(None);
    }
    let Some(judgement) = reverted_after(earlier, next, now) else {
        return Ok(None);
    };
    transaction.set_task_outcome(task_id, judgement.outcome, judgement.evidence.as_deref())?;
    let linking = linking(config, seq, TaskOutcome::Reverted);
    // What the earlier end gave a link has decayed since as the link has, so taking it back
    // brought forward the same way leaves the link as it would be without that task.
    let mut links: EarnedLinks = linked
        .iter()
        .map(|(pair, add)| (*pair, -decayed(*add, *earlier_seq, seq, config.link_decay)))
        .collect();
    for (pair, weaken) in links_of(transaction.queries(), earlier, &linking)? {
        *links.entry(pair).or_insert(0.0) += weaken;
    }
    add_links(transaction, &linking, &links)?;
    if let Some(mut record) = transaction.queries().episode_of_task(task_id)? {
        record.outcome = TaskOutcome::Reverted;
        transaction.replace_episode(&record)?;
    }
    Ok(Some(task_id))
}
