//! What the daemon learns as tasks end. The recorder hands out each task's state as it ends; the
//! writer learns from each in a transaction of its own between the batches it records, recording
//! the hook events that wait between one task and the next, so that learning holds no event past
//! the durability bound. Once the tasks waiting are learned from, the clusters of tasks are written
//! again.

use cortex_core::Config;
use cortex_learn::{Learner, TaskState, cluster_tasks};
use cortex_store::Writer;

use crate::host::{count_failed, now_ms};
use crate::inbox::Inbox;

/// Most ended tasks waiting to be learned from. The writer learns between the batches it records,
/// so tasks wait only while an index or a mining holds it; a flood of prompts ends a task each,
/// and past this the oldest waiting are let go.
pub(crate) const MAX_TASKS_WAITING: usize = 256;

/// The tasks that ended and wait to be learned from, and what learns from them.
#[derive(Debug, Default)]
pub(crate) struct Learning {
    /// What learns from them, holding each session's last task for the next end's look.
    learner: Learner,
    /// The tasks waiting, in the order they ended.
    ended: Vec<TaskState>,
}

impl Learning {
    /// Adds the tasks that just ended, in order, letting the oldest waiting go past
    /// [`MAX_TASKS_WAITING`].
    pub(crate) fn wait(&mut self, ended: impl IntoIterator<Item = TaskState>) {
        self.ended.extend(ended);
        // INVARIANT: budgeted (at most MAX_TASKS_WAITING ended tasks wait to be learned from)
        let over = self.ended.len().saturating_sub(MAX_TASKS_WAITING);
        if over > 0 {
            tracing::warn!(
                dropped = over,
                "too many ended tasks waiting to be learned from"
            );
            self.ended.drain(..over);
        }
    }

    /// Whether no ended task waits.
    pub(crate) fn is_idle(&self) -> bool {
        self.ended.is_empty()
    }

    /// Learns from every task waiting, each in a transaction of `writer` of its own, with the
    /// settings of `config`, running `between` after each to record the hook events waiting and
    /// hand back the tasks they ended; then writes the clusters of tasks again. A task whose
    /// learning the store refuses is counted in `inbox` with the error and let go. Tasks that end
    /// while this runs wait for the next time. Whether any task waited: what the store holds of
    /// the tasks may then have changed.
    pub(crate) fn learn(
        &mut self,
        writer: &mut Writer,
        (inbox, config): (&Inbox, &Config),
        mut between: impl FnMut(&mut Writer) -> Vec<TaskState>,
    ) -> bool {
        if self.ended.is_empty() {
            return false;
        }
        let ended = std::mem::take(&mut self.ended);
        for state in ended {
            let learned = writer.transaction().and_then(|mut transaction| {
                let learned = self.learner.learn(&mut transaction, &state, config)?;
                transaction.commit()?;
                Ok(learned)
            });
            // The learner remembers the task only once its learning is committed.
            let kept = learned.and_then(|learned| match learned {
                Some(learned) => self.learner.keep(writer.queries(), state, learned.linked),
                None => Ok(()),
            });
            if let Err(error) = kept {
                count_failed(inbox, "what a task's end teaches", &error);
            }
            let more = between(writer);
            self.wait(more);
        }
        let clustered = writer.transaction().and_then(|mut transaction| {
            cluster_tasks(&mut transaction, now_ms())?;
            transaction.commit()
        });
        if let Err(error) = clustered {
            count_failed(inbox, "the clusters of tasks", &error);
        }
        true
    }
}
