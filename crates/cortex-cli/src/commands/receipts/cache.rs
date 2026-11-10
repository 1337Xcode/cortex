//! A session's prompt-cache health as lines of text: the share of its input read from the cache,
//! the share of its estimated cost spent writing to it, how often its conversations lost the cache
//! and why, and the losses nothing known explains, listed one by one.

use cortex_meter::{BreakCause, CacheBreak, SessionReceipt, Tally};

use super::text::{counted, when};

/// The most breaks with no known cause listed by turn; the rest are counted.
const MAX_LISTED_BREAKS: usize = 5;

/// The causes a break is counted under, in the order the line names them.
const CAUSES: [BreakCause; 4] = [
    BreakCause::ModelSwitch,
    BreakCause::Compaction,
    BreakCause::IdleGap,
    BreakCause::HookText,
];

/// The cache lines of `session`: none when its usage is not read or it recorded no response.
pub(super) fn cache_lines(session: &SessionReceipt) -> Vec<String> {
    let total = &session.total;
    if !session.is_metered || total.turns == 0 {
        return Vec::new();
    }
    let mut parts = vec![format!(
        "{} of input read from it",
        percent(
            u128::from(total.cache_reads),
            u128::from(total.cumulative_input)
        )
    )];
    parts.extend(write_share(total));
    parts.push(losses(&session.cache_breaks));
    let mut lines = vec![format!("  prompt cache: {}", parts.join("; "))];
    let unexplained: Vec<&CacheBreak> = session
        .cache_breaks
        .iter()
        .filter(|found| found.cause.is_none())
        .collect();
    if !unexplained.is_empty() {
        lines.push(unexplained_line(session, &unexplained));
    }
    lines
}

/// The share of `total`'s estimated cost spent on cache writes, when every response has a price
/// and the cost is more than nothing: a share of the priced ones alone would say nothing of the
/// session.
fn write_share(total: &Tally) -> Option<String> {
    let estimate = total.estimate.as_ref()?;
    let is_whole = estimate.priced_turns == total.turns && estimate.picodollars > 0;
    is_whole.then(|| {
        format!(
            "writing to it about {} of the estimated cost",
            percent(estimate.cache_write_picodollars, estimate.picodollars)
        )
    })
}

/// How often the cache was lost, by cause, the unexplained last.
fn losses(breaks: &[CacheBreak]) -> String {
    if breaks.is_empty() {
        return "never lost".to_owned();
    }
    let count_of = |cause: Option<BreakCause>| {
        breaks
            .iter()
            .filter(|found| found.cause == cause)
            .fold(0, |count: u64, _| count.saturating_add(1))
    };
    let causes: Vec<String> = CAUSES
        .iter()
        .map(|cause| Some(*cause))
        .chain([None])
        .filter_map(|cause| {
            let count = count_of(cause);
            (count > 0).then(|| format!("{count} {}", reason(cause)))
        })
        .collect();
    let times = breaks
        .iter()
        .fold(0, |count: u64, _| count.saturating_add(1));
    format!(
        "lost {} ({})",
        counted(times, "time", "times"),
        causes.join(", ")
    )
}

/// What a count of breaks with `cause` reads as after the count.
fn reason(cause: Option<BreakCause>) -> &'static str {
    match cause {
        Some(BreakCause::ModelSwitch) => "after a model switch",
        Some(BreakCause::Compaction) => "after a compaction",
        Some(BreakCause::IdleGap) => "after idling past the cache's lifetime",
        Some(BreakCause::HookText) => "after text Cortex gave",
        None => "with no known cause",
    }
}

/// The line listing the breaks of `session` with no known cause, `unexplained`, by turn and time,
/// a subagent's named, at most [`MAX_LISTED_BREAKS`] of them.
fn unexplained_line(session: &SessionReceipt, unexplained: &[&CacheBreak]) -> String {
    let listed: Vec<String> = unexplained
        .iter()
        .take(MAX_LISTED_BREAKS)
        .map(|found| {
            let of = if found.session_id == session.session_id {
                String::new()
            } else {
                format!(" of subagent {}", found.session_id.as_str())
            };
            format!("turn {}{of} at {}", found.turn, when(found.at_ms))
        })
        .collect();
    let rest = unexplained.len().saturating_sub(MAX_LISTED_BREAKS);
    let more = if rest > 0 {
        format!("; and {rest} more")
    } else {
        String::new()
    };
    format!(
        "  cache lost with no known cause: {}{more}",
        listed.join("; ")
    )
}

/// `part` of `whole` as a whole percentage rounded half up, or `under 1%` when it rounds to none
/// but is not, so that a share never reads as nothing.
fn percent(part: u128, whole: u128) -> String {
    if whole == 0 {
        return "0%".to_owned();
    }
    let rounded = part.saturating_mul(100).saturating_add(whole / 2) / whole;
    if rounded == 0 && part > 0 {
        return "under 1%".to_owned();
    }
    format!("{rounded}%")
}

#[cfg(test)]
mod tests {
    use cortex_core::Identifier;
    use cortex_meter::{BreakCause, CacheBreak, Estimate, SessionReceipt, Tally};

    use super::{cache_lines, percent};

