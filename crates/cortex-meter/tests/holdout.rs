//! The holdout comparison on synthetic arms: the rates and medians worked out by hand, the
//! intervals checked by what must hold of any bootstrap of them, and which tasks are weighed.

use cortex_core::{Identifier, Ulid};
use cortex_meter::{
    Compared, Interval, MIN_TASKS_PER_ARM, SessionReceipt, Tally, TaskReceipt, Weighed, compare,
    mean_interval, ratio_interval, weighed,
};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// `count` tasks, the first `successes` of them successful, the successful ones' cumulative input
/// `input` in turn (cycling), their other costs 1.
fn arm(count: usize, successes: usize, input: &[f64]) -> Vec<Weighed> {
    (0..count)
        .map(|at| Weighed {
            is_success: at < successes,
            costs: Some([
                input.get(at % input.len().max(1)).copied().unwrap_or(0.0),
                1.0,
                1.0,
                1.0,
            ]),
        })
        .collect()
}

/// Whether `value` is within a millionth of `expected`.
fn near(value: f64, expected: f64) -> bool {
    (value - expected).abs() < 1e-6
}

/// Fewer than 30 tasks in either arm make no comparison; 30 in each make one.
#[test]
fn a_comparison_needs_thirty_tasks_in_each_arm() {
    assert_eq!(MIN_TASKS_PER_ARM, 30);
    let thirty = arm(30, 10, &[1.0]);
    let twenty_nine = arm(29, 10, &[1.0]);
    assert!(compare(&thirty, &twenty_nine).is_none());
    assert!(compare(&twenty_nine, &thirty).is_none());
    assert!(compare(&thirty, &thirty).is_some());
}

/// Rates and medians by hand. Cortex on: 40 tasks, the first 30 successful, their cumulative input
/// 100, 200, ..., 3,000; the median of 30 is the mean of the 15th and 16th, (1,500 + 1,600) / 2 =
/// 1,550. Held out: 36 tasks, 30 successful, input 4,000, 5,000, ..., 33,000; the median is
/// (18,000 + 19,000) / 2 = 18,500. Success rates 30/40 = 0.75 and 30/36 = 5/6, a difference of
/// -1/12. Every treated input is below every held-out one, so every resampled median difference is
/// too: the interval lies within 100 - 33,000 = -32,900 and 3,000 - 4,000 = -1,000, below zero.
#[test]
fn rates_and_medians_are_the_arms_own() -> TestResult {
    let treated_inputs: Vec<f64> = (1..=30).map(|at| f64::from(at) * 100.0).collect();
    let held_inputs: Vec<f64> = (4..=33).map(|at| f64::from(at) * 1_000.0).collect();
    let treated = arm(40, 30, &treated_inputs);
    let held_out = arm(36, 30, &held_inputs);
    let comparison = compare(&treated, &held_out).ok_or("a comparison")?;
    assert_eq!(comparison.tasks, [40, 36]);
    assert_eq!(comparison.per_success_tasks, [30, 30]);
    let rate = comparison.success_rate;
    assert!(
        near(rate.treated, 0.75) && near(rate.held_out, 5.0 / 6.0),
        "{rate:?}"
    );
    assert!(near(rate.difference, -1.0 / 12.0), "{rate:?}");
    let [input, uncached, turns, calls] = comparison.per_success.ok_or("medians")?;
    assert!(
        near(input.treated, 1_550.0) && near(input.held_out, 18_500.0),
        "{input:?}"
    );
    assert!(near(input.difference, -16_950.0), "{input:?}");
    assert!(
        input.low >= -32_900.0 && input.high <= -1_000.0,
        "{input:?}"
    );
    assert!(
        input.low <= input.difference && input.difference <= input.high,
        "{input:?}"
    );
    for constant in [uncached, turns, calls] {
        assert_eq!(
            constant,
            Compared {
                treated: 1.0,
                held_out: 1.0,
                difference: 0.0,
                low: 0.0,
                high: 0.0
            }
        );
    }
    Ok(())
}

