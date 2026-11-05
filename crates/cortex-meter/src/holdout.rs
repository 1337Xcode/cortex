//! The holdout comparison: the tasks Cortex held out set against the tasks it could help, on how
//! often a task succeeded and, per successful task, the median cumulative input, uncached input,
//! turns and tool calls, each difference with a 95% bootstrap interval.
//!
//! A task counts once it has ended with an outcome; its usage counts once its session's agent's
//! transcripts are read. The comparison is made only with [`MIN_TASKS_PER_ARM`] or more such tasks
//! in each arm: fewer make it too uncertain to state, and receipts say so instead.
//!
//! An interval is the percentile bootstrap: each arm resampled with replacement
//! [`BOOTSTRAP_RESAMPLES`] times, the difference taken each time, the 2.5th and 97.5th percentiles
//! of those differences kept. The resampling draws from a generator seeded by a hash of the values
//! compared, so the same tasks always give the same interval.

use cortex_core::{ContentHash, TaskOutcome};
use serde::Serialize;

use crate::receipts::{MIN_TASKS_PER_ARM, SessionReceipt, TaskReceipt};

/// How many times each arm is resampled for an interval.
pub const BOOTSTRAP_RESAMPLES: usize = 2_000;

/// The share of the resampled differences left below the interval, and above it.
const TAIL: f64 = 0.025;

/// One task as the comparison weighs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weighed {
    /// Whether it succeeded.
    pub is_success: bool,
    /// Its usage, when its session's agent's transcripts are read: every input token, the
    /// uncached part, its turns and its tool calls.
    pub costs: Option<[f64; 4]>,
}

/// One measure compared: each arm's value and the difference, Cortex on minus held out, with its
/// 95% interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Compared {
    /// With Cortex.
    pub treated: f64,
    /// Held out.
    pub held_out: f64,
    /// Treated minus held out.
    pub difference: f64,
    /// The difference's interval: its low end.
    pub low: f64,
    /// Its high end.
    pub high: f64,
}

/// The comparison of the two arms.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    /// How many tasks with an outcome each arm holds.
    pub tasks: [u64; 2],
    /// The share of tasks that succeeded.
    pub success_rate: Compared,
    /// Per successful task, the medians, when both arms have a successful task whose usage was
    /// read: cumulative input, uncached input, turns and tool calls.
    pub per_success: Option<[Compared; 4]>,
}

/// The tasks of `sessions` the comparison weighs, by arm (treated, held out): each that ended
/// with an outcome and whose arm was decided.
#[must_use]
pub fn weighed(sessions: &[SessionReceipt]) -> (Vec<Weighed>, Vec<Weighed>) {
    let mut arms = (Vec::new(), Vec::new());
    for session in sessions {
        for task in &session.tasks {
            let (Some(outcome), Some(held_out)) = (task.outcome, task.held_out) else {
                continue;
            };
            let one = Weighed {
                is_success: outcome == TaskOutcome::Success.name(),
                costs: session.is_metered.then(|| costs(task)),
            };
            if held_out {
                arms.1.push(one);
            } else {
                arms.0.push(one);
            }
        }
    }
    arms
}

/// `task`'s usage as numbers to compare.
#[expect(
    clippy::cast_precision_loss,
    reason = "token and call counts stay far below the 2^52 a float holds exactly"
)]
fn costs(task: &TaskReceipt) -> [f64; 4] {
    [
        task.usage.cumulative_input as f64,
        task.usage.uncached_input as f64,
        task.usage.turns as f64,
        task.tool_calls as f64,
    ]
}

/// The comparison of `treated` with `held_out`; none when either holds fewer than
/// [`MIN_TASKS_PER_ARM`] tasks.
#[must_use]
pub fn compare(treated: &[Weighed], held_out: &[Weighed]) -> Option<Comparison> {
    let enough = u64::try_from(treated.len().min(held_out.len())).unwrap_or(0) >= MIN_TASKS_PER_ARM;
    if !enough {
        return None;
    }
    let rate = |arm: &[Weighed]| -> Vec<f64> {
        arm.iter()
            .map(|task| if task.is_success { 1.0 } else { 0.0 })
            .collect()
    };
    let success_rate = compared(&rate(treated), &rate(held_out), mean);
    let measure = |arm: &[Weighed], at: usize| -> Vec<f64> {
        arm.iter()
            .filter(|task| task.is_success)
            .filter_map(|task| task.costs.and_then(|costs| costs.get(at).copied()))
            .collect()
    };
    let per_success = (0..4)
        .map(|at| {
            let (on, off) = (measure(treated, at), measure(held_out, at));
            (!on.is_empty() && !off.is_empty()).then(|| compared(&on, &off, median))
        })
        .collect::<Option<Vec<Compared>>>()
        .and_then(|all| <[Compared; 4]>::try_from(all).ok());
    Some(Comparison {
        tasks: [treated.len(), held_out.len()]
            .map(|count| u64::try_from(count).unwrap_or(u64::MAX)),
        success_rate,
        per_success,
    })
}

