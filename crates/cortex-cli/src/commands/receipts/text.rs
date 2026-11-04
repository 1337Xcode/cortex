//! The receipts as lines of text: what the costs were estimated from, what the counts leave out,
//! whether there is enough to compare held-out tasks with the rest, how precise the work orders
//! given were, then each session with its primers, its tasks, the work outside any task, and its
//! total.

use std::path::Path;

use cortex_meter::{Estimate, Precision, SessionReceipt, Tally, TaskReceipt, Unpriced, utc_text};

use super::{Holdout, Prices, Report};

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
    lines.push(format!(
        "usage: as each transcript records it; {}, are not counted",
        report.uncounted
    ));
    lines.push(holdout_line(&recorded.holdout));
    lines.push(precision_line(recorded.precision));
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
        Prices::Read {
            file,
            updated,
            source,
        } => {
            let from = source
                .as_ref()
                .map(|source| format!(", read from {source}"))
                .unwrap_or_default();
            format!(
                "prices: as of {updated}{from}, kept in {file}; costs are estimates and count \
                 tokens only"
            )
        }
        Prices::Missing { why } => format!("prices: none, so no cost is estimated ({why})"),
    }
}

/// The line saying whether held-out tasks can be compared with the rest yet.
fn holdout_line(holdout: &Holdout) -> String {
    let counts = format!(
        "the repository has {} held out and {} not",
        counted(holdout.arms.held_out, "task", "tasks"),
        grouped(holdout.arms.treated)
    );
    if holdout.arms.is_enough() {
        return format!("holdout comparison: {counts}; this version does not compare them");
    }
    format!(
        "holdout comparison: not enough data yet ({counts}; {} of each are needed)",
        holdout.needed_per_arm
    )
}

/// The line saying how precise the work orders given were.
fn precision_line(precision: Precision) -> String {
    match precision.share() {
        None => "injection precision: no work order given is labelled yet".to_owned(),
        Some(share) => format!(
            "injection precision: {share:.2}, {} of {} given and labelled were used",
            grouped(precision.used),
            counted(precision.labelled, "work order", "work orders")
        ),
    }
}

/// What a session of an agent whose transcripts Cortex does not read shows for its usage.
const NOT_METERED: &str = "usage not read, as Cortex reads no transcript of this agent";

/// The lines of one session: itself, each task, the work outside any task, and its total. A
/// session whose agent is not metered says so in place of every count of usage.
fn session_lines(session: &SessionReceipt) -> Vec<String> {
    let usage = |tally: &Tally| {
        if session.is_metered {
            usage(tally)
        } else {
            NOT_METERED.to_owned()
        }
    };
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
        lines.push(format!(
            "  outside any task: {}; {}",
            usage(&session.outside_tasks),
            counted(session.outside_tool_calls, "tool call", "tool calls")
        ));
    }
    lines.push(format!(
        "  total: {}; {}; {} injected by Cortex",
        usage(&session.total),
        counted(session.tool_calls, "tool call", "tool calls"),
        counted(session.injected_tokens, "token", "tokens")
    ));
    lines
}

/// The line of one task, whose usage reads `usage`.
fn task_line(task: &TaskReceipt, usage: String) -> String {
    let state = match (task.outcome, task.ended_at_ms) {
        (Some(outcome), _) => outcome,
        (None, Some(_)) => "ended",
        (None, None) => "open",
    };
    format!(
        "  task {} ({state}), started {}: {usage}; {}; {} injected by Cortex",
        task.seq,
        when(task.started_at_ms),
        counted(task.tool_calls, "tool call", "tool calls"),
        counted(task.injected_tokens, "token", "tokens")
    )
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
fn when(millis: i64) -> String {
    let Some(text) = utc_text(millis) else {
        return format!("{millis} ms after 1970");
    };
    match (text.get(..10), text.get(11..16)) {
        (Some(day), Some(minute)) => format!("{day} {minute} UTC"),
        _ => text,
    }
}

/// `count` followed by `one` or `many`, its digits grouped.
fn counted(count: u64, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{} {noun}", grouped(count))
}

/// `number` with its digits grouped in threes, as `153,282`.
fn grouped(number: u64) -> String {
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
    use cortex_meter::{Arms, Estimate, Precision, SessionReceipt, Tally, TaskReceipt, Unpriced};

    use super::super::{Holdout, Prices, Recorded, Report, UNCOUNTED};
    use super::{amount, grouped, lines, session_lines, usage, when};

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
    /// and the precision: 52 used of 73 labelled is 0.7123, shown as 0.71.
    #[test]
    fn a_window_without_sessions_says_so() {
        let report = Report {
            prices: Prices::Missing {
                why: "no home folder".to_owned(),
            },
            uncounted: UNCOUNTED,
            recorded: Some(Recorded {
                holdout: Holdout {
                    arms: Arms {
                        held_out: 31,
                        treated: 1_200,
                    },
                    needed_per_arm: 30,
                },
                precision: Precision {
                    labelled: 73,
                    used: 52,
                },
                sessions: Vec::new(),
            }),
        };
        assert_eq!(
            lines(&report, Path::new("repo"), Some(2)),
            [
                "prices: none, so no cost is estimated (no home folder)",
                "usage: as each transcript records it; calls a transcript does not record, such \
                 as compaction, are not counted",
                "holdout comparison: the repository has 31 tasks held out and 1,200 not; this \
                 version does not compare them",
                "injection precision: 0.71, 52 of 73 work orders given and labelled were used",
                "no session started in the last 2 days",
            ]
        );
        assert_eq!(lines(&report, Path::new("repo"), None)[4], "no session");
    }

    /// A session of an agent whose transcripts Cortex does not read says its usage was not read,
    /// for each task and in its total, where a metered session with no response says none was
    /// recorded.
    #[test]
    fn a_session_not_metered_says_its_usage_was_not_read() -> Result<(), Box<dyn std::error::Error>>
    {
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
            tool_calls: 2,
            tasks: vec![task],
            outside_tasks: nothing.clone(),
            outside_tool_calls: 0,
            total: nothing,
            primer_tokens: 0,
            injected_tokens: 0,
        };
        assert_eq!(
            session_lines(&codex),
            [
                "session s1 (codex, ended), started 1970-01-01 00:00 UTC, 0 subagents",
                "  task 1 (ended), started 1970-01-01 00:01 UTC: usage not read, as Cortex reads \
                 no transcript of this agent; 2 tool calls; 0 tokens injected by Cortex",
                "  total: usage not read, as Cortex reads no transcript of this agent; 2 tool \
                 calls; 0 tokens injected by Cortex",
            ]
        );
        let claude = SessionReceipt {
            agent: "claude",
            is_metered: true,
            ..codex
        };
        let said = session_lines(&claude);
        assert_eq!(
            said.get(2).map(String::as_str),
            Some("  total: no usage recorded; 2 tool calls; 0 tokens injected by Cortex")
        );
        Ok(())
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
