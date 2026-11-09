//! A session's position on a node of the map: what it read, edited or claimed, and until when that
//! holds. A read holds for 10 minutes; an edit 30 minutes after the session's last edit of the
//! node, or 5 minutes after the task that made it ended, whichever comes first; a claim 2 hours.
//! A session is live while it has had an event in the last 15 minutes.

use cortex_core::CoordinationConfig;
pub use cortex_core::PositionMode;

/// How long each kind of position holds, and how recent an event keeps a session live, in
/// milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long a read holds after it.
    pub read_ms: i64,
    /// How long an edit holds after the session's last edit of the node.
    pub edit_ms: i64,
    /// How long an edit holds after its task ended.
    pub edit_after_task_ms: i64,
    /// How long a claim holds after it was made.
    pub claim_ms: i64,
    /// How recent a session's last event must be for it to be live.
    pub live_ms: i64,
    /// How close two sessions' edits of one file must be for both to be told first.
    pub same_file_ms: i64,
    /// How long a session must have been quiet for its handoff to be given though it never
    /// ended.
    pub handoff_idle_ms: i64,
}

impl Timing {
    /// The holds `config` sets, in milliseconds.
    #[must_use]
    pub fn of(config: &CoordinationConfig) -> Self {
        let ms = |minutes: u32| i64::from(minutes) * MINUTE_MS;
        Self {
            read_ms: ms(config.read_minutes),
            edit_ms: ms(config.edit_minutes),
            edit_after_task_ms: ms(config.edit_after_task_minutes),
            claim_ms: ms(config.claim_minutes),
            live_ms: ms(config.live_minutes),
            same_file_ms: ms(config.same_file_minutes),
            handoff_idle_ms: ms(config.handoff_idle_minutes),
        }
    }

    /// The longest any position holds: how far back a position may have been taken and still
    /// hold.
    #[must_use]
    pub fn longest_ms(&self) -> i64 {
        self.read_ms.max(self.edit_ms).max(self.claim_ms)
    }
}

/// A minute in milliseconds.
const MINUTE_MS: i64 = 60_000;

/// One session's position on one node, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position<S, N> {
    /// The session.
    pub session: S,
    /// The node.
    pub node: N,
    /// What the session did to it: of two, the one that holds longer is kept (see
    /// [`Position::longer_of`]).
    pub mode: PositionMode,
    /// When the session last did it, in UTC milliseconds.
    pub last_ms: i64,
    /// When the task that made an edit ended, if it has.
    pub task_ended_ms: Option<i64>,
}

impl<S, N> Position<S, N> {
    /// Whether the position still holds at `now_ms`.
    #[must_use]
    pub fn holds_at(&self, now_ms: i64, timing: &Timing) -> bool {
        now_ms < self.until(timing)
    }

    /// Of the position `kept` and the session's new one on the same node, the one that holds
    /// longer: so a read after an edit does not cut the edit short, and an edit that no longer
    /// holds does not hide a later read. A tie keeps the new one, its time the later. A claim still
    /// holding when the new one is taken is kept whatever the new one is: a claim is made on
    /// purpose, and an edit that would seem to hold longer only does until its task ends.
    #[must_use]
    pub fn longer_of(kept: Option<Self>, new: Self, timing: &Timing) -> Self {
        match kept {
            Some(kept)
                if kept.mode == PositionMode::Claim && kept.holds_at(new.last_ms, timing) =>
            {
                kept
            }
            Some(kept) if kept.until(timing) > new.until(timing) => kept,
            _ => new,
        }
    }

    /// When the position stops holding, in UTC milliseconds.
    fn until(&self, timing: &Timing) -> i64 {
        match self.mode {
            PositionMode::Read => self.last_ms + timing.read_ms,
            PositionMode::Claim => self.last_ms + timing.claim_ms,
            PositionMode::Edit => {
                let after_edit = self.last_ms + timing.edit_ms;
                self.task_ended_ms.map_or(after_edit, |ended| {
                    after_edit.min(ended + timing.edit_after_task_ms)
                })
            }
        }
    }
}

