//! Two arms of one evaluation side by side, paired by task: the same tasks run with Cortex giving
//! work orders (the treated arm) and with Cortex holding every task out, or with the tasks' project
//! rules noted and without them. For each measure, the mean
//! over tasks in each arm (a task's value is the mean of its runs) and the mean of the per-task
//! differences, treated minus held out, with its 95% bootstrap interval over tasks. The same again
//! for the tasks whose treated runs got a work order and for those whose did not, since the arms
//! differ only where Cortex said something. Counts only, as the summaries are.

use std::collections::BTreeMap;

use cortex_meter::{Interval, mean_interval};
use serde::Serialize;

use crate::agent::Session;
use crate::run::TaskRun;

/// A measure of one run: its name, the decimals it is written with, and how a run gives it.
type Measure = (&'static str, usize, fn(&TaskRun) -> f64);

/// What is compared: the measured session's usage, every session's cost, and the check.
const MEASURES: [Measure; 6] = [
    ("measured session, cumulative input tokens", 0, |run| {
        measured(run, |session| session.cumulative_input)
    }),
    ("measured session, uncached input tokens", 0, |run| {
        measured(run, |session| session.uncached_input)
    }),
    ("measured session, turns", 2, |run| {
        measured(run, |session| session.turns)
    }),
    ("measured session, tool calls", 2, |run| {
        measured(run, |session| session.tool_calls)
    }),
    ("every session of the run, US dollars", 3, |run| {
        run.sessions.iter().map(|session| session.cost_usd).sum()
    }),
    ("check passed, share of runs", 2, |run| {
        if run.is_success { 1.0 } else { 0.0 }
    }),
];

/// One measure compared over a group of tasks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Compared {
    /// The measure.
    pub(crate) measure: &'static str,
    /// The decimals it is written with.
    #[serde(skip)]
    decimals: usize,
    /// The mean over the group's tasks with Cortex.
    pub(crate) treated: f64,
    /// The mean held out.
    pub(crate) held_out: f64,
    /// The mean per-task difference, treated minus held out, with its interval.
    pub(crate) difference: Interval,
}

/// A group of paired tasks and its measures compared.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Group {
    /// Which tasks: all, those given a work order, those not.
    pub(crate) name: &'static str,
    /// How many tasks.
    pub(crate) tasks: usize,
    /// Each measure, in [`MEASURES`]' order; empty for a group of no task.
    pub(crate) measures: Vec<Compared>,
}

