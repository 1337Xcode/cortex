//! What an evaluation's runs add up to: how often the tasks succeeded, how precise Cortex's work
//! orders to the measured sessions were, with how many there were, and what the measured sessions
//! took; each share with its 95% bootstrap interval. It is written as JSON and as text, and holds
//! counts only, never what a session said.

use std::collections::BTreeSet;

use cortex_meter::{Interval, mean_interval};
use serde::Serialize;

use crate::run::TaskRun;

/// What the runs add up to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Summary {
    /// How many runs.
    pub(crate) runs: u64,
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

/// What `runs` add up to.
#[must_use]
pub(crate) fn summary(runs: &[TaskRun]) -> Summary {
    let shares = |ones: u64, all: u64| -> Vec<f64> {
        (0..all)
            .map(|at| if at < ones { 1.0 } else { 0.0 })
            .collect()
    };
    let count = |of: &dyn Fn(&TaskRun) -> u64| runs.iter().map(of).sum::<u64>();
    let successes = count(&|run| u64::from(run.is_success));
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
        success: mean_interval(&shares(successes, total)),
        delivered: count(&|run| run.given.delivered),
        labelled,
        used,
        precision: mean_interval(&shares(used, labelled)),
        median_cumulative_input: median_of(&|session| session.cumulative_input),
        median_turns: median_of(&|session| session.turns),
        median_tool_calls: median_of(&|session| session.tool_calls),
        cost_usd: sessions.clone().map(|session| session.cost_usd).sum(),
        agents: agents.into_iter().collect(),
        failed_sessions: u64::try_from(sessions.filter(|session| session.is_error).count())
            .unwrap_or(u64::MAX),
    }
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
                    "{:.2} (95% interval {:.2} to {:.2})",
                    interval.value, interval.low, interval.high
                )
            },
        )
    };
    let number =
        |value: Option<f64>| value.map_or_else(|| "none".to_owned(), |value| format!("{value:.1}"));
    vec![
        format!(
            "runs: {}; sessions that ended in an error: {}",
            summary.runs, summary.failed_sessions
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
        let summary = summary(&runs);
        assert_eq!(summary.runs, 4);
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

    /// With no work order labelled there is no precision, said as none.
    #[test]
    fn no_label_means_no_precision() {
        let summary = summary(&[run(true, (1, 1, 1), Given::default())]);
        assert_eq!(summary.precision, None);
        assert!(lines(&summary).contains(&"precision: none".to_owned()));
    }
}
