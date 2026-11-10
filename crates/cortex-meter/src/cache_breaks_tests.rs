//! Tests of [`super::cache_breaks`]. Expected values are worked by hand beside each test.

use std::collections::BTreeMap;

use cortex_core::Identifier;
use cortex_store::UsageRow;

use super::{
    BreakCause, BreakContext, CacheBreak, FIVE_MINUTE_LIFETIME_MS, ONE_HOUR_LIFETIME_MS,
    cache_breaks,
};

/// What a test returns: any error fails it.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A minute in milliseconds.
const MINUTE_MS: i64 = 60_000;

/// The response numbered `turn` of session `s1`, written at `at_ms` by `model`, with
/// `[input, cache writes, cache reads]` tokens, every cache write for five minutes.
fn call(
    turn: u32,
    at_ms: i64,
    model: &str,
    [input, cache_write, cache_read]: [u64; 3],
) -> TestResult<UsageRow> {
    Ok(UsageRow {
        session_id: Identifier::new("s1")?,
        turn,
        at_ms,
        model: Some(Identifier::new(model)?),
        input,
        cache_write,
        cache_write_1h: Some(0),
        cache_read,
        output: 50,
        reasoning: None,
        service_tier: None,
        speed: None,
        inference_geo: None,
        response: None,
    })
}

/// Two responses of `alpha`, a minute apart, the second of which lost the cache: it read 100 of
/// the previous response's 1,000 and wrote 900, which is more than half of its 1,000 cached input.
fn breaking_pair() -> TestResult<Vec<UsageRow>> {
    Ok(vec![
        call(1, 0, "alpha", [10, 0, 1_000])?,
        call(2, MINUTE_MS, "alpha", [10, 900, 100])?,
    ])
}

/// The break `breaking_pair` holds, with `cause`.
fn the_break(cause: Option<BreakCause>) -> TestResult<CacheBreak> {
    Ok(CacheBreak {
        session_id: Identifier::new("s1")?,
        turn: 2,
        at_ms: MINUTE_MS,
        cause,
    })
}

/// Session `s1`'s compactions at `times`.
fn compacted(times: &[i64]) -> TestResult<BTreeMap<Identifier, Vec<i64>>> {
    Ok(BTreeMap::from([(Identifier::new("s1")?, times.to_vec())]))
}

/// The breaks of `calls` with no compaction and no text given.
fn breaks_alone(calls: &[UsageRow]) -> Vec<CacheBreak> {
    let none = BTreeMap::new();
    let context = BreakContext {
        compactions_at_ms: &none,
        texts_given_at_ms: &[],
    };
    cache_breaks(calls, &context)
}

/// Three responses, `[input, writes, reads]` of `[100, 1000, 0]`, `[10, 100, 1000]` and
/// `[10, 100, 1100]`: none loses the cache, the second and third writing 100 of over 1,000 cached.
#[test]
fn a_session_that_extends_its_cache_has_no_breaks() -> TestResult {
    let calls = [
        call(1, 0, "alpha", [100, 1_000, 0])?,
        call(2, MINUTE_MS, "alpha", [10, 100, 1_000])?,
        call(3, 2 * MINUTE_MS, "alpha", [10, 100, 1_100])?,
    ];
    assert_eq!(breaks_alone(&calls), []);
    Ok(())
}

/// A different model on the break's response explains it.
#[test]
fn a_model_switch_is_the_cause_of_a_break_across_models() -> TestResult {
    let mut calls = breaking_pair()?;
    calls[1].model = Some(Identifier::new("beta")?);
    assert_eq!(
        breaks_alone(&calls),
        [the_break(Some(BreakCause::ModelSwitch))?]
    );
    Ok(())
}

