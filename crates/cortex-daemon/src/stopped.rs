//! The tasks strict mode gave a reason to go on when their agent stopped: each is given one once.
//! Claimed under one lock, so two stops of one task answered at once give one reason.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use cortex_core::Ulid;

/// How many tasks are kept, the one claimed longest ago dropped first: tasks run one at a time in
/// a session, so a few dozen cover every session working at once. A task dropped could be given a
/// second reason, which a task that long-lived has earned.
const MAX_TASKS: usize = 64;

/// The tasks given their reason.
#[derive(Debug, Default)]
pub(crate) struct Stopped {
    /// Each task, the one claimed longest ago first.
    tasks: Mutex<VecDeque<Ulid>>,
}

impl Stopped {
    /// Whether task `task_id` is to be given its reason: true the first time, which is kept in the
    /// same step.
    pub(crate) fn claim(&self, task_id: Ulid) -> bool {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        if tasks.contains(&task_id) {
            return false;
        }
        if tasks.len() == MAX_TASKS {
            tasks.pop_front();
        }
        tasks.push_back(task_id);
        true
    }

    /// Gives back task `task_id`'s claim: its reason never reached the agent.
    pub(crate) fn release(&self, task_id: Ulid) {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.retain(|task| *task != task_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TASKS, Stopped};
    use cortex_core::Ulid;

    /// A task is claimed once, another apart; once [`MAX_TASKS`] more were claimed, the first is
    /// forgotten.
    #[test]
    fn a_task_is_claimed_once() {
        let stopped = Stopped::default();
        let first = Ulid::from_seed(1, b"task");
        assert!(stopped.claim(first));
        assert!(!stopped.claim(first));
        assert!(stopped.claim(Ulid::from_seed(2, b"task")));
        for (_, seed) in (0..MAX_TASKS).zip(100_u64..) {
            assert!(stopped.claim(Ulid::from_seed(seed, b"task")));
        }
        assert!(stopped.claim(first));
        assert!(!stopped.claim(first));
    }
}