/// Whether a session whose last event was at `last_event_ms` is live at `now_ms`.
#[must_use]
pub fn is_live(last_event_ms: i64, now_ms: i64, timing: &Timing) -> bool {
    now_ms - last_event_ms < timing.live_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A position of `mode` last at minute 0, its task ended at `task_ended` minutes if any.
    fn at_zero(mode: PositionMode, task_ended: Option<i64>) -> Position<&'static str, u32> {
        Position {
            session: "a",
            node: 1,
            mode,
            last_ms: 0,
            task_ended_ms: task_ended.map(|minutes| minutes * MINUTE_MS),
        }
    }

    /// Worked by hand from the rules: a read holds through minute 9 and not at 10; an edit through
    /// 29 and not at 30; an edit whose task ended at minute 2 until 7 (5 after), and one whose
    /// task ended at 40 until 30 (30 after the edit comes first); a claim through 119 and not at
    /// 120. A session with an event 14 minutes ago is live, one 15 minutes ago is not.
    #[test]
    fn each_position_holds_as_long_as_its_mode_says() {
        let timing = Timing::of(&CoordinationConfig::default());
        let holds = |position: &Position<&str, u32>, minute: i64| {
            position.holds_at(minute * MINUTE_MS, &timing)
        };
        let read = at_zero(PositionMode::Read, None);
        assert!(holds(&read, 9) && !holds(&read, 10));
        let edit = at_zero(PositionMode::Edit, None);
        assert!(holds(&edit, 29) && !holds(&edit, 30));
        let ended_soon = at_zero(PositionMode::Edit, Some(2));
        assert!(holds(&ended_soon, 6) && !holds(&ended_soon, 7));
        let ended_late = at_zero(PositionMode::Edit, Some(40));
        assert!(holds(&ended_late, 29) && !holds(&ended_late, 30));
        let claim = at_zero(PositionMode::Claim, None);
        assert!(holds(&claim, 119) && !holds(&claim, 120));
        assert!(is_live(0, 14 * MINUTE_MS, &timing));
        assert!(!is_live(0, 15 * MINUTE_MS, &timing));
    }

    /// A position of `mode` last at `minute`.
    fn at(mode: PositionMode, minute: i64) -> Position<&'static str, u32> {
        Position {
            last_ms: minute * MINUTE_MS,
            ..at_zero(mode, None)
        }
    }

    /// Worked by hand: an edit at minute 0 holds until 30, a read at 5 until 15: the edit is kept.
    /// A read at 25 holds until 35, past the edit's 30: the read is kept. A claim at 0 holds until
    /// 120, an edit at 60 until 90: the claim is kept; so is it against an edit at 95, which would
    /// hold until 125 while its task is open; an edit at 120, when the claim no longer holds, is
    /// kept. With nothing kept, the new one; two reads, the later.
    #[test]
    fn the_position_that_holds_longer_is_kept() {
        let timing = Timing::of(&CoordinationConfig::default());
        let longer = |kept, new| Position::longer_of(Some(kept), new, &timing);
        let edit = at(PositionMode::Edit, 0);
        assert_eq!(longer(edit.clone(), at(PositionMode::Read, 5)), edit);
        let late_read = at(PositionMode::Read, 25);
        assert_eq!(longer(edit, late_read.clone()), late_read);
        let claim = at(PositionMode::Claim, 0);
        assert_eq!(longer(claim.clone(), at(PositionMode::Edit, 60)), claim);
        assert_eq!(longer(claim.clone(), at(PositionMode::Edit, 95)), claim);
        let after = at(PositionMode::Edit, 120);
        assert_eq!(longer(claim, after.clone()), after);
        let read = at(PositionMode::Read, 3);
        assert_eq!(Position::longer_of(None, read.clone(), &timing), read);
        let later = at(PositionMode::Read, 4);
        assert_eq!(longer(read, later.clone()), later);
    }
}
