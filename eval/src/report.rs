//! What an evaluation's runs add up to: how often the tasks succeeded, how precise Cortex's work
//! orders to the measured sessions were, with how many there were, what the measured sessions
//! took, and which runs could not complete. Each share has a 95% bootstrap interval that resamples
//! whole tasks, since the runs of one task share its prompt and code and are not independent. It
//! is written as JSON and as text, and holds counts only, never what a session said.

use std::collections::{BTreeMap, BTreeSet};

use cortex_meter::{Interval, ratio_interval};
use serde::{Deserialize, Serialize};

use crate::run::TaskRun;
use crate::shown::Shown;
use crate::texts::{Texts, add_texts, share_of_input};
use crate::warned::{Warnings, add_warnings, all_kinds};

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
    /// The check of the code every session was shown, over the runs that made it; none when no
    /// run did.
    pub(crate) shown: Option<Shown>,
    /// The texts Cortex gave every session that recorded them, by the hook they answered; none
    /// when no session did.
    pub(crate) texts: Option<Texts>,
    /// The most those texts can have added to those sessions' input, as a share of it.
    pub(crate) texts_share: Option<f64>,
    /// The warnings every run that recorded them got, by kind; none when no run did.
    pub(crate) warnings: Option<Warnings>,
    /// The share of the labelled warnings that were resolved, over the runs that recorded them.
    pub(crate) warning_precision: Option<Interval>,
    /// The project rules the runs noted before their first sessions.
    pub(crate) notes: u64,
    /// The runs whose arm left their tasks' rules out.
    pub(crate) without_facts: u64,
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
    let agents: BTreeSet<String> = runs
        .iter()
        .flat_map(|run| run.sessions.iter().map(move |session| (run, session)))
        .map(|(run, session)| {
            format!(
                "{} {} at {} effort via {}",
                session
                    .agent_version
                    .as_deref()
                    .unwrap_or("unknown version"),
                session.model.as_deref().unwrap_or("unknown model"),
                run.effort.as_deref().unwrap_or("the default"),
                session
                    .api_key_source
                    .as_deref()
                    .unwrap_or("unknown credentials")
            )
        })
        .collect();
    let recorded: Vec<(&Texts, u64, u64)> = runs
        .iter()
        .flat_map(|run| run.sessions.iter())
        .filter_map(|session| {
            Some((
                session.texts.as_ref()?,
                session.turns,
                session.cumulative_input,
            ))
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
        // A float sum of nothing is minus zero, which prints as a cost of -0.00.
        cost_usd: sessions
            .clone()
            .fold(0.0, |total, session| total + session.cost_usd),
        agents: agents.into_iter().collect(),
        failed_sessions: u64::try_from(sessions.filter(|session| session.is_error).count())
            .unwrap_or(u64::MAX),
        shown: runs.iter().filter_map(|run| run.shown).reduce(Shown::add),
        texts: recorded
            .iter()
            .map(|(texts, _, _)| *texts)
            .fold(None, |sum: Option<Texts>, texts| {
                Some(add_texts(sum.unwrap_or_default(), texts))
            }),
        texts_share: share_of_input(&recorded),
        warnings: runs
            .iter()
            .filter_map(|run| run.warnings.as_ref())
            .fold(None, |sum: Option<Warnings>, warnings| {
                Some(add_warnings(sum.unwrap_or_default(), warnings))
            }),
        warning_precision: ratio_interval(&by_task(runs, &|run| {
            let warned = run.warnings.as_ref().map(all_kinds).unwrap_or_default();
            (warned.resolved, warned.labelled)
        })),
        notes: count(&|run| u64::from(run.notes)),
        without_facts: count(&|run| u64::from(run.facts == Some(false))),
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
        summary.shown.map_or_else(
            || "code lines shown to the agents: not checked".to_owned(),
            |shown| {
                format!(
                    "code lines shown to the agents: {}, of which {} were not their file's line when \
                     shown; sessions not checked: {}",
                    shown.lines, shown.differ, shown.unread_sessions
                )
            },
        ),
        texts_line(summary),
        warnings_line(summary, &share),
        format!("agents: {}", summary.agents.join("; ")),
    ]
    .into_iter()
    .chain(notes_line(summary))
    .collect()
}

/// The line saying how many project rules the runs noted before their first sessions and how many
/// runs left them out; none when no run did either.
fn notes_line(summary: &Summary) -> Option<String> {
    (summary.notes > 0 || summary.without_facts > 0).then(|| {
        format!(
            "project rules noted before the first sessions: {}; runs that left them out: {}",
            summary.notes, summary.without_facts
        )
    })
}

