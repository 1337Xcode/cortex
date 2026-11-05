//! What an evaluation's runs add up to: how often the tasks succeeded, how precise Cortex's work
//! orders to the measured sessions were, with how many there were, what the measured sessions
//! took, and which runs could not complete. Each share has a 95% bootstrap interval that resamples
//! whole tasks, since the runs of one task share its prompt and code and are not independent. It
//! is written as JSON and as text, and holds counts only, never what a session said.

use std::collections::{BTreeMap, BTreeSet};

use cortex_meter::{Interval, ratio_interval};
use serde::{Deserialize, Serialize};

use crate::run::TaskRun;

/// A run that could not complete: its task and number. Why is in the runner's output, which names
/// local paths and stays out of the results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Incomplete {
    /// The task.
    pub(crate) task: String,
    /// The run's number, from 1.
    pub(crate) run: u32,
}

/// What the runs add up to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Summary {
    /// How many runs completed; only they are counted below.
    pub(crate) runs: u64,
    /// The runs that could not complete.
    pub(crate) incomplete: Vec<Incomplete>,
    /// The share of runs whose check passed.
    pub(crate) success: Option<Interval>,
    /// Work orders that reached the measured sessions, and of those labelled and used.
    pub(crate) delivered: u64,
    /// Of the delivered, the labelled.
    pub(crate) labelled: u64,
    /// Of the labelled, the used.
    pub(crate) used: u64,
    /// The share of the labelled work orders that were used.
    pub(crate) precision: Option<Interval>,
    /// The median of the measured sessions' cumulative input tokens.
    pub(crate) median_cumulative_input: Option<f64>,
    /// The median of their turns.
    pub(crate) median_turns: Option<f64>,
    /// The median of their tool calls.
    pub(crate) median_tool_calls: Option<f64>,
    /// What every session cost, as the agent reported it, in US dollars.
    pub(crate) cost_usd: f64,
    /// The agent versions, models and credential sources the sessions reported.
    pub(crate) agents: Vec<String>,
    /// Sessions that ended in an error.
    pub(crate) failed_sessions: u64,
}

/// What the completed `runs` add up to, with the runs that could not complete, `incomplete`.
#[must_use]
pub(crate) fn summary(runs: &[TaskRun], incomplete: &[Incomplete]) -> Summary {
    let count = |of: &dyn Fn(&TaskRun) -> u64| runs.iter().map(of).sum::<u64>();
    let total = u64::try_from(runs.len()).unwrap_or(u64::MAX);
    let (labelled, used) = (
        count(&|run| run.given.labelled),
        count(&|run| run.given.used),
    );
    let measured: Vec<&crate::agent::Session> =
        runs.iter().filter_map(|run| run.sessions.last()).collect();
    let median_of = |of: &dyn Fn(&crate::agent::Session) -> u64| {
        #[expect(
            clippy::cast_precision_loss,
            reason = "token and call counts stay below 2^52"
        )]
        let values: Vec<f64> = measured.iter().map(|session| of(session) as f64).collect();
        median(&values)
    };
    let sessions = runs.iter().flat_map(|run| run.sessions.iter());
    let agents: BTreeSet<String> = sessions
        .clone()
        .map(|session| {
            format!(
                "{} {} via {}",
                session
                    .agent_version
                    .as_deref()
                    .unwrap_or("unknown version"),
                session.model.as_deref().unwrap_or("unknown model"),
                session
                    .api_key_source
                    .as_deref()
                    .unwrap_or("unknown credentials")
            )
        })
        .collect();
    Summary {
        runs: total,
        incomplete: incomplete.to_vec(),
        success: ratio_interval(&by_task(runs, &|run| (u64::from(run.is_success), 1))),
        delivered: count(&|run| run.given.delivered),
        labelled,
        used,
        precision: ratio_interval(&by_task(runs, &|run| (run.given.used, run.given.labelled))),
        median_cumulative_input: median_of(&|session| session.cumulative_input),
        median_turns: median_of(&|session| session.turns),
        median_tool_calls: median_of(&|session| session.tool_calls),
        cost_usd: sessions.clone().map(|session| session.cost_usd).sum(),
        agents: agents.into_iter().collect(),
        failed_sessions: u64::try_from(sessions.filter(|session| session.is_error).count())
            .unwrap_or(u64::MAX),
    }
}

/// Each task's `(part, whole)` summed over its runs, such as its used and labelled work orders, in
/// the order of the tasks' names.
fn by_task(runs: &[TaskRun], of: &dyn Fn(&TaskRun) -> (u64, u64)) -> Vec<(f64, f64)> {
    let mut tasks: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for run in runs {
        let (part, whole) = of(run);
        let sums = tasks.entry(run.task.as_str()).or_default();
        *sums = (sums.0 + part, sums.1 + whole);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "run and work order counts stay below 2^52"
    )]
    tasks
        .into_values()
        .map(|(part, whole)| (part as f64, whole as f64))
        .collect()
}

/// The median of `values`, none for none.
fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let upper = *sorted.get(middle)?;
    Some(if sorted.len() % 2 == 1 {
        upper
    } else {
        sorted
            .get(middle.wrapping_sub(1))
            .map_or(upper, |lower| f64::midpoint(*lower, upper))
    })
}

