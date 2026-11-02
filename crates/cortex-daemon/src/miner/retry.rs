//! How long failures hold the miner's next try back: 2 s after one, twice as long after each
//! further one in a row, never more than an hour, so that a failure that stays costs a read of the
//! repository once an hour at most and a failure that passes costs little. A failure met after
//! HEAD was read holds back a mining of that HEAD only: a move of HEAD is tried at once.

use std::time::{Duration, Instant};

use super::HEAD_EVERY;

/// The longest a failure holds the next try back: a failure that stays costs a read of the
/// repository once an hour at most, and a cause that went away is found within the hour.
const MAX_PAUSE: Duration = Duration::from_secs(60 * 60);

/// The failures in a row, and how long they hold the next try back.
#[derive(Debug, Default)]
pub(super) struct Retry {
    /// The failures in a row.
    failures: u32,
    /// HEAD's commit at the latest failure, when it was read.
    head: Option<String>,
    /// When the pause after the latest failure ends.
    until: Option<Instant>,
}

impl Retry {
    /// Counts a failure met at `now`, HEAD's commit being `head` when it was read, and pauses for
    /// [`pause_after`] the failures in a row.
    pub(super) fn failed(&mut self, head: Option<String>, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.head = head;
        self.until = now.checked_add(pause_after(self.failures));
    }

    /// Whether the latest failure holds back a look at `now`, HEAD's commit being `head` once read:
    /// while its pause lasts, every look when it came before HEAD was read, and otherwise a mining
    /// of the same HEAD.
    pub(super) fn holds_back(&self, head: Option<&str>, now: Instant) -> bool {
        let is_pausing = self.until.is_some_and(|until| now < until);
        is_pausing && (self.head.is_none() || self.head.as_deref() == head)
    }
}

/// The pause after `failures` failures in a row: [`HEAD_EVERY`] after the first, doubled after
/// each further one, and never more than [`MAX_PAUSE`].
fn pause_after(failures: u32) -> Duration {
    2_u32
        .checked_pow(failures.saturating_sub(1))
        .and_then(|factor| HEAD_EVERY.checked_mul(factor))
        .map_or(MAX_PAUSE, |pause| pause.min(MAX_PAUSE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure pauses the next try for 2 s, each further one in a row for twice as long, never
    /// more than an hour: 2,048 s after the eleventh, the cap after the twelfth (4,096 s). A failure
    /// met before HEAD was read holds back every look while it pauses; one met at a HEAD holds
    /// back that HEAD only, and nothing once the pause is over.
    #[test]
    fn failures_hold_the_next_try_back_longer_each_time() {
        let pauses: Vec<u64> = [1, 2, 3, 11, 12, u32::MAX]
            .into_iter()
            .map(|failures| pause_after(failures).as_secs())
            .collect();
        assert_eq!(pauses, [2, 4, 8, 2_048, 3_600, 3_600]);
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let over = now + Duration::from_secs(2);
        let mut blind = Retry::default();
        assert!(!blind.holds_back(None, now));
        blind.failed(None, now);
        assert!(blind.holds_back(None, later) && blind.holds_back(Some("h"), later));
        assert!(!blind.holds_back(None, over));
        let mut at_head = Retry::default();
        at_head.failed(Some("h".to_owned()), now);
        assert!(at_head.holds_back(Some("h"), later));
        assert!(!at_head.holds_back(None, later) && !at_head.holds_back(Some("g"), later));
        assert!(!at_head.holds_back(Some("h"), over));
    }
}
