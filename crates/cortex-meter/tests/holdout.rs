//! The holdout comparison on synthetic arms: the rates and medians worked out by hand, the
//! intervals checked by what must hold of any bootstrap of them, and which tasks are weighed.

use cortex_core::{Identifier, Ulid};
use cortex_meter::{
    Compared, MIN_TASKS_PER_ARM, SessionReceipt, Tally, TaskReceipt, Weighed, compare, weighed,
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

/// Rates and medians by hand. Cortex on: 30 tasks, the first 15 successful, their cumulative input
/// 100, 200, ..., 1,500; the median of 15 is the 8th, 800. Held out: 30 tasks, 10 successful, input
/// 2,000, 3,000, ..., 11,000; the median of 10 is the mean of the 5th and 6th, (6,000 + 7,000) / 2
/// = 6,500. Success rates 15/30 = 0.5 and 10/30 = 1/3, a difference of 1/6. Every treated input is
/// below every held-out one, so every resampled median difference is too: the interval lies within
/// 100 - 11,000 = -10,900 and 1,500 - 2,000 = -500, below zero.
#[test]
fn rates_and_medians_are_the_arms_own() -> TestResult {
    let treated_inputs: Vec<f64> = (1..=15).map(|at| f64::from(at) * 100.0).collect();
    let held_inputs: Vec<f64> = (2..=11).map(|at| f64::from(at) * 1_000.0).collect();
    let treated = arm(30, 15, &treated_inputs);
    let held_out = arm(30, 10, &held_inputs);
    let comparison = compare(&treated, &held_out).ok_or("a comparison")?;
    assert_eq!(comparison.tasks, [30, 30]);
    let rate = comparison.success_rate;
    assert!(
        near(rate.treated, 0.5) && near(rate.held_out, 1.0 / 3.0),
        "{rate:?}"
    );
    assert!(near(rate.difference, 1.0 / 6.0), "{rate:?}");
    let [input, uncached, turns, calls] = comparison.per_success.ok_or("medians")?;
    assert!(
        near(input.treated, 800.0) && near(input.held_out, 6_500.0),
        "{input:?}"
    );
    assert!(near(input.difference, -5_700.0), "{input:?}");
    assert!(input.low >= -10_900.0 && input.high <= -500.0, "{input:?}");
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

/// With no successful task in an arm there are no medians to compare, but the rates still are.
#[test]
fn without_a_success_in_an_arm_only_the_rates_compare() -> TestResult {
    let comparison = compare(&arm(30, 0, &[5.0]), &arm(30, 3, &[5.0])).ok_or("a comparison")?;
    assert_eq!(comparison.per_success, None);
    assert!(near(comparison.success_rate.difference, -0.1));
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