    /// What a test returns: any error fails it.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A session `s1` of Claude Code with `total` and `cache_breaks`, metered.
    fn session(total: Tally, cache_breaks: Vec<CacheBreak>) -> TestResult<SessionReceipt> {
        Ok(SessionReceipt {
            session_id: Identifier::new("s1")?,
            agent: "claude",
            is_metered: true,
            started_at_ms: 0,
            ended_at_ms: None,
            status: "live",
            subagents: 1,
            tool_calls: 0,
            tasks: Vec::new(),
            outside_tasks: total.clone(),
            outside_tool_calls: 0,
            total,
            compactions_left_out: 0,
            cache_breaks,
            primer_tokens: 0,
            injected_tokens: 0,
        })
    }

    /// Four responses: 9,000 of 10,000 input tokens read from the cache, 90%; writes cost 310 of
    /// 1,000 picodollars, 31%.
    fn total(estimate: Option<Estimate>) -> Tally {
        Tally {
            turns: 4,
            cumulative_input: 10_000,
            uncached_input: 200,
            cache_reads: 9_000,
            cache_writes: 800,
            output: 40,
            estimate,
        }
    }

    /// A break of `session` at `turn`, minute `minute` of 2026-11-10 from 10:00 UTC.
    fn lost(
        session: &str,
        turn: u32,
        minute: i64,
        cause: Option<BreakCause>,
    ) -> TestResult<CacheBreak> {
        // 2026-11-10 10:00 UTC is 1,794,304,800 seconds after 1970.
        Ok(CacheBreak {
            session_id: Identifier::new(session)?,
            turn,
            at_ms: 1_794_304_800_000 + minute * 60_000,
            cause,
        })
    }

    /// A session that never lost its cache, priced whole, gives both shares and says so.
    #[test]
    fn a_session_that_kept_its_cache_shows_both_shares() -> TestResult {
        let priced = Estimate {
            picodollars: 1_000,
            cache_write_picodollars: 310,
            priced_turns: 4,
            ..Estimate::default()
        };
        let lines = cache_lines(&session(total(Some(priced)), Vec::new())?);
        assert_eq!(
            lines,
            [
                "  prompt cache: 90% of input read from it; writing to it about 31% of the \
              estimated cost; never lost"
            ]
        );
        Ok(())
    }

    /// Without a price table, or with a response unpriced, the write share is left out. Three
    /// breaks after a switch, an idle gap and with no cause are counted by cause, the unexplained
    /// last, and the unexplained one, a subagent's, is listed by turn and minute.
    #[test]
    fn breaks_are_counted_by_cause_and_the_unexplained_listed() -> TestResult {
        let partly = Estimate {
            picodollars: 1_000,
            cache_write_picodollars: 310,
            priced_turns: 3,
            ..Estimate::default()
        };
        let breaks = vec![
            lost("s1", 3, 1, Some(BreakCause::IdleGap))?,
            lost("s1/agent-1", 2, 7, None)?,
            lost("s1", 9, 12, Some(BreakCause::ModelSwitch))?,
        ];
        let expected = [
            "  prompt cache: 90% of input read from it; lost 3 times (1 after a model switch, 1 \
             after idling past the cache's lifetime, 1 with no known cause)",
            "  cache lost with no known cause: turn 2 of subagent s1/agent-1 at 2026-11-10 10:07 \
             UTC",
        ];
        assert_eq!(
            cache_lines(&session(total(None), breaks.clone())?),
            expected
        );
        assert_eq!(
            cache_lines(&session(total(Some(partly)), breaks)?),
            expected
        );
        Ok(())
    }

    /// Six breaks with no cause list the first five and count the sixth.
    #[test]
    fn at_most_five_unexplained_breaks_are_listed() -> TestResult {
        let breaks = (1..=6)
            .map(|turn| lost("s1", turn, i64::from(turn), None))
            .collect::<Result<Vec<_>, _>>()?;
        let lines = cache_lines(&session(total(None), breaks)?);
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some(
                "  cache lost with no known cause: turn 1 at 2026-11-10 10:01 UTC; turn 2 at \
                 2026-11-10 10:02 UTC; turn 3 at 2026-11-10 10:03 UTC; turn 4 at 2026-11-10 10:04 \
                 UTC; turn 5 at 2026-11-10 10:05 UTC; and 1 more"
            )
        );
        Ok(())
    }

    /// A session not metered, or with no response, has no cache lines.
    #[test]
    fn no_usage_gives_no_cache_lines() -> TestResult {
        let mut unread = session(total(None), Vec::new())?;
        unread.is_metered = false;
        assert!(cache_lines(&unread).is_empty());
        let empty = Tally {
            turns: 0,
            ..total(None)
        };
        assert!(cache_lines(&session(empty, Vec::new())?).is_empty());
        Ok(())
    }

    /// Shares round half up to a whole percent: 1 of 200 is 0.5%, shown as 1%; 1 of 201 rounds to
    /// none and shows as under 1%; nothing of something is 0%, and a share of nothing is 0%.
    #[test]
    fn percentages_round_half_up_and_never_read_as_nothing() {
        assert_eq!(percent(1, 200), "1%");
        assert_eq!(percent(1, 201), "under 1%");
        assert_eq!(percent(0, 7), "0%");
        assert_eq!(percent(0, 0), "0%");
        assert_eq!(percent(2, 3), "67%");
    }
}
