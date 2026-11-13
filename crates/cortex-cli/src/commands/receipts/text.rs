//! The receipts as lines of text: what the costs were estimated from, how far the held-out tasks
//! are from being enough to compare with the rest, how precise the work orders given were, then
//! each session with its primers, its tasks, the work outside any task, its total, the
//! compactions its total leaves out, and how well it used the prompt cache.

use std::path::Path;

use cortex_meter::{
    Compared, Estimate, Precision, SessionReceipt, Tally, TaskReceipt, Unpriced, utc_text,
};

use super::cache::cache_lines;
use cortex_meter::Holdout;

use super::{Prices, Report};

/// A hundredth of a cent, the smallest amount receipts show, in picodollars.
const HUNDREDTH_OF_A_CENT_PICODOLLARS: u128 = 100_000_000;

/// The lines of `report` of the repository at `root`, whose sessions are those of the last `days`
/// days when it lists them by time.
pub(super) fn lines(report: &Report, root: &Path, days: Option<u32>) -> Vec<String> {
    let mut lines = vec![prices_line(&report.prices)];
    let Some(recorded) = &report.recorded else {
        lines.push(format!(
            "nothing recorded: {} has no store yet; `cortex daemon` builds one",
            root.display()
        ));
        return lines;
    };
    lines.extend(holdout_lines(&recorded.holdout));
    lines.push(precision_line(recorded.precision));
    lines.push(warning_line(recorded.warnings));
    if recorded.sessions.is_empty() {
        lines.push(match days {
            Some(days) => format!(
                "no session started in the last {}",
                counted(u64::from(days), "day", "days")
            ),
            None => "no session".to_owned(),
        });
    }
    for session in &recorded.sessions {
        lines.extend(session_lines(session));
    }
    lines
}

/// The line saying what the costs were estimated from.
fn prices_line(prices: &Prices) -> String {
    match prices {
        Prices::Read { file, updated, .. } => format!("prices: as of {updated}, from {file}"),
        Prices::Missing { why } => format!("prices: none, so no cost is estimated ({why})"),
    }
}

/// The lines saying how the held-out tasks compare with the rest, or how many of each there are
/// so far against how many the comparison needs.
fn holdout_lines(holdout: &Holdout) -> Vec<String> {
    let needed = holdout.needed_per_arm;
    let (held_out, treated) = (holdout.arms.held_out, holdout.arms.treated);
    if !holdout.arms.is_enough() {
        return vec![format!(
            "holdout comparison: {} of {needed} held-out and {} of {needed} treated tasks so far",
            grouped(held_out),
            grouped(treated)
        )];
    }
    let Some(comparison) = &holdout.comparison else {
        return vec![format!(
            "holdout comparison: {} held-out and {} treated tasks so far, but fewer than {needed} \
             of each have ended",
            grouped(held_out),
            grouped(treated)
        )];
    };
    let [treated, held_out] = comparison.tasks;
    let mut lines = vec![format!(
        "holdout comparison: {} ended with Cortex and {} held out; each difference is with Cortex \
         minus held out, its 95% bootstrap interval in brackets",
        counted(treated, "task", "tasks"),
        grouped(held_out)
    )];
    lines.push(compared_line(
        "success rate",
        &comparison.success_rate,
        (share, signed_share),
    ));
    let [treated_succeeded, held_out_succeeded] = comparison.per_success_tasks;
    match &comparison.per_success {
        Some([input, uncached, turns, calls]) => {
            for (what, compared) in [
                ("median input tokens per successful task", input),
                ("median uncached input tokens per successful task", uncached),
                ("median turns per successful task", turns),
                ("median tool calls per successful task", calls),
            ] {
                lines.push(compared_line(what, compared, (quantity, signed_quantity)));
            }
            lines.push(format!(
                "  medians from {} with Cortex and {} held out whose usage was read",
                counted(treated_succeeded, "successful task", "successful tasks"),
                grouped(held_out_succeeded)
            ));
        }
        None => lines.push(format!(
            "  per successful task: {} of {needed} with Cortex and {} of {needed} held out have \
             usage read so far",
            grouped(treated_succeeded),
            grouped(held_out_succeeded)
        )),
    }
    lines
}

