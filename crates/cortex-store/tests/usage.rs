//! The tokens each response of a session took: written, read back by turn for one session,
//! replaced in place when a transcript is read again, refused when a count does not fit or the
//! session is unknown, and reported when a stored row is damaged. Expected rows are written by
//! hand.

mod common;

use common::{Scratch, TestResult, name};
use cortex_core::{Agent, ShortHash};
use cortex_store::{SessionRecord, StoreError, UsageRow, Writer};

/// A session of Claude Code with id `id`, started at 100 ms.
fn session(id: &str) -> TestResult<SessionRecord> {
    Ok(SessionRecord {
        session_id: name(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 100,
    })
}

/// The usage of turn `turn` of `session_id`: its counts are input, cache writes, cache reads,
/// output and reasoning; its cache writes all for five minutes, served at the standard tier and
/// speed, run anywhere.
fn usage(
    session_id: &str,
    turn: u32,
    counts: (u64, u64, u64, u64),
    reasoning: Option<u64>,
) -> TestResult<UsageRow> {
    let (input, cache_write, cache_read, output) = counts;
    Ok(UsageRow {
        session_id: name(session_id)?,
        turn,
        at_ms: 1_000 + i64::from(turn),
        model: Some(name("claude-sonnet-5-5")?),
        input,
        cache_write,
        cache_write_1h: Some(0),
        cache_read,
        output,
        reasoning,
        service_tier: Some(name("standard")?),
        speed: Some(name("standard")?),
        inference_geo: Some(name("global")?),
        response: None,
    })
}

/// A writer whose database knows the sessions `s1` and `s2`.
fn with_sessions(scratch: &Scratch) -> TestResult<Writer> {
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.record_session(&session("s1")?)?;
    transaction.record_session(&session("s2")?)?;
    transaction.commit()?;
    Ok(writer)
}

/// Rows read back for their session only, by turn, whatever order they were written in; writing
/// a turn again replaces its row (a transcript read again after a copy of a response grew), and
/// leaves the other turns and the other session alone. A reasoning count the transcript did not
/// give stays absent rather than becoming zero, and so do a model, a split of the cache writes and
/// how the response was served; a response served fast in the United States, with an hour's cache
/// writes, keeps that.
#[test]
fn usage_is_written_read_back_by_turn_and_replaced() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = with_sessions(&scratch)?;
    let second = usage("s1", 2, (2, 3_819, 29_797, 95), Some(0))?;
    let first = UsageRow {
        model: None,
        cache_write_1h: None,
        service_tier: None,
        speed: None,
        inference_geo: None,
        ..usage("s1", 1, (3_702, 29_797, 0, 100), None)?
    };
    let other = UsageRow {
        cache_write_1h: Some(0),
        speed: Some(name("fast")?),
        inference_geo: Some(name("us")?),
        ..usage("s2", 1, (7, 0, 0, 1), Some(1))?
    };
    let mut transaction = writer.transaction()?;
    transaction.replace_usage(&[second.clone(), first.clone(), other.clone()])?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().usage_of_session(&name("s1")?)?,
        [first.clone(), second]
    );
    let grown = usage("s1", 2, (2, 3_819, 29_797, 197), Some(66))?;
    let mut transaction = writer.transaction()?;
    transaction.replace_usage(std::slice::from_ref(&grown))?;
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(queries.usage_of_session(&name("s1")?)?, [first, grown]);
    assert_eq!(queries.usage_of_session(&name("s2")?)?, [other]);
    assert_eq!(queries.usage_of_session(&name("s3")?)?, []);
    Ok(())
}

/// A response two sessions report, as a forked session's transcript copies the responses of the
/// one it was forked from, is read back for the session that started first only. `s1` and `s2`
/// started at 100 ms and `s3` at 200 ms. `m1` is reported by all three: `s1` keeps it, as the
/// smaller id of the two that started first. `m2` is reported by `s3` alone, which keeps it. A row
/// whose response is not known is never taken for a copy.
#[test]
fn a_response_two_sessions_report_counts_once() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = with_sessions(&scratch)?;
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        started_at_ms: 200,
        ..session("s3")?
    })?;
    let (m1, m2) = (Some(ShortHash::of(b"m1")), Some(ShortHash::of(b"m2")));
    let of = |session_id: &str, turn: u32, response: Option<ShortHash>| {
        usage(session_id, turn, (u64::from(turn), 0, 0, 1), None)
            .map(|row| UsageRow { response, ..row })
    };
    let rows = [
        of("s1", 1, m1)?,
        of("s1", 2, None)?,
        of("s2", 1, m1)?,
        of("s3", 1, m1)?,
        of("s3", 2, m2)?,
        of("s3", 3, None)?,
    ];
    transaction.replace_usage(&rows)?;
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(
        queries.usage_of_session(&name("s1")?)?,
        [of("s1", 1, m1)?, of("s1", 2, None)?]
    );
    assert_eq!(queries.usage_of_session(&name("s2")?)?, []);
    assert_eq!(
        queries.usage_of_session(&name("s3")?)?,
        [of("s3", 2, m2)?, of("s3", 3, None)?]
    );
    Ok(())
}