/// A compaction of the session between the two responses explains the break.
#[test]
fn a_compaction_between_the_calls_is_the_cause() -> TestResult {
    let compactions = compacted(&[MINUTE_MS / 2])?;
    let context = BreakContext {
        compactions_at_ms: &compactions,
        texts_given_at_ms: &[],
    };
    let breaks = cache_breaks(&breaking_pair()?, &context);
    assert_eq!(breaks, [the_break(Some(BreakCause::Compaction))?]);
    Ok(())
}

/// Six minutes between the responses is over the five a cache lives.
#[test]
fn a_gap_of_six_minutes_is_the_cause() -> TestResult {
    let mut calls = breaking_pair()?;
    calls[1].at_ms = 6 * MINUTE_MS;
    let found = CacheBreak {
        at_ms: 6 * MINUTE_MS,
        ..the_break(Some(BreakCause::IdleGap))?
    };
    assert_eq!(breaks_alone(&calls), [found]);
    Ok(())
}

/// Text Cortex gave between the responses explains a break nothing else does.
#[test]
fn a_text_given_between_the_calls_is_the_cause_when_nothing_else_is() -> TestResult {
    let none = BTreeMap::new();
    let context = BreakContext {
        compactions_at_ms: &none,
        texts_given_at_ms: &[MINUTE_MS / 2],
    };
    let breaks = cache_breaks(&breaking_pair()?, &context);
    assert_eq!(breaks, [the_break(Some(BreakCause::HookText))?]);
    Ok(())
}

/// A break with the same model, no compaction, a minute's gap and no text has no cause.
#[test]
fn a_break_nothing_explains_has_no_cause() -> TestResult {
    assert_eq!(breaks_alone(&breaking_pair()?), [the_break(None)?]);
    Ok(())
}

/// Responses 1 to 3 are a minute apart, 3 to 4 six minutes. Response 2 loses the cache of 1
/// (reads 1,000 down to 100, writes 900), response 3 writes nothing so loses nothing, and
/// response 4 reads 10 of 3's 100 and writes 900 after the gap.
#[test]
fn every_break_of_a_session_is_found_in_turn_order() -> TestResult {
    let calls = [
        call(4, 8 * MINUTE_MS, "alpha", [10, 900, 10])?,
        call(1, 0, "alpha", [10, 0, 1_000])?,
        call(3, 2 * MINUTE_MS, "alpha", [10, 0, 100])?,
        call(2, MINUTE_MS, "alpha", [10, 900, 100])?,
    ];
    let turns: Vec<(u32, Option<BreakCause>)> = breaks_alone(&calls)
        .iter()
        .map(|found| (found.turn, found.cause))
        .collect();
    assert_eq!(turns, [(2, None), (4, Some(BreakCause::IdleGap))]);
    Ok(())
}

/// Writing exactly half of the cached input is not a break: 400 written and 400 read make 800
/// cached, and 400 is not over half of 800; 401 written and 400 read make 801 cached, and 401 is
/// over half of it. The read fell below half of the previous 1,000 in both.
#[test]
fn writing_exactly_half_the_cached_input_is_not_a_break() -> TestResult {
    let previous = call(1, 0, "alpha", [10, 0, 1_000])?;
    let half = call(2, MINUTE_MS, "alpha", [10, 400, 400])?;
    let over = call(2, MINUTE_MS, "alpha", [10, 401, 400])?;
    assert_eq!(breaks_alone(&[previous.clone(), half]), []);
    assert_eq!(breaks_alone(&[previous, over]).len(), 1);
    Ok(())
}

/// Reading exactly half of the previous response's cache reads is not a break: 500 of 1,000. 499
/// is. Both write 900, which is over half of their cached input (900 of 1,400 and of 1,399).
#[test]
fn reading_exactly_half_of_the_previous_reads_is_not_a_break() -> TestResult {
    let previous = call(1, 0, "alpha", [10, 0, 1_000])?;
    let half = call(2, MINUTE_MS, "alpha", [10, 900, 500])?;
    let under = call(2, MINUTE_MS, "alpha", [10, 900, 499])?;
    assert_eq!(breaks_alone(&[previous.clone(), half]), []);
    assert_eq!(breaks_alone(&[previous, under]).len(), 1);
    Ok(())
}