/// How a measure is written: its value, and a difference or a bound of its interval, signed.
type Writers = (fn(f64) -> String, fn(f64) -> String);

/// One measure of the comparison as a line: each arm's value written by `number`, the difference
/// and its interval by `signed`.
fn compared_line(what: &str, compared: &Compared, (number, signed): Writers) -> String {
    format!(
        "  {what}: {} with Cortex, {} held out, {} ({} to {})",
        number(compared.treated),
        number(compared.held_out),
        signed(compared.difference),
        signed(compared.low),
        signed(compared.high)
    )
}

/// A share from 0 to 1 as a decimal of two places, never written as one of the ends when it is
/// not: "0" and "1" for the ends themselves, "under 0.01" below 0.005 and "over 0.99" from 0.995,
/// as the map writes it.
fn share(value: f64) -> String {
    if value <= 0.0 {
        "0".to_owned()
    } else if value >= 1.0 {
        "1".to_owned()
    } else if value < 0.005 {
        "under 0.01".to_owned()
    } else if value >= 0.995 {
        "over 0.99".to_owned()
    } else {
        format!("{value:.2}")
    }
}

/// A difference of shares, from -1 to 1, with its sign, by the same rule: "+0.17", "-0.08", "0"
/// for none, and a bound on its side of 0 for one that would round to 0 or 1 either way
/// ("under +0.01", "over -0.01", "over +0.99", "under -0.99").
fn signed_share(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let is_up = value > 0.0;
    match share(value.abs()).as_str() {
        "under 0.01" => if is_up { "under +0.01" } else { "over -0.01" }.to_owned(),
        "over 0.99" => if is_up { "over +0.99" } else { "under -0.99" }.to_owned(),
        size => format!("{}{size}", if is_up { "+" } else { "-" }),
    }
}

/// A median or a difference of medians as [`quantity`] writes it, with a `+` when above 0.
fn signed_quantity(value: f64) -> String {
    let text = quantity(value);
    if value > 0.0 {
        format!("+{text}")
    } else {
        text
    }
}

/// A median or a difference of medians, grouped in threes, with its tenths when it has any (a
/// median of an even count of whole numbers can end in a half).
pub(in crate::commands) fn quantity(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let tenths = (value.abs() * 10.0).round() as u64;
    let whole = grouped(tenths / 10);
    match tenths % 10 {
        0 => format!("{sign}{whole}"),
        tenth => format!("{sign}{whole}.{tenth}"),
    }
}

/// The line saying how precise the work orders given were.
fn precision_line(precision: Precision) -> String {
    match precision.share() {
        None => "injection precision: no work order given is labelled yet".to_owned(),
        Some(precision_share) => format!(
            "injection precision: {}, {} of {} given and labelled were used",
            share(precision_share),
            grouped(precision.used),
            counted(precision.labelled, "work order", "work orders")
        ),
    }
}

/// The line saying how often the files warnings named were then read or edited.
fn warning_line(precision: Precision) -> String {
    match precision.share() {
        None => "warning precision: no warning is marked yet".to_owned(),
        Some(precision_share) => format!(
            "warning precision: {}, {} of {} warnings named and marked were then read or \
             edited",
            share(precision_share),
            grouped(precision.used),
            counted(precision.labelled, "file", "files")
        ),
    }
}