/// `summary` as lines of text.
#[must_use]
pub(crate) fn lines(summary: &Summary) -> Vec<String> {
    let share = |interval: Option<Interval>| {
        interval.map_or_else(
            || "none".to_owned(),
            |interval| {
                format!(
                    "{:.2} (95% interval {:.2} to {:.2}, resampling tasks)",
                    interval.value, interval.low, interval.high
                )
            },
        )
    };
    let number =
        |value: Option<f64>| value.map_or_else(|| "none".to_owned(), |value| format!("{value:.1}"));
    vec![
        format!(
            "runs: {} completed, {} could not complete; sessions that ended in an error: {}",
            summary.runs,
            summary.incomplete.len(),
            summary.failed_sessions
        ),
        format!("success: {}", share(summary.success)),
        format!(
            "work orders to the measured sessions: {} delivered, {} labelled, {} used",
            summary.delivered, summary.labelled, summary.used
        ),
        format!("precision: {}", share(summary.precision)),
        format!(
            "measured sessions, medians: {} cumulative input tokens, {} turns, {} tool calls",
            number(summary.median_cumulative_input),
            number(summary.median_turns),
            number(summary.median_tool_calls)
        ),
        format!(
            "cost of every session, as the agent reported it: ${:.2}",
            summary.cost_usd
        ),
        format!("agents: {}", summary.agents.join("; ")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Session;
    use crate::run::Given;

    /// A run of `task`, its check passed or not, its measured session's counts, and what Cortex
    /// gave it.
    fn run(is_success: bool, (input, turns, calls): (u64, u64, u64), given: Given) -> TaskRun {
        let session = Session {
            session_id: Some("s".to_owned()),
            agent_version: Some("2.1.318".to_owned()),
            model: Some("m".to_owned()),
            api_key_source: Some("none".to_owned()),
            is_error: false,
            turns,
            tool_calls: calls,
            cumulative_input: input,
            uncached_input: 0,
            output: 0,
            cost_usd: 0.5,
            duration_ms: 0,
        };
        TaskRun {
            task: "t".to_owned(),
            kind: "k".to_owned(),
            run: 1,
            holdout_share: 0.0,
            sessions: vec![session.clone(), session],
            is_success,
            given,
        }
    }

    /// Four runs, worked by hand: three succeed, 0.75; work orders delivered 1 + 1 + 1 + 0 = 3, all
    /// labelled, 2 used, a precision of 2/3; the measured sessions' inputs 100, 300, 200 and 400
    /// have the median (200 + 300) / 2 = 250, their turns 1, 3, 2 and 4 the median 2.5, their tool
    /// calls 5 each the median 5; eight sessions at $0.50 cost $4.
    #[test]
    fn the_runs_add_up() {
        let one = |used| Given {
            delivered: 1,
            labelled: 1,
            used,
        };
        let runs = [
            run(true, (100, 1, 5), one(1)),
            run(true, (300, 3, 5), one(1)),
            run(false, (200, 2, 5), one(0)),
            run(true, (400, 4, 5), Given::default()),
        ];
        let missed = [Incomplete {
            task: "t".to_owned(),
            run: 5,
        }];
        let summary = summary(&runs, &missed);
        assert_eq!(summary.runs, 4);
        assert_eq!(summary.incomplete, missed);
        assert!(
            lines(&summary).contains(
                &"runs: 4 completed, 1 could not complete; sessions that ended in an error: 0"
                    .to_owned()
            )
        );
        assert!(
            summary
                .success
                .is_some_and(|success| (success.value - 0.75).abs() < 1e-9)
        );
        assert_eq!(
            (summary.delivered, summary.labelled, summary.used),
            (3, 3, 2)
        );
        assert!(
            summary
                .precision
                .is_some_and(|precision| (precision.value - 2.0 / 3.0).abs() < 1e-9)
        );
        assert_eq!(summary.median_cumulative_input, Some(250.0));
        assert_eq!(summary.median_turns, Some(2.5));
        assert_eq!(summary.median_tool_calls, Some(5.0));
        assert!((summary.cost_usd - 4.0).abs() < 1e-9);
        assert_eq!(summary.agents, ["2.1.318 m via none"]);
    }

    /// The intervals resample whole tasks. Task a's four runs pass and use their work order, task
    /// b's four fail and do not: success and precision are 4/8 = 0.5. A resample draws two tasks:
    /// a twice gives 1, b twice 0, one of each 0.5, so of 2,000 resamples about 500 are 0 and 500
    /// are 1, and the interval runs from 0 to 1. Taken run by run, eight draws would all be ones
    /// only one time in 256, too rarely to reach either end.
    #[test]
    fn intervals_resample_whole_tasks() {
        let used = |used| Given {
            delivered: 1,
            labelled: 1,
            used,
        };
        let of = |task: &str, is_success, given| TaskRun {
            task: task.to_owned(),
            ..run(is_success, (1, 1, 1), given)
        };
        let runs: Vec<TaskRun> = (0..4)
            .flat_map(|_| [of("a", true, used(1)), of("b", false, used(0))])
            .collect();
        let summary = summary(&runs, &[]);
        let half = Some(Interval {
            value: 0.5,
            low: 0.0,
            high: 1.0,
        });
        assert_eq!(summary.success, half);
        assert_eq!(summary.precision, half);
        assert!(
            lines(&summary).contains(
                &"precision: 0.50 (95% interval 0.00 to 1.00, resampling tasks)".to_owned()
            )
        );
    }

    /// With no work order labelled there is no precision, said as none.
    #[test]
    fn no_label_means_no_precision() {
        let summary = summary(&[run(true, (1, 1, 1), Given::default())], &[]);
        assert_eq!(summary.precision, None);
        assert!(lines(&summary).contains(&"precision: none".to_owned()));
    }
}