/// Forgetting a session's usage removes every row of that session and no other's, so that a
/// transcript read again from its start leaves no row past its new end.
#[test]
fn a_session_s_usage_is_forgotten_whole() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = with_sessions(&scratch)?;
    let other = usage("s2", 1, (7, 0, 0, 1), None)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_usage(&[
        usage("s1", 1, (3_702, 29_797, 0, 100), None)?,
        usage("s1", 2, (2, 3_819, 29_797, 95), None)?,
        other.clone(),
    ])?;
    transaction.forget_usage(&name("s1")?)?;
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(queries.usage_of_session(&name("s1")?)?, []);
    assert_eq!(queries.usage_of_session(&name("s2")?)?, [other]);
    Ok(())
}

/// A count past what SQLite stores is refused naming its column, and nothing of the batch is
/// written; a row for a session the database does not know is refused too.
#[test]
fn usage_that_cannot_be_stored_is_refused_whole() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = with_sessions(&scratch)?;
    let fits = usage("s1", 1, (1, 0, 0, 1), None)?;
    let too_large = usage("s1", 2, (1, 0, u64::MAX, 1), None)?;
    let mut transaction = writer.transaction()?;
    let refused = transaction.replace_usage(&[fits.clone(), too_large]);
    assert!(
        matches!(
            refused,
            Err(StoreError::TooLarge {
                column: "usage.cache_read",
                value: u64::MAX
            })
        ),
        "{refused:?}"
    );
    let unknown = transaction.replace_usage(&[usage("s9", 1, (1, 0, 0, 1), None)?]);
    assert!(
        matches!(unknown, Err(StoreError::Sqlite { .. })),
        "{unknown:?}"
    );
    transaction.commit()?;
    assert_eq!(writer.queries().usage_of_session(&name("s1")?)?, []);
    let mut transaction = writer.transaction()?;
    transaction.replace_usage(std::slice::from_ref(&fits))?;
    transaction.commit()?;
    assert_eq!(writer.queries().usage_of_session(&name("s1")?)?, [fits]);
    Ok(())
}

/// Rows damaged outside Cortex are reported as damaged, naming the column: a count below zero, a
/// turn past what a turn number holds, an hour's cache writes below zero, a model, tier, speed or
/// region that is no name, and a response hash that is not 16 bytes.
#[test]
fn damaged_usage_rows_are_reported() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = with_sessions(&scratch)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_usage(&[usage("s1", 1, (1, 0, 0, 1), None)?])?;
    transaction.commit()?;
    drop(writer);
    let damages = [
        ("UPDATE usage SET output = -1", "usage.output"),
        (
            "UPDATE usage SET output = 1, turn = 4294967296",
            "usage.turn",
        ),
        (
            "UPDATE usage SET turn = 1, model = 'a' || char(10) || 'b'",
            "usage.model",
        ),
        (
            "UPDATE usage SET model = NULL, speed = 'a' || char(10) || 'b'",
            "usage.speed",
        ),
        (
            "UPDATE usage SET speed = NULL, service_tier = 'a' || char(10) || 'b'",
            "usage.service_tier",
        ),
        (
            "UPDATE usage SET service_tier = NULL, inference_geo = 'a' || char(10) || 'b'",
            "usage.inference_geo",
        ),
        (
            "UPDATE usage SET inference_geo = NULL, cache_write_1h = -1",
            "usage.cache_write_1h",
        ),
        (
            "UPDATE usage SET cache_write_1h = NULL, response = x'0102'",
            "usage.response",
        ),
    ];
    for (damage, column) in damages {
        rusqlite::Connection::open(scratch.folder.join("cortex.db"))?.execute(damage, [])?;
        let read = scratch.reader()?.queries().usage_of_session(&name("s1")?);
        assert!(
            matches!(read, Err(StoreError::Corrupt { column: found, .. }) if found == column),
            "{damage}: {read:?}"
        );
    }
    Ok(())
}