/// The lines of one session: itself, each task, the work outside any task, its total, the
/// compactions the total leaves out, and its prompt cache's health. A session whose agent is not metered leaves usage out of
/// every line, as Cortex reads none.
fn session_lines(session: &SessionReceipt) -> Vec<String> {
    let usage = |tally: &Tally| session.is_metered.then(|| usage(tally));
    let mut lines = vec![format!(
        "session {} ({}, {}), started {}, {}",
        session.session_id.as_str(),
        session.agent,
        session.status,
        when(session.started_at_ms),
        counted(session.subagents, "subagent", "subagents")
    )];
    if session.primer_tokens > 0 {
        lines.push(format!(
            "  primer: {} injected by Cortex",
            counted(session.primer_tokens, "token", "tokens")
        ));
    }
    lines.extend(
        session
            .tasks
            .iter()
            .map(|task| task_line(task, usage(&task.usage))),
    );
    if session.outside_tasks.turns > 0 || session.outside_tool_calls > 0 {
        let calls = counted(session.outside_tool_calls, "tool call", "tool calls");
        lines.push(format!(
            "  outside any task: {}",
            joined(usage(&session.outside_tasks), vec![calls])
        ));
    }
    let rest = vec![
        counted(session.tool_calls, "tool call", "tool calls"),
        format!(
            "{} injected by Cortex",
            counted(session.injected_tokens, "token", "tokens")
        ),
    ];
    lines.push(format!("  total: {}", joined(usage(&session.total), rest)));
    if session.compactions_left_out > 0 {
        lines.push(format!(
            "  not in the total: {}, which the transcript does not record",
            counted(session.compactions_left_out, "compaction", "compactions")
        ));
    }
    lines.extend(cache_lines(session));
    lines
}

/// The line of one task, whose usage reads `usage` when its agent is metered.
fn task_line(task: &TaskReceipt, usage: Option<String>) -> String {
    let state = match (task.outcome, task.ended_at_ms) {
        (Some(outcome), _) => outcome,
        (None, Some(_)) => "ended",
        (None, None) => "open",
    };
    let rest = vec![
        counted(task.tool_calls, "tool call", "tool calls"),
        format!(
            "{} injected by Cortex",
            counted(task.injected_tokens, "token", "tokens")
        ),
    ];
    format!(
        "  task {} ({state}), started {}: {}",
        task.seq,
        when(task.started_at_ms),
        joined(usage, rest)
    )
}

/// The parts of a line, `usage` first when there is any, separated by semicolons.
fn joined(usage: Option<String>, rest: Vec<String>) -> String {
    let mut parts: Vec<String> = usage.into_iter().collect();
    parts.extend(rest);
    parts.join("; ")
}

/// The responses of `tally` and what they cost, as words; that none was recorded when there is
/// none, rather than counts of nothing.
fn usage(tally: &Tally) -> String {
    if tally.turns == 0 {
        return "no usage recorded".to_owned();
    }
    let tokens = format!(
        "{}, {} input tokens ({} uncached, {} read from the cache, {} written to it), {}",
        counted(tally.turns, "turn", "turns"),
        grouped(tally.cumulative_input),
        grouped(tally.uncached_input),
        grouped(tally.cache_reads),
        grouped(tally.cache_writes),
        counted(tally.output, "output token", "output tokens")
    );
    match &tally.estimate {
        Some(estimate) => format!("{tokens}, {}", cost(estimate, tally.turns)),
        None => tokens,
    }
}

/// The estimated cost of `turns` responses, one or more, and why any of them have none.
fn cost(estimate: &Estimate, turns: u64) -> String {
    let reasons: Vec<String> = estimate
        .unpriced_turns
        .iter()
        .map(|(reason, count)| format!("{} {}", grouped(*count), unpriced(*reason)))
        .collect();
    if estimate.priced_turns == 0 {
        return format!("no cost estimated ({})", reasons.join(", "));
    }
    let amount = amount(estimate.picodollars);
    if estimate.priced_turns == turns {
        return amount;
    }
    format!(
        "{amount} for {} of them ({})",
        grouped(estimate.priced_turns),
        reasons.join(", ")
    )
}

/// Why responses have no estimate, after their count.
fn unpriced(reason: Unpriced) -> &'static str {
    match reason {
        Unpriced::Model => "with no price for their model",
        Unpriced::Tier => "not at the standard tier",
        Unpriced::Speed => "with no price for their speed",
        Unpriced::Region => "with no price where they ran",
        Unpriced::CacheSplit => "whose cache writes are not split by lifetime",
    }
}