/// Two arms of the same tasks differ by nothing, and the interval holds zero; the same arms give
/// the same interval every time.
#[test]
fn the_same_arms_differ_by_nothing_and_always_alike() -> TestResult {
    let inputs: Vec<f64> = (1..=30).map(|at| f64::from(at * at)).collect();
    let tasks = arm(40, 30, &inputs);
    let comparison = compare(&tasks, &tasks).ok_or("a comparison")?;
    for compared in
        std::iter::once(comparison.success_rate).chain(comparison.per_success.ok_or("medians")?)
    {
        assert!(near(compared.difference, 0.0), "{compared:?}");
        assert!(compared.low <= 0.0 && 0.0 <= compared.high, "{compared:?}");
    }
    assert_eq!(compare(&tasks, &tasks), Some(comparison));
    Ok(())
}

/// The medians per successful task need 30 successful tasks with their usage read in each arm, as
/// the comparison needs 30 tasks; the rates compare all the same, and the counts are kept. No
/// success against 3: no medians, counts 0 and 3, rates 0 and 0.1. 29 successes against 30: no
/// medians. 30 against 30: medians. 30 successes of which one's usage was not read count 29: no
/// medians.
#[test]
fn medians_need_thirty_successful_tasks_in_each_arm() -> TestResult {
    let comparison = compare(&arm(30, 0, &[5.0]), &arm(30, 3, &[5.0])).ok_or("a comparison")?;
    assert_eq!(comparison.per_success, None);
    assert_eq!(comparison.per_success_tasks, [0, 3]);
    assert!(near(comparison.success_rate.difference, -0.1));
    let twenty_nine = compare(&arm(40, 29, &[5.0]), &arm(40, 30, &[5.0])).ok_or("a comparison")?;
    assert_eq!(twenty_nine.per_success, None);
    assert_eq!(twenty_nine.per_success_tasks, [29, 30]);
    let thirty = compare(&arm(40, 30, &[5.0]), &arm(40, 30, &[5.0])).ok_or("a comparison")?;
    assert!(thirty.per_success.is_some());
    let mut unread = arm(40, 30, &[5.0]);
    if let Some(first) = unread.first_mut() {
        first.costs = None;
    }
    let unmetered = compare(&unread, &arm(40, 30, &[5.0])).ok_or("a comparison")?;
    assert_eq!(unmetered.per_success, None);
    assert_eq!(unmetered.per_success_tasks, [29, 30]);
    Ok(())
}

/// A task is weighed once it has an outcome and its arm is decided; its costs once its session's
/// agent's transcripts are read. Of four tasks of a metered session (succeeded and treated, failed
/// and held out, open, undecided) and one succeeded and held out in a session not metered: the
/// treated arm holds the first with its costs, the held-out arm the second with its costs and the
/// last without.
#[test]
fn only_ended_decided_tasks_are_weighed() -> TestResult {
    let usage = Tally {
        turns: 4,
        cumulative_input: 900,
        uncached_input: 90,
        cache_reads: 800,
        cache_writes: 10,
        output: 40,
        estimate: None,
    };
    let task = |seq: u64, outcome: Option<&'static str>, held_out: Option<bool>| TaskReceipt {
        task_id: Ulid::from_seed(seq, b"holdout"),
        seq,
        started_at_ms: 0,
        ended_at_ms: outcome.map(|_| 1),
        outcome,
        held_out,
        tool_calls: 3,
        injected_tokens: 0,
        usage: usage.clone(),
    };
    let metered = SessionReceipt {
        session_id: Identifier::new("s1")?,
        agent: "claude",
        is_metered: true,
        started_at_ms: 0,
        ended_at_ms: None,
        status: "live",
        subagents: 0,
        tool_calls: 12,
        tasks: vec![
            task(1, Some("success"), Some(false)),
            task(2, Some("failure"), Some(true)),
            task(3, None, Some(false)),
            task(4, Some("success"), None),
        ],
        outside_tasks: usage.clone(),
        outside_tool_calls: 0,
        total: usage.clone(),
        compactions_left_out: 0,
        primer_tokens: 0,
        injected_tokens: 0,
    };
    let unmetered = SessionReceipt {
        session_id: Identifier::new("s2")?,
        agent: "codex",
        is_metered: false,
        tasks: vec![task(5, Some("success"), Some(true))],
        ..metered.clone()
    };
    let (treated, held_out) = weighed(&[metered, unmetered]);
    let costs = Some([900.0, 90.0, 4.0, 3.0]);
    assert_eq!(
        treated,
        [Weighed {
            is_success: true,
            costs
        }]
    );
    assert_eq!(
        held_out,
        [
            Weighed {
                is_success: false,
                costs
            },
            Weighed {
                is_success: true,
                costs: None
            }
        ]
    );
    Ok(())
}