/// `statistic` of `treated` and of `held_out`, their difference and its bootstrap interval.
fn compared(treated: &[f64], held_out: &[f64], statistic: fn(&[f64]) -> f64) -> Compared {
    let (on, off) = (statistic(treated), statistic(held_out));
    let mut sequence = Sequence::seeded(treated, held_out);
    let mut differences: Vec<f64> = (0..BOOTSTRAP_RESAMPLES)
        .map(|_| statistic(&sequence.resample(treated)) - statistic(&sequence.resample(held_out)))
        .collect();
    differences.sort_by(f64::total_cmp);
    Compared {
        treated: on,
        held_out: off,
        difference: on - off,
        low: percentile(&differences, TAIL),
        high: percentile(&differences, 1.0 - TAIL),
    }
}

/// A mean of one sample and its 95% bootstrap interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Interval {
    /// The mean.
    pub value: f64,
    /// The interval's low end.
    pub low: f64,
    /// Its high end.
    pub high: f64,
}

/// The mean of `values` with its 95% percentile bootstrap interval, resampled as the comparison
/// resamples an arm, from a generator seeded by a hash of the values; none for no value. A share,
/// such as a precision, is the mean of ones and zeros.
#[must_use]
pub fn mean_interval(values: &[f64]) -> Option<Interval> {
    if values.is_empty() {
        return None;
    }
    let mut sequence = Sequence::seeded(values, &[]);
    let mut means: Vec<f64> = (0..BOOTSTRAP_RESAMPLES)
        .map(|_| mean(&sequence.resample(values)))
        .collect();
    means.sort_by(f64::total_cmp);
    Some(Interval {
        value: mean(values),
        low: percentile(&means, TAIL),
        high: percentile(&means, 1.0 - TAIL),
    })
}

/// The mean of `values`, 0 for none.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample sizes stay far below 2^52"
)]
fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// The median of `values`: the middle one, or the mean of the middle two; 0 for none.
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    match (sorted.len() % 2, sorted.get(middle)) {
        (_, None) => 0.0,
        (1, Some(value)) => *value,
        (_, Some(upper)) => sorted
            .get(middle.wrapping_sub(1))
            .map_or(*upper, |lower| f64::midpoint(*lower, *upper)),
    }
}

/// The `share` percentile of `sorted`, by nearest rank; 0 for none.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the rank of a share of a few thousand resamples is a small whole number"
)]
fn percentile(sorted: &[f64], share: f64) -> f64 {
    let rank = (share * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank - 1).copied().unwrap_or(0.0)
}

/// A fixed sequence of pseudo-random numbers (`SplitMix64`).
struct Sequence(u64);

impl Sequence {
    /// The sequence seeded by a hash of the values compared, so that the same values always draw
    /// the same resamples.
    fn seeded(treated: &[f64], held_out: &[f64]) -> Self {
        let mut bytes = Vec::with_capacity((treated.len() + held_out.len() + 1) * 8);
        for value in treated {
            bytes.extend(value.to_bits().to_le_bytes());
        }
        bytes.extend(u64::MAX.to_le_bytes());
        for value in held_out {
            bytes.extend(value.to_bits().to_le_bytes());
        }
        let hash = ContentHash::of(&bytes);
        let mut seed = [0; 8];
        seed.copy_from_slice(hash.as_bytes().get(..8).unwrap_or(&[0; 8]));
        Self(u64::from_le_bytes(seed))
    }

    /// The next number below `bound`.
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        usize::try_from(mixed % u64::try_from(bound.max(1)).unwrap_or(1)).unwrap_or(0)
    }

    /// `values` drawn with replacement, as many as there are.
    fn resample(&mut self, values: &[f64]) -> Vec<f64> {
        (0..values.len())
            .filter_map(|_| values.get(self.below(values.len())).copied())
            .collect()
    }
}