/// An estimate of `picodollars` in words: dollars rounded half up to a hundredth of a cent, or
/// that it is less than that, so that a cost never reads as nothing.
fn amount(picodollars: u128) -> String {
    let hundredths_of_cents = picodollars.saturating_add(HUNDREDTH_OF_A_CENT_PICODOLLARS / 2)
        / HUNDREDTH_OF_A_CENT_PICODOLLARS;
    if picodollars > 0 && hundredths_of_cents == 0 {
        return "less than $0.0001".to_owned();
    }
    format!(
        "about ${}.{:04}",
        hundredths_of_cents / 10_000,
        hundredths_of_cents % 10_000
    )
}

/// The UTC minute `millis` falls in, as `2026-11-03 07:12 UTC`, or the milliseconds themselves
/// when they name no time the form can write.
pub(super) fn when(millis: i64) -> String {
    let Some(text) = utc_text(millis) else {
        return format!("{millis} ms after 1970");
    };
    match (text.get(..10), text.get(11..16)) {
        (Some(day), Some(minute)) => format!("{day} {minute} UTC"),
        _ => text,
    }
}

/// `count` followed by `one` or `many`, its digits grouped.
pub(super) fn counted(count: u64, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{} {noun}", grouped(count))
}

/// `number` with its digits grouped in threes, as `153,282`.
pub(in crate::commands) fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut text = String::with_capacity(digits.len() * 4 / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use cortex_core::{Identifier, Ulid};
    use cortex_meter::{
        Arms, Compared, Comparison, Estimate, Precision, SessionReceipt, Tally, TaskReceipt,
        Unpriced,
    };

    use cortex_meter::{Holdout, Recorded};

    use super::super::{Prices, Report};
    use super::{
        amount, grouped, holdout_lines, lines, session_lines, share, signed_share, usage, when,
    };

    /// A share is never written as an end it is not: 0 and 1 themselves, 0.004 as under 0.01,
    /// 0.996 as over 0.99, 0.5 as 0.50, 0.005 as 0.01; a difference keeps its side of 0 by the
    /// same rule.
    #[test]
    fn a_share_is_never_rounded_to_an_end() {
        let shares: Vec<String> = [0.0, 1.0, 0.004, 0.996, 0.5, 0.005]
            .into_iter()
            .map(share)
            .collect();
        assert_eq!(
            shares,
            ["0", "1", "under 0.01", "over 0.99", "0.50", "0.01"]
        );
        let differences: Vec<String> = [0.0, 0.004, -0.004, 0.996, -0.996, 0.17, -0.08]
            .into_iter()
            .map(signed_share)
            .collect();
        assert_eq!(
            differences,
            [
                "0",
                "under +0.01",
                "over -0.01",
                "over +0.99",
                "under -0.99",
                "+0.17",
                "-0.08"
            ]
        );
    }
    /// Digits grouped by hand.
    #[test]
    fn numbers_are_grouped_in_threes() {
        let cases = [
            (0, "0"),
            (999, "999"),
            (1_000, "1,000"),
            (153_282, "153,282"),
        ];
        for (number, text) in cases {
            assert_eq!(grouped(number), text);
        }
        assert_eq!(grouped(u64::MAX), "18,446,744,073,709,551,615");
    }

    /// Dollars rounded by hand: 109,168,100,000 picodollars are $0.10916..., 50,000,000 round up
    /// to a hundredth of a cent and 49,999,999 to nothing, which is said to be less than that;
    /// only no cost at all is $0.0000.
    #[test]
    fn picodollars_are_rounded_to_hundredths_of_cents() {
        assert_eq!(amount(109_168_100_000), "about $0.1092");
        assert_eq!(amount(50_000_000), "about $0.0001");
        assert_eq!(amount(49_999_999), "less than $0.0001");
        assert_eq!(amount(1), "less than $0.0001");
        assert_eq!(amount(0), "about $0.0000");
        assert_eq!(amount(12_345_678_900_000_000), "about $12345.6789");
    }

    /// No response recorded reads as none, not as counts and a cost of nothing; responses none
    /// of which has a price say why.
    #[test]
    fn usage_without_responses_or_prices_says_so() {
        let tally = |turns, estimate| Tally {
            turns,
            cumulative_input: 7 * turns,
            uncached_input: 7 * turns,
            cache_reads: 0,
            cache_writes: 0,
            output: turns,
            estimate,
        };
        assert_eq!(
            usage(&tally(0, Some(Estimate::default()))),
            "no usage recorded"
        );
        assert_eq!(usage(&tally(0, None)), "no usage recorded");
        let unpriced = Estimate {
            picodollars: 0,
            cache_write_picodollars: 0,
            priced_turns: 0,
            unpriced_turns: BTreeMap::from([(Unpriced::Model, 2)]),
        };
        assert_eq!(
            usage(&tally(2, Some(unpriced))),
            "2 turns, 14 input tokens (14 uncached, 0 read from the cache, 0 written to it), 2 \
             output tokens, no cost estimated (2 with no price for their model)"
        );
    }

    /// A store with no session in the window says so after what the costs come from, the arms
    /// and the precisions: 52 used of 73 labelled is 0.7123, shown as 0.71; 5 of 9 files warnings
    /// named followed up is 0.5556, shown as 0.56.
    #[test]
    fn a_window_without_sessions_says_so() {
        let report = Report {
            prices: Prices::Missing {
                why: "no home folder".to_owned(),
            },
            recorded: Some(Recorded {
                holdout: Holdout {
                    arms: Arms {
                        held_out: 31,
                        treated: 1_200,
                    },
                    needed_per_arm: 30,
                    comparison: None,
                },
                precision: Precision {
                    labelled: 73,
                    used: 52,
                },
                warnings: Precision {
                    labelled: 9,
                    used: 5,
                },
                sessions: Vec::new(),
            }),
        };
        assert_eq!(
            lines(&report, Path::new("repo"), Some(2)),
            [
                "prices: none, so no cost is estimated (no home folder)",
                "holdout comparison: 31 held-out and 1,200 treated tasks so far, but fewer than \
                 30 of each have ended",
                "injection precision: 0.71, 52 of 73 work orders given and labelled were used",
                "warning precision: 0.56, 5 of 9 files warnings named and marked were then read \
                 or edited",
                "no session started in the last 2 days",
            ]
        );
        assert_eq!(lines(&report, Path::new("repo"), None)[4], "no session");
    }

    /// A session of an agent whose transcripts Cortex does not read leaves usage out of each task,
    /// the work outside any task and its total, where a metered session with no response says
    /// none was recorded.
    #[test]
    fn a_session_not_metered_leaves_its_usage_out() -> Result<(), Box<dyn std::error::Error>> {
        let nothing = Tally {
            turns: 0,
            cumulative_input: 0,
            uncached_input: 0,
            cache_reads: 0,
            cache_writes: 0,
            output: 0,
            estimate: None,
        };
        let task = TaskReceipt {
            task_id: Ulid::from_seed(0, b"task"),
            seq: 1,
            started_at_ms: 60_000,
            ended_at_ms: Some(120_000),
            outcome: None,
            held_out: None,
            tool_calls: 2,
            injected_tokens: 0,
            usage: nothing.clone(),
        };
        let codex = SessionReceipt {
            session_id: Identifier::new("s1")?,
            agent: "codex",
            is_metered: false,
            started_at_ms: 0,
            ended_at_ms: Some(180_000),
            status: "ended",
            subagents: 0,
            tool_calls: 3,
            tasks: vec![task],
            outside_tasks: nothing.clone(),
            outside_tool_calls: 1,
            total: nothing,
            compactions_left_out: 0,
            cache_breaks: Vec::new(),
            primer_tokens: 0,
            injected_tokens: 0,
        };
        assert_eq!(
            session_lines(&codex),
            [
                "session s1 (codex, ended), started 1970-01-01 00:00 UTC, 0 subagents",
                "  task 1 (ended), started 1970-01-01 00:01 UTC: 2 tool calls; 0 tokens injected \
                 by Cortex",
                "  outside any task: 1 tool call",
                "  total: 3 tool calls; 0 tokens injected by Cortex",
            ]
        );
        let claude = SessionReceipt {
            agent: "claude",
            is_metered: true,
            compactions_left_out: 1,
            ..codex
        };
        let said = session_lines(&claude);
        assert_eq!(
            said[2..],
            [
                "  outside any task: no usage recorded; 1 tool call",
                "  total: no usage recorded; 3 tool calls; 0 tokens injected by Cortex",
                "  not in the total: 1 compaction, which the transcript does not record",
            ]
        );
        Ok(())
    }

    /// A comparison as lines, its numbers chosen here: rates to two places with a sign on each
    /// difference and interval end above zero (1/6 is +0.17, -1/12 is -0.08, 1/3 is 0.33),
    /// medians grouped in threes with a tenth when they have one (12.5, -7.5, +0.5), a zero
    /// unsigned.
    #[test]
    fn a_comparison_reads_as_one_line_a_measure() {
        let measure = |treated, held_out, difference, low, high| Compared {
            treated,
            held_out,
            difference,
            low,
            high,
        };
        let holdout = Holdout {
            arms: Arms {
                held_out: 31,
                treated: 40,
            },
            needed_per_arm: 30,
            comparison: Some(Comparison {
                tasks: [40, 31],
                success_rate: measure(0.5, 1.0 / 3.0, 1.0 / 6.0, -1.0 / 12.0, 0.4),
                per_success_tasks: [30, 30],
                per_success: Some([
                    measure(800.0, 6_500.0, -5_700.0, -9_000.0, -2_100.0),
                    measure(12.5, 20.0, -7.5, -15.0, 0.0),
                    measure(3.0, 4.0, -1.0, -2.0, 0.5),
                    measure(0.0, 0.0, 0.0, 0.0, 0.0),
                ]),
            }),
        };
        assert_eq!(
            holdout_lines(&holdout),
            [
                "holdout comparison: 40 tasks ended with Cortex and 31 held out; each difference \
                 is with Cortex minus held out, its 95% bootstrap interval in brackets",
                "  success rate: 0.50 with Cortex, 0.33 held out, +0.17 (-0.08 to +0.40)",
                "  median input tokens per successful task: 800 with Cortex, 6,500 held out, \
                 -5,700 (-9,000 to -2,100)",
                "  median uncached input tokens per successful task: 12.5 with Cortex, 20 held \
                 out, -7.5 (-15 to 0)",
                "  median turns per successful task: 3 with Cortex, 4 held out, -1 (-2 to +0.5)",
                "  median tool calls per successful task: 0 with Cortex, 0 held out, 0 (0 to 0)",
                "  medians from 30 successful tasks with Cortex and 30 held out whose usage was \
                 read",
            ]
        );
        let mut thin = holdout.clone();
        if let Some(comparison) = thin.comparison.as_mut() {
            comparison.per_success_tasks = [12, 30];
            comparison.per_success = None;
        }
        assert_eq!(
            holdout_lines(&thin).last().map(String::as_str),
            Some(
                "  per successful task: 12 of 30 with Cortex and 30 of 30 held out have usage \
                 read so far"
            )
        );
        let few = Holdout {
            arms: Arms {
                held_out: 4,
                treated: 12,
            },
            needed_per_arm: 30,
            comparison: None,
        };
        assert_eq!(
            holdout_lines(&few),
            ["holdout comparison: 4 of 30 held-out and 12 of 30 treated tasks so far"]
        );
    }

    /// Times by hand: 2000-03-01 is 11,017 days after 1970-01-01, and 3,723,004 ms is 01:02.
    #[test]
    fn times_are_shown_to_the_minute() {
        assert_eq!(
            when(11_017 * 86_400_000 + 3_723_004),
            "2000-03-01 01:02 UTC"
        );
        assert_eq!(when(-5), "-5 ms after 1970");
    }
}