/// The line saying what warnings the edits got and how precise they were, `share` writing the
/// precision.
fn warnings_line(summary: &Summary, share: &dyn Fn(Option<Interval>) -> String) -> String {
    let Some(warnings) = &summary.warnings else {
        return "warnings after edits: not recorded".to_owned();
    };
    if warnings.is_empty() {
        return "warnings after edits: none".to_owned();
    }
    let kinds: Vec<String> = warnings
        .iter()
        .map(|(kind, warned)| {
            format!(
                "{kind} {} ({} labelled, {} resolved)",
                warned.given, warned.labelled, warned.resolved
            )
        })
        .collect();
    format!(
        "warnings after edits: {}; precision: {}",
        kinds.join(", "),
        share(summary.warning_precision)
    )
}

/// The line saying what Cortex's texts to the agents came to.
fn texts_line(summary: &Summary) -> String {
    let Some(texts) = &summary.texts else {
        return "Cortex's texts that reached the agents: not recorded".to_owned();
    };
    if texts.is_empty() {
        return "Cortex's texts that reached the agents: none".to_owned();
    }
    let kinds: Vec<String> = texts
        .iter()
        .map(|(kind, sent)| format!("{kind} {} ({} tokens)", sent.count, sent.tokens))
        .collect();
    let share = summary.texts_share.map_or_else(
        || "no input to weigh them against".to_owned(),
        |share| format!("at most {:.2}% of the sessions' input", share * 100.0),
    );
    format!(
        "Cortex's texts that reached the agents, tokens as Cortex estimates them: {}; {share}",
        kinds.join(", ")
    )
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
            texts: None,
        };
        TaskRun {
            task: "t".to_owned(),
            kind: "k".to_owned(),
            run: 1,
            holdout_share: 0.0,
            sessions: vec![session.clone(), session],
            is_success,
            given,
            shown: None,
            warnings: None,
            config: Vec::new(),
            facts: None,
            notes: 0,
            effort: None,
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
        assert_eq!(summary.agents, ["2.1.318 m at the default effort via none"]);
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

    /// The check of the code shown adds up over the runs that made it, worked by hand: 10 + 5 = 15
    /// lines, 1 + 0 = 1 differing, 0 + 1 = 1 session not checked; a run from before the check is
    /// left out. With no run that made it, the summary says it was not checked.
    #[test]
    fn the_code_shown_adds_up() {
        let checked = |lines, differ, unread_sessions| TaskRun {
            shown: Some(Shown {
                lines,
                differ,
                unread_sessions,
            }),
            ..run(true, (1, 1, 1), Given::default())
        };
        let runs = [
            checked(10, 1, 0),
            checked(5, 0, 1),
            run(true, (1, 1, 1), Given::default()),
        ];
        let summary = summary(&runs, &[]);
        assert_eq!(
            summary.shown,
            Some(Shown {
                lines: 15,
                differ: 1,
                unread_sessions: 1
            })
        );
        assert!(lines(&summary).contains(
            &"code lines shown to the agents: 15, of which 1 were not their file's line when shown; \
              sessions not checked: 1"
                .to_owned()
        ));
        let before = super::summary(&runs[2..], &[]);
        assert_eq!(before.shown, None);
        assert!(lines(&before).contains(&"code lines shown to the agents: not checked".to_owned()));
    }

    /// The texts add up over every session that recorded them, worked by hand: two sessions of a
    /// run each got a primer of 50 tokens over 10 turns of 100,000 input, the measured one also a
    /// work order of 400; primers 2 (100 tokens), work orders 1 (400 tokens); read 50 * 10 + 450 *
    /// 10 = 5,000 times against 200,000 of input, at most 2.50%. A run from before the texts were
    /// recorded is left out, and with none recorded the summary says so.
    #[test]
    fn the_texts_add_up() {
        let sent = |count, tokens| crate::texts::Sent { count, tokens };
        let primer = Texts::from([("session_start".to_owned(), sent(1, 50))]);
        let both = Texts::from([
            ("prompt".to_owned(), sent(1, 400)),
            ("session_start".to_owned(), sent(1, 50)),
        ]);
        let mut recorded = run(true, (100_000, 10, 1), Given::default());
        recorded.sessions = vec![
            Session {
                texts: Some(primer),
                ..recorded.sessions[0].clone()
            },
            Session {
                texts: Some(both),
                ..recorded.sessions[1].clone()
            },
        ];
        let runs = [recorded, run(true, (1, 1, 1), Given::default())];
        let summary = summary(&runs, &[]);
        assert_eq!(
            summary.texts,
            Some(Texts::from([
                ("prompt".to_owned(), sent(1, 400)),
                ("session_start".to_owned(), sent(2, 100)),
            ]))
        );
        assert!(
            summary
                .texts_share
                .is_some_and(|share| (share - 0.025).abs() < 1e-12)
        );
        assert!(lines(&summary).contains(
            &"Cortex's texts that reached the agents, tokens as Cortex estimates them: prompt 1 \
              (400 tokens), session_start 2 (100 tokens); at most 2.50% of the sessions' input"
                .to_owned()
        ));
        let before = super::summary(&runs[1..], &[]);
        assert!(
            lines(&before)
                .contains(&"Cortex's texts that reached the agents: not recorded".to_owned())
        );
    }

    /// The warnings add up over the runs that recorded them, worked by hand: task a's run got two
    /// call-site warnings, both resolved; task b's run a test warning, not resolved, and a co-change
    /// warning whose task had not ended. Call sites 2 (2 labelled, 2 resolved), co-change 1 (0
    /// labelled), tests 1 (1 labelled, 0 resolved); precision 2 resolved of 3 labelled. A run from
    /// before the warnings were recorded counts in no sum, and alone it says so.
    #[test]
    fn the_warnings_add_up() {
        let warned = |given, labelled, resolved| crate::warned::Warned {
            given,
            labelled,
            resolved,
        };
        let of = |task: &str, warnings: Option<Warnings>| TaskRun {
            task: task.to_owned(),
            warnings,
            ..run(true, (1, 1, 1), Given::default())
        };
        let runs = [
            of(
                "a",
                Some(Warnings::from([("call_site".to_owned(), warned(2, 2, 2))])),
            ),
            of(
                "b",
                Some(Warnings::from([
                    ("cochange".to_owned(), warned(1, 0, 0)),
                    ("test".to_owned(), warned(1, 1, 0)),
                ])),
            ),
            of("c", None),
        ];
        let summary = summary(&runs, &[]);
        assert!(
            summary
                .warning_precision
                .is_some_and(|precision| (precision.value - 2.0 / 3.0).abs() < 1e-9)
        );
        assert!(lines(&summary).iter().any(|line| line.starts_with(
            "warnings after edits: call_site 2 (2 labelled, 2 resolved), cochange 1 (0 labelled, 0 \
             resolved), test 1 (1 labelled, 0 resolved); precision: 0.67"
        )));
        let before = super::summary(&runs[2..], &[]);
        assert!(lines(&before).contains(&"warnings after edits: not recorded".to_owned()));
    }

    /// With no work order labelled there is no precision, said as none.
    #[test]
    fn no_label_means_no_precision() {
        let summary = summary(&[run(true, (1, 1, 1), Given::default())], &[]);
        assert_eq!(summary.precision, None);
        assert!(lines(&summary).contains(&"precision: none".to_owned()));
    }

    /// With no run completed, nothing was spent: the cost is zero and printed as $0.00.
    #[test]
    fn no_run_costs_nothing() {
        let summary = summary(&[], &[]);
        assert!(summary.cost_usd == 0.0 && summary.cost_usd.is_sign_positive());
        assert!(
            lines(&summary)
                .contains(&"cost of every session, as the agent reported it: $0.00".to_owned())
        );
    }

    /// Runs that noted 3 rules and 2, and one whose arm left them out, add up to 5 rules noted and
    /// 1 run without; runs from before tasks had rules give no such line.
    #[test]
    fn the_rules_noted_are_counted() {
        let with = |facts, notes| TaskRun {
            facts,
            notes,
            ..run(true, (100, 1, 1), Given::default())
        };
        let runs = [
            with(Some(true), 3),
            with(Some(true), 2),
            with(Some(false), 0),
        ];
        let noted = summary(&runs, &[]);
        assert_eq!((noted.notes, noted.without_facts), (5, 1));
        assert_eq!(
            lines(&noted).last().map(String::as_str),
            Some("project rules noted before the first sessions: 5; runs that left them out: 1")
        );
        let older = summary(&[with(None, 0)], &[]);
        assert!(
            lines(&older)
                .last()
                .is_some_and(|line| line.starts_with("agents: "))
        );
    }
}