/// One task's runs in each arm, treated first.
type Paired<'a> = (&'a [&'a TaskRun], &'a [&'a TaskRun]);

/// The value of `field` of `run`'s measured session (its last), 0 when it has none.
#[expect(clippy::cast_precision_loss, reason = "token counts stay below 2^52")]
fn measured(run: &TaskRun, field: fn(&Session) -> u64) -> f64 {
    run.sessions
        .last()
        .map_or(0.0, |session| field(session) as f64)
}

/// The mean of `values`, 0 for none.
#[expect(clippy::cast_precision_loss, reason = "run counts stay far below 2^52")]
fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// The runs of each task, by task.
fn by_task(runs: &[TaskRun]) -> BTreeMap<&str, Vec<&TaskRun>> {
    let mut tasks: BTreeMap<&str, Vec<&TaskRun>> = BTreeMap::new();
    for run in runs {
        tasks.entry(run.task.as_str()).or_default().push(run);
    }
    tasks
}

/// `treated` and `held_out` compared over the tasks both ran, in task order: all of them, those
/// whose treated runs got a work order (any of them), and those whose did not. A task only one
/// arm ran is left out.
pub(crate) fn compare(treated: &[TaskRun], held_out: &[TaskRun]) -> Vec<Group> {
    let (on, off) = (by_task(treated), by_task(held_out));
    let paired: Vec<Paired<'_>> = on
        .iter()
        .filter_map(|(task, runs)| off.get(task).map(|held| (runs.as_slice(), held.as_slice())))
        .collect();
    let is_given = |runs: &[&TaskRun]| runs.iter().any(|run| run.given.delivered > 0);
    let (given, silent): (Vec<Paired<'_>>, Vec<Paired<'_>>) =
        paired.iter().partition(|(runs, _)| is_given(runs));
    vec![
        group("every task", &paired),
        group("tasks whose treated runs got a work order", &given),
        group("tasks whose treated runs got none", &silent),
    ]
}

/// The measures over `tasks`.
fn group(name: &'static str, tasks: &[Paired<'_>]) -> Group {
    let measures = MEASURES
        .iter()
        .filter_map(|&(measure, decimals, of)| {
            let value = |runs: &[&TaskRun]| {
                let values: Vec<f64> = runs.iter().map(|run| of(run)).collect();
                mean(&values)
            };
            let on: Vec<f64> = tasks.iter().map(|(runs, _)| value(runs)).collect();
            let off: Vec<f64> = tasks.iter().map(|(_, runs)| value(runs)).collect();
            let differences: Vec<f64> = on.iter().zip(&off).map(|(a, b)| a - b).collect();
            Some(Compared {
                measure,
                decimals,
                treated: mean(&on),
                held_out: mean(&off),
                difference: mean_interval(&differences)?,
            })
        })
        .collect();
    Group {
        name,
        tasks: tasks.len(),
        measures,
    }
}

/// The names the comparison gives its two arms, from the second arm's `runs`: "with facts" and
/// "without facts" when Cortex treated every one of them and none noted its task's rules; else
/// "with Cortex" and "held out".
pub(crate) fn arm_names(second: &[TaskRun]) -> (&'static str, &'static str) {
    let is_without_facts = !second.is_empty()
        && second
            .iter()
            .all(|run| run.facts == Some(false) && run.holdout_share < 1.0);
    if is_without_facts {
        ("with facts", "without facts")
    } else {
        ("with Cortex", "held out")
    }
}

/// `groups` as lines of text, the arms named `names`: each group's name and size, then a line per
/// measure.
pub(crate) fn lines(groups: &[Group], (first, second): (&str, &str)) -> Vec<String> {
    let mut lines = Vec::new();
    for group in groups {
        lines.push(format!("{}, paired tasks: {}", group.name, group.tasks));
        for compared in &group.measures {
            let decimals = compared.decimals;
            let difference = &compared.difference;
            lines.push(format!(
                "  {}: {:.decimals$} {first}, {:.decimals$} {second}; difference \
                 {:+.decimals$} (95% interval {:+.decimals$} to {:+.decimals$}, resampling tasks)",
                compared.measure,
                compared.treated,
                compared.held_out,
                difference.value,
                difference.low,
                difference.high
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Given;

    /// A run of `task` whose measured session took `input` tokens and `turns` turns, at $1, with
    /// `delivered` work orders.
    fn run(task: &str, (input, turns): (u64, u64), delivered: u64) -> TaskRun {
        let session = Session {
            session_id: None,
            agent_version: None,
            model: None,
            api_key_source: None,
            is_error: false,
            turns,
            tool_calls: turns,
            cumulative_input: input,
            uncached_input: input / 2,
            output: 0,
            cost_usd: 1.0,
            duration_ms: 0,
            texts: None,
        };
        TaskRun {
            task: task.to_owned(),
            kind: "k".to_owned(),
            run: 1,
            holdout_share: 0.0,
            sessions: vec![session],
            is_success: true,
            given: Given {
                delivered,
                labelled: delivered,
                used: delivered,
            },
            shown: None,
            warnings: None,
            config: Vec::new(),
            facts: None,
            notes: 0,
        }
    }

    /// Worked by hand. Task `a`, given a work order: treated runs of 100 and 300 input tokens (mean
    /// 200) against one held-out run of 260, a difference of -60; turns 2 and 4 (mean 3) against
    /// 5, -2. Task `b`, given none: 500 against 400, +100; turns 6 against 6, 0. Task `c` ran held
    /// out only and is left out. Every task: 350 against 330, a mean difference of +20; a group of
    /// one task has its one difference for mean and both ends.
    #[test]
    fn tasks_are_paired_and_split_by_work_order() {
        let treated = [
            run("a", (100, 2), 1),
            run("a", (300, 4), 0),
            run("b", (500, 6), 0),
        ];
        let held_out = [
            run("a", (260, 5), 0),
            run("b", (400, 6), 0),
            run("c", (900, 9), 0),
        ];
        let groups = compare(&treated, &held_out);
        let sizes: Vec<(&str, usize)> = groups
            .iter()
            .map(|group| (group.name, group.tasks))
            .collect();
        assert_eq!(
            sizes,
            [
                ("every task", 2),
                ("tasks whose treated runs got a work order", 1),
                ("tasks whose treated runs got none", 1)
            ]
        );
        fn input(group: &Group) -> &Compared {
            &group.measures[0]
        }
        let every = input(&groups[0]);
        assert_eq!((every.treated, every.held_out), (350.0, 330.0));
        assert_eq!(every.difference.value, 20.0);
        assert!(every.difference.low <= 20.0 && every.difference.high >= 20.0);
        let given = &groups[1];
        assert_eq!(
            input(given).difference,
            Interval {
                value: -60.0,
                low: -60.0,
                high: -60.0
            }
        );
        assert_eq!(given.measures[2].difference.value, -2.0);
        assert_eq!(input(&groups[2]).difference.value, 100.0);
        assert_eq!(groups[2].measures[2].difference.value, 0.0);
    }

    /// The given group's input line, worked by hand above; a group of no task has its name alone.
    #[test]
    fn the_comparison_reads_as_lines() {
        let groups = compare(&[run("a", (100, 2), 1)], &[run("a", (160, 5), 0)]);
        let lines = lines(&groups, ("with Cortex", "held out"));
        assert_eq!(lines[0], "every task, paired tasks: 1");
        assert_eq!(
            lines[1],
            "  measured session, cumulative input tokens: 100 with Cortex, 160 held out; \
             difference -60 (95% interval -60 to -60, resampling tasks)"
        );
        assert_eq!(
            lines[3],
            "  measured session, turns: 2.00 with Cortex, 5.00 held out; difference -3.00 (95% \
             interval -3.00 to -3.00, resampling tasks)"
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("tasks whose treated runs got none, paired tasks: 0")
        );
    }

    /// A second arm whose every run Cortex treated without noting its task's rules is the arm
    /// without facts; one held out, one whose runs noted the rules or predate the choice, and none
    /// at all are the held-out arm.
    #[test]
    fn the_arms_are_named_for_what_sets_them_apart() {
        let with = |holdout_share, facts| TaskRun {
            holdout_share,
            facts,
            ..run("a", (100, 2), 1)
        };
        assert_eq!(
            arm_names(&[with(0.0, Some(false)), with(0.0, Some(false))]),
            ("with facts", "without facts")
        );
        for second in [
            vec![with(1.0, Some(false))],
            vec![with(0.0, Some(false)), with(0.0, Some(true))],
            vec![with(1.0, None)],
            Vec::new(),
        ] {
            assert_eq!(
                arm_names(&second),
                ("with Cortex", "held out"),
                "{second:?}"
            );
        }
    }
}
