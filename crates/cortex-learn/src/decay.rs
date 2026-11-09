//! What the tasks since take from a weight.
//!
//! Interest and links alike are kept as of the task they were last changed at, and fade by a
//! factor for each task in the repository since. Applying it where a weight is read, instead of
//! to every row at every task, means a task writes only what it touched.

/// The least a faded link weight or interest score keeps and still counts: below it, upkeep
/// removes the row, and a node with no interest has none.
pub const FADED_OUT_BELOW: f64 = 0.01;

/// Whether `weight`, written at task `last_seq`, has faded below [`FADED_OUT_BELOW`] by task `seq`
/// at `factor` for each task since.
#[must_use]
pub fn has_faded_out(weight: f64, last_seq: u64, seq: u64, factor: f64) -> bool {
    decayed(weight, last_seq, seq, factor) < FADED_OUT_BELOW
}

/// The `weight` a row held at task `last_seq`, brought forward to task `seq` by `factor` for each
/// task since. A weight written at a task yet to come is left as it is, and one left long enough
/// fades to nothing.
#[must_use]
pub fn decayed(weight: f64, last_seq: u64, seq: u64, factor: f64) -> f64 {
    let tasks = i32::try_from(seq.saturating_sub(last_seq)).unwrap_or(i32::MAX);
    weight * factor.powi(tasks)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A weight decays by the factor for each task since it was written, worked out by hand: 1.0
    /// written at task 1 and read at task 11 has decayed for 10 tasks, 0.98 to the tenth, which is
    /// 0.8170728068875467. A weight read at the task it was written at has not decayed at all, one
    /// read at an earlier task is left as it is, and one left for a million tasks is gone.
    #[test]
    fn a_weight_decays_by_the_factor_for_each_task_since() {
        assert!((decayed(1.0, 1, 11, 0.98) - 0.817_072_806_887_546_7).abs() < 1e-12);
        assert!((decayed(2.0, 11, 11, 0.98) - 2.0).abs() < 1e-12);
        assert!((decayed(2.0, 11, 4, 0.98) - 2.0).abs() < 1e-12);
        assert!(decayed(1.0, 1, 1_000_000, 0.98).abs() < 1e-12);
        // A link fades far more slowly: 0.995 to the tenth is 0.9511101304657719.
        assert!((decayed(1.0, 1, 11, 0.995) - 0.951_110_130_465_771_9).abs() < 1e-12);
        // 0.02 halved once is 0.01, which still counts; halved twice, 0.005, which does not.
        assert!(!has_faded_out(0.02, 0, 1, 0.5));
        assert!(has_faded_out(0.02, 0, 2, 0.5));
    }

    proptest! {
        /// Decay never raises a weight, never takes it below zero, and the longer a weight has
        /// gone untouched the less of it is left.
        #[test]
        fn decay_only_ever_takes_from_a_weight(
            weight in 0.0..1e6_f64,
            last_seq in 0..1_000_000_u64,
            tasks in 0..1_000_000_u64,
            more in 0..1_000_000_u64,
            factor in 0.001..=1.0_f64,
        ) {
            let seq = last_seq.saturating_add(tasks);
            let left = decayed(weight, last_seq, seq, factor);
            prop_assert!(left >= 0.0);
            prop_assert!(left <= weight);
            let later = decayed(weight, last_seq, seq.saturating_add(more), factor);
            prop_assert!(later <= left);
        }
    }
}
