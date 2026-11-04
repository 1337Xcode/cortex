//! What names a task, and whether it is one of the tasks held out from Cortex's help.
//!
//! A task is named by the prompt that starts it, so that the daemon, which answers the prompt
//! before its writer records it, and the writer, which records it, give it the same id. Whether a
//! task is held out follows from that id alone: a share of tasks, chosen by a hash of the id, gets
//! no injections, so that what agents do with Cortex can be compared with what they do without it.

use crate::Ulid;

/// The context the holdout hash is derived under, so that it is independent of every other hash of
/// a task's id. Changing it moves every task to another arm.
const HOLDOUT_CONTEXT: &str = "cortex 2026-11-04 holdout of a task by its id";

/// 2 to the 64th power: what a 64-bit number is divided by to make a fraction from 0 below 1.
const TWO_TO_64: f64 = 18_446_744_073_709_551_616.0;

/// The id of the task the prompt event `event_id`, which happened at `ts_ms`, starts: made at the
/// prompt's time from the prompt event's id, so that the same prompt always names the same task.
#[must_use]
pub fn task_id_of_prompt(event_id: Ulid, ts_ms: i64) -> Ulid {
    Ulid::from_seed(
        u64::try_from(ts_ms).unwrap_or(0),
        event_id.to_string().as_bytes(),
    )
}

/// Whether the task `task_id` is held out when `share` of tasks are: the first 8 bytes of a BLAKE3
/// hash of the id's text, derived under [`HOLDOUT_CONTEXT`] and read as a little-endian number over
/// 2^64, fall below `share`. A share of 0 or less holds none out, 1 or more every one.
#[must_use]
pub fn is_held_out(task_id: Ulid, share: f64) -> bool {
    if share <= 0.0 {
        return false;
    }
    if share >= 1.0 {
        return true;
    }
    let hash = blake3::derive_key(HOLDOUT_CONTEXT, task_id.to_string().as_bytes());
    let head = hash.first_chunk::<8>().copied().unwrap_or_default();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a fraction is compared with a share, which needs no more than 53 bits"
    )]
    let fraction = u64::from_le_bytes(head) as f64 / TWO_TO_64;
    fraction < share
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task's id is made at its prompt's time from the prompt event's id: the same event always
    /// names the same task, and two prompt events name two tasks.
    #[test]
    fn a_prompt_names_its_task() {
        let first = Ulid::from_seed(1_700_000_000_000, b"prompt one");
        let second = Ulid::from_seed(1_700_000_000_000, b"prompt two");
        let task = task_id_of_prompt(first, 1_700_000_000_123);
        assert_eq!(task, task_id_of_prompt(first, 1_700_000_000_123));
        assert_eq!(task.timestamp_ms(), 1_700_000_000_123);
        assert_ne!(task, task_id_of_prompt(second, 1_700_000_000_123));
        assert_ne!(task, first);
    }

    /// Ten thousand tasks, the ids of prompts a second apart.
    fn tasks() -> Vec<Ulid> {
        (0..10_000_u64)
            .map(|at| {
                let ts_ms = 1_700_000_000_000 + at * 1_000;
                let event = Ulid::from_seed(ts_ms, &at.to_le_bytes());
                task_id_of_prompt(event, i64::try_from(ts_ms).unwrap_or(0))
            })
            .collect()
    }

    /// A share of 0 holds no task out and a share of 1 every one; at the default of one in ten,
    /// 10,000 tasks hold out between 900 and 1,100 of them. Each task's choice is a coin of its
    /// own, so the count is binomial: a mean of 1,000 and a standard deviation of
    /// sqrt(10,000 x 0.1 x 0.9) = 30, and 900 and 1,100 lie 3.3 deviations out.
    #[test]
    fn a_share_of_tasks_is_held_out() {
        let tasks = tasks();
        assert!(tasks.iter().all(|task| !is_held_out(*task, 0.0)));
        assert!(tasks.iter().all(|task| is_held_out(*task, 1.0)));
        let held = tasks.iter().filter(|task| is_held_out(**task, 0.1)).count();
        assert!((900..=1_100).contains(&held), "{held} of 10,000 held out");
    }

    /// A task held out at one share is held out at every larger share: raising the share moves
    /// tasks into the holdout and never out of it.
    #[test]
    fn a_larger_share_keeps_every_task_a_smaller_one_held() {
        for task in tasks().into_iter().take(2_000) {
            for (smaller, larger) in [(0.05, 0.1), (0.1, 0.5), (0.5, 0.99)] {
                if is_held_out(task, smaller) {
                    assert!(
                        is_held_out(task, larger),
                        "{task} at {smaller} and {larger}"
                    );
                }
            }
        }
    }

    /// The rule as documented, worked through for one task with the hash written out here: BLAKE3
    /// derived under the holdout context from the id's text, its first 8 bytes read as a
    /// little-endian number, divided by 2^64, compared with the share.
    #[test]
    fn the_choice_follows_the_documented_hash() {
        for task in tasks().into_iter().take(500) {
            let hash = blake3::derive_key(
                "cortex 2026-11-04 holdout of a task by its id",
                task.to_string().as_bytes(),
            );
            let head: [u8; 8] = hash.first_chunk().copied().unwrap_or_default();
            #[expect(clippy::cast_precision_loss, reason = "a fraction for a comparison")]
            let fraction = u64::from_le_bytes(head) as f64 / 18_446_744_073_709_551_616.0;
            for share in [0.01, 0.1, 0.3, 0.9] {
                assert_eq!(
                    is_held_out(task, share),
                    fraction < share,
                    "{task} at {share}"
                );
            }
        }
    }
}