/// Nothing is lost by a response that follows one that read nothing, however much it writes, and
/// the first response is never a break; no responses have none.
#[test]
fn there_is_no_break_without_a_cache_to_lose() -> TestResult {
    let calls = [
        call(1, 0, "alpha", [10, 900, 0])?,
        call(2, MINUTE_MS, "alpha", [10, 900, 0])?,
    ];
    assert_eq!(breaks_alone(&calls), []);
    assert_eq!(breaks_alone(&calls[..1]), []);
    assert_eq!(breaks_alone(&[]), []);
    Ok(())
}

/// A gap of exactly five minutes is not over five.
#[test]
fn a_gap_of_exactly_five_minutes_is_not_idle() -> TestResult {
    assert_eq!(FIVE_MINUTE_LIFETIME_MS, 5 * MINUTE_MS);
    let mut calls = breaking_pair()?;
    calls[1].at_ms = FIVE_MINUTE_LIFETIME_MS;
    let found = CacheBreak {
        at_ms: FIVE_MINUTE_LIFETIME_MS,
        ..the_break(None)?
    };
    assert_eq!(breaks_alone(&calls), [found]);
    Ok(())
}

/// A conversation whose every cache write was for an hour keeps its cache an hour: of three
/// responses, the first writing 1,000 for an hour, the second reading them a minute later, the
/// third losing them, six minutes after the second has no known cause, and an hour and a minute
/// after it was idle too long. One write of the first for five minutes, or one whose lifetime the
/// transcript does not give, puts the lifetime back to five minutes, and six minutes is idle.
#[test]
fn a_cache_written_for_an_hour_lives_an_hour() -> TestResult {
    assert_eq!(ONE_HOUR_LIFETIME_MS, 60 * MINUTE_MS);
    let hour = |turn: u32, at_ms: i64, tokens: [u64; 3]| -> TestResult<UsageRow> {
        let row = call(turn, at_ms, "alpha", tokens)?;
        Ok(UsageRow {
            cache_write_1h: Some(row.cache_write),
            ..row
        })
    };
    let cause_after = |gap_ms: i64, first: &UsageRow| -> TestResult<Option<BreakCause>> {
        let calls = [
            first.clone(),
            hour(2, MINUTE_MS, [10, 0, 1_000])?,
            hour(3, MINUTE_MS + gap_ms, [10, 900, 100])?,
        ];
        let breaks = breaks_alone(&calls);
        assert_eq!(breaks.len(), 1);
        Ok(breaks.first().and_then(|found| found.cause))
    };
    let for_an_hour = hour(1, 0, [10, 1_000, 0])?;
    assert_eq!(cause_after(6 * MINUTE_MS, &for_an_hour)?, None);
    assert_eq!(
        cause_after(61 * MINUTE_MS, &for_an_hour)?,
        Some(BreakCause::IdleGap)
    );
    let for_five_minutes = call(1, 0, "alpha", [10, 1_000, 0])?;
    assert_eq!(
        cause_after(6 * MINUTE_MS, &for_five_minutes)?,
        Some(BreakCause::IdleGap)
    );
    let untold = UsageRow {
        cache_write_1h: None,
        ..for_an_hour
    };
    assert_eq!(
        cause_after(6 * MINUTE_MS, &untold)?,
        Some(BreakCause::IdleGap)
    );
    Ok(())
}

