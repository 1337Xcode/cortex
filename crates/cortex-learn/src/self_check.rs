//! The self-check: whether the work orders the gate lets through are used often enough. After a
//! task's delivered injections are labelled, the precision of the latest [`SELF_CHECK_WINDOW`]
//! delivered, labelled injections is taken; below [`SELF_CHECK_FLOOR`] the gate's bar is raised
//! one [`RAISE_STEP`], at most [`MAX_RAISE_STEPS`] steps in all, and a raise needs that many more
//! labelled since the last, so that one bad stretch raises the bar once. The bar is never lowered:
//! once work orders were seen to go unused at that bar, a stretch of better ones does not show the
//! bar was too high.

use cortex_store::{SelfCheckRecord, StoreError, WriteTx};

/// How many of the latest delivered, labelled injections the precision is taken over.
pub const SELF_CHECK_WINDOW: usize = 100;

/// The precision below which the gate's bar is raised.
pub const SELF_CHECK_FLOOR: f64 = 0.6;

/// What one raise adds to the gate's bar.
pub const RAISE_STEP: f64 = 0.05;

/// The most steps the bar is raised: 0.2 above where it started.
pub const MAX_RAISE_STEPS: u32 = 4;

/// What the self-check finds after `labelled` delivered injections were labelled in all, `recent`
/// the labels of the latest of them, the newest first, at most [`SELF_CHECK_WINDOW`], and
/// `previous` what it found before.
#[must_use]
pub fn self_checked(previous: SelfCheckRecord, labelled: u64, recent: &[bool]) -> SelfCheckRecord {
    let window = recent.get(..SELF_CHECK_WINDOW).unwrap_or(recent);
    let used = window.iter().filter(|label| **label).count();
    let mut found = SelfCheckRecord {
        recent_labelled: u32::try_from(window.len()).unwrap_or(u32::MAX),
        recent_used: u32::try_from(used).unwrap_or(u32::MAX),
        ..previous
    };
    let since_raise = labelled.saturating_sub(previous.raised_at);
    // The labels counted and the window come from the same rows, so a hundred labelled since the
    // last raise, or since the start, is a full window.
    let is_due = usize::try_from(since_raise).is_ok_and(|since| since >= SELF_CHECK_WINDOW);
    let is_below = recent_precision(&found).is_some_and(|precision| precision < SELF_CHECK_FLOOR);
    if is_due && is_below && previous.raise_steps < MAX_RAISE_STEPS {
        found.raise_steps = previous.raise_steps + 1;
        found.raised_at = labelled;
    }
    found
}

/// What the gate's bar is raised by, as the gate takes it.
#[must_use]
pub fn raise_of(record: &SelfCheckRecord) -> f64 {
    f64::from(record.raise_steps) * RAISE_STEP
}

/// The share of the latest delivered, labelled injections that were used; none before any was
/// labelled.
#[must_use]
pub fn recent_precision(record: &SelfCheckRecord) -> Option<f64> {
    (record.recent_labelled > 0)
        .then(|| f64::from(record.recent_used) / f64::from(record.recent_labelled))
}

/// Runs the self-check in `transaction` over the labels the store holds, keeping what it finds.
///
/// # Errors
///
/// The store's errors.
pub fn check_self(transaction: &mut WriteTx<'_>) -> Result<SelfCheckRecord, StoreError> {
    let queries = transaction.queries();
    let found = self_checked(
        queries.self_check()?,
        queries.delivered_labelled()?,
        &queries.delivered_labels(SELF_CHECK_WINDOW)?,
    );
    transaction.set_self_check(&found)?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count` labels, the first `used` of them used.
    fn labels(count: usize, used: usize) -> Vec<bool> {
        (0..count).map(|at| at < used).collect()
    }

    /// Worked by hand. With 99 labelled, fewer than a window since the start: nothing is raised,
    /// whatever the labels (0 of 99 used). At 100 with 59 used, 0.59 is below 0.6: one step, raised at 100. At
    /// 150, only 50 since: no raise, though 50 of 100 is below. At 200, 100 since and 50 of 100:
    /// a second step. 60 of 100 is the floor itself: no raise. Past four steps, no more.
    #[test]
    fn the_bar_is_raised_one_step_a_window() {
        let start = SelfCheckRecord::default();
        let early = self_checked(start, 99, &labels(99, 0));
        assert_eq!((early.raise_steps, early.raised_at), (0, 0));
        assert_eq!((early.recent_labelled, early.recent_used), (99, 0));
        let first = self_checked(start, 100, &labels(100, 59));
        assert_eq!(
            first,
            SelfCheckRecord {
                raise_steps: 1,
                raised_at: 100,
                recent_labelled: 100,
                recent_used: 59,
            }
        );
        let soon = self_checked(first, 150, &labels(100, 50));
        assert_eq!((soon.raise_steps, soon.raised_at), (1, 100));
        let second = self_checked(soon, 200, &labels(100, 50));
        assert_eq!((second.raise_steps, second.raised_at), (2, 200));
        let at_floor = self_checked(second, 300, &labels(100, 60));
        assert_eq!((at_floor.raise_steps, at_floor.raised_at), (2, 200));
        let most = SelfCheckRecord {
            raise_steps: 4,
            raised_at: 400,
            ..start
        };
        let held = self_checked(most, 600, &labels(100, 0));
        assert_eq!((held.raise_steps, held.raised_at), (4, 400));
    }

    /// The bar never comes down: four steps, then 100 of 100 used, stay four steps, 0.2. The
    /// precision is 100 of 100; before any label there is none, and labels past the window are
    /// not counted.
    #[test]
    fn what_the_gate_takes_follows_the_record() {
        let most = SelfCheckRecord {
            raise_steps: 4,
            raised_at: 400,
            ..SelfCheckRecord::default()
        };
        let after = self_checked(most, 800, &labels(100, 100));
        assert_eq!(after.raise_steps, 4);
        assert!((raise_of(&after) - 0.2).abs() < 1e-12);
        assert_eq!(recent_precision(&after), Some(1.0));
        assert_eq!(recent_precision(&SelfCheckRecord::default()), None);
        let long = self_checked(SelfCheckRecord::default(), 150, &labels(150, 150));
        assert_eq!((long.recent_labelled, long.recent_used), (100, 100));
    }
}