/// A mean's interval: of ten values all 0.5, the mean and both ends are 0.5; of seven ones and
/// three zeros, the mean is 0.7, held by the interval, which lies within 0 and 1 and is the same
/// every time; of no value there is none.
#[test]
fn a_mean_has_its_interval() -> TestResult {
    let constant = mean_interval(&[0.5; 10]).ok_or("an interval")?;
    assert_eq!(
        constant,
        Interval {
            value: 0.5,
            low: 0.5,
            high: 0.5
        }
    );
    let labels = [1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let precision = mean_interval(&labels).ok_or("an interval")?;
    assert!(near(precision.value, 0.7), "{precision:?}");
    assert!(
        precision.low >= 0.0 && precision.high <= 1.0,
        "{precision:?}"
    );
    assert!(
        precision.low <= 0.7 && 0.7 <= precision.high,
        "{precision:?}"
    );
    assert_eq!(mean_interval(&labels), Some(precision));
    assert_eq!(mean_interval(&[]), None);
    Ok(())
}

/// A ratio's interval resamples whole groups. Groups of one ratio, 1/2, 2/4 and 3/6, give 0.5 in
/// every resample, so the value and both ends are 0.5; a group with nothing in it changes no ratio
/// and a resample of it alone has none, so 0/0 beside 1/2 gives 0.5 throughout. Two groups, 4 of 4
/// and 0 of 4, give 0.5; a resample draws two groups: both the first (one time in four) gives 1,
/// both the second (one in four) gives 0, one of each gives 0.5. Of 2,000 resamples about 500 are
/// 0 and 500 are 1, so the 2.5th percentile (the 50th lowest) is 0 and the 97.5th is 1. The same
/// eight outcomes taken one by one reach 1 only when all eight draws are ones, one time in 256,
/// about 8 of 2,000, short of the 50 the top end needs, and 0 as rarely: their interval lies
/// strictly inside 0 to 1. No whole, or no group, gives none.
#[test]
fn a_ratio_resamples_whole_groups() -> TestResult {
    let alike = ratio_interval(&[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]).ok_or("an interval")?;
    let half = Interval {
        value: 0.5,
        low: 0.5,
        high: 0.5,
    };
    assert_eq!(alike, half);
    assert_eq!(ratio_interval(&[(0.0, 0.0), (1.0, 2.0)]), Some(half));
    let split = [(4.0, 4.0), (0.0, 4.0)];
    let grouped = ratio_interval(&split).ok_or("an interval")?;
    assert_eq!(
        grouped,
        Interval {
            value: 0.5,
            low: 0.0,
            high: 1.0
        }
    );
    let one_by_one =
        mean_interval(&[1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]).ok_or("an interval")?;
    assert!(near(one_by_one.value, 0.5), "{one_by_one:?}");
    assert!(
        one_by_one.low > 0.0 && one_by_one.high < 1.0,
        "{one_by_one:?}"
    );
    assert_eq!(ratio_interval(&split), Some(grouped));
    assert_eq!(ratio_interval(&[(0.0, 0.0)]), None);
    assert_eq!(ratio_interval(&[]), None);
    Ok(())
}