/// A compaction or a text before the previous response or after the current one explains nothing;
/// one on either edge of the span does.
#[test]
fn events_outside_the_span_explain_nothing() -> TestResult {
    let calls = breaking_pair()?;
    let before = compacted(&[-1])?;
    let outside = BreakContext {
        compactions_at_ms: &before,
        texts_given_at_ms: &[MINUTE_MS + 1],
    };
    assert_eq!(cache_breaks(&calls, &outside), [the_break(None)?]);
    let edge = compacted(&[MINUTE_MS])?;
    let on_the_edge = BreakContext {
        compactions_at_ms: &edge,
        texts_given_at_ms: &[],
    };
    let breaks = cache_breaks(&calls, &on_the_edge);
    assert_eq!(breaks, [the_break(Some(BreakCause::Compaction))?]);
    Ok(())
}

/// A response that names no model is no model switch.
#[test]
fn a_call_without_a_model_is_no_switch() -> TestResult {
    let mut calls = breaking_pair()?;
    calls[1].model = None;
    assert_eq!(breaks_alone(&calls), [the_break(None)?]);
    Ok(())
}

/// When several causes hold, the first of model switch, compaction, idle gap and text wins.
#[test]
fn the_first_cause_that_holds_is_the_one_given() -> TestResult {
    let mut calls = breaking_pair()?;
    calls[1].at_ms = 6 * MINUTE_MS;
    let compactions = compacted(&[MINUTE_MS])?;
    let none = BTreeMap::new();
    let cause_of = |calls: &[UsageRow], compactions| {
        let context = BreakContext {
            compactions_at_ms: compactions,
            texts_given_at_ms: &[MINUTE_MS],
        };
        cache_breaks(calls, &context)
            .first()
            .and_then(|found| found.cause)
    };
    // A text, a compaction and an idle gap: the compaction.
    assert_eq!(cause_of(&calls, &compactions), Some(BreakCause::Compaction));
    // A model switch on top of the same: the switch.
    let mut switched = calls.clone();
    switched[1].model = Some(Identifier::new("beta")?);
    assert_eq!(
        cause_of(&switched, &compactions),
        Some(BreakCause::ModelSwitch)
    );
    // A text and an idle gap: the gap.
    assert_eq!(cause_of(&calls, &none), Some(BreakCause::IdleGap));
    Ok(())
}

/// Each conversation keeps a cache of its own. Session `s1` reads 1,000 then 1,000; its subagent
/// `s1/agent-1`, between them, reads 100 and writes 900. Taken together the subagent's response
/// would look like a break of `s1`'s, and `s1`'s second like one of the subagent's; apart, neither
/// breaks. A subagent that loses its own cache (1,000 read, then 100 read and 900 written) breaks
/// alone, and a compaction of `s1` at the same time does not explain it.
#[test]
fn each_conversation_is_looked_at_apart() -> TestResult {
    let subagent = Identifier::new("s1/agent-1")?;
    let of_subagent = |turn: u32, at_ms: i64, tokens: [u64; 3]| -> TestResult<UsageRow> {
        Ok(UsageRow {
            session_id: subagent.clone(),
            ..call(turn, at_ms, "alpha", tokens)?
        })
    };
    let calls = [
        call(1, 0, "alpha", [10, 0, 1_000])?,
        of_subagent(1, MINUTE_MS, [10, 900, 100])?,
        call(2, 2 * MINUTE_MS, "alpha", [10, 0, 1_000])?,
    ];
    assert_eq!(breaks_alone(&calls), []);
    let losing = [
        call(1, 0, "alpha", [10, 0, 1_000])?,
        of_subagent(1, MINUTE_MS, [10, 0, 1_000])?,
        of_subagent(2, 2 * MINUTE_MS, [10, 900, 100])?,
    ];
    let compactions = compacted(&[90_000])?;
    let context = BreakContext {
        compactions_at_ms: &compactions,
        texts_given_at_ms: &[],
    };
    let found = CacheBreak {
        session_id: subagent.clone(),
        turn: 2,
        at_ms: 2 * MINUTE_MS,
        cause: None,
    };
    assert_eq!(cache_breaks(&losing, &context), [found]);
    Ok(())
}
