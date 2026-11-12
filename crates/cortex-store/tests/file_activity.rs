//! What agents did to each file, added up: reads, the last read or edit, and the tokens of the
//! responses given while an agent worked on the file. Expected numbers are counted by hand in the
//! comments.

mod common;

use common::{Scratch, TestResult, index, name, path};
use cortex_core::{Agent, EventKind, EventMeta, EventSource, ShortHash};
use cortex_store::{EventRecord, FileReads, NodeId, SessionRecord, UsageRow, Writer};

/// Stores `a.py` and `b.py` and the sessions `s1` and `s2`, both started at 100 ms; returns the
/// files' nodes.
fn store(writer: &mut Writer) -> TestResult<(NodeId, NodeId)> {
    let mut transaction = writer.transaction()?;
    let a = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    let b = transaction
        .replace_file(&index("b.py", Vec::new(), Vec::new())?)?
        .file_node;
    for id in ["s1", "s2"] {
        transaction.record_session(&SessionRecord {
            session_id: name(id)?,
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 100,
        })?;
    }
    transaction.commit()?;
    Ok((a, b))
}

/// An event of `kind` that session `session_id` reported at `ts_ms` about the file at `node`,
/// as the file watcher when `source` says so.
fn event(
    (session_id, ts_ms): (&str, i64),
    kind: EventKind,
    (node, file): (NodeId, &str),
    source: Option<EventSource>,
) -> TestResult<EventRecord> {
    Ok(EventRecord {
        task_id: None,
        session_id: Some(name(session_id)?),
        ts_ms,
        kind,
        node_id: Some(node),
        path: Some(path(file)?),
        range: None,
        meta: EventMeta {
            source,
            ..EventMeta::default()
        },
    })
}

/// A response of `session_id` at `at_ms` of `input` input and `output` output tokens, with no
/// cache use, that the provider's id `response` names when it is given.
fn usage(
    (session_id, turn, at_ms): (&str, u32, i64),
    (input, output): (u64, u64),
    response: Option<&str>,
) -> TestResult<UsageRow> {
    Ok(UsageRow {
        session_id: name(session_id)?,
        turn,
        at_ms,
        model: None,
        input,
        cache_write: 0,
        cache_write_1h: None,
        cache_read: 0,
        output,
        reasoning: None,
        service_tier: None,
        speed: None,
        inference_geo: None,
        response: response.map(|id| ShortHash::of(id.as_bytes())),
    })
}

/// Reads and tokens, counted by hand.
///
/// Events: `s1` reads a.py at 200 and 210, edits b.py at 300, and the watcher sees an edit of
/// a.py at 400; `s2` reads b.py at 150. So a.py was read 2 times and last touched at 210 (the
/// watcher's edit is the user's), and b.py once and last touched at 300.
///
/// Responses and the file each is counted for (the latest read or edit by the same session at or
/// before it, the watcher's not counting): `s1` turn 1 at 150 has none yet, 5 + 5 = 10 tokens,
/// for no file; turn 2 at 205 follows the read of a.py at 200: 1 + 2 = 3 tokens for a.py; turn 3
/// at 350 follows the edit of b.py at 300: 5 + 5 = 10 for b.py; turn 4 at 500 still follows that
/// edit, as the watcher's at 400 counts for nothing: 10 + 10 = 20 for b.py. `s2` turn 1 at 260
/// follows its read of b.py at 150: 3 + 4 = 7 for b.py; its turn 2 at 270 is a copy of `s1`'s
/// turn 2 (the same response id, and `s1` is the first to be started by id), which counts once,
/// for `s1`: its 1000 + 1000 tokens count for no file. So a.py took 3 and b.py 10 + 20 + 7 = 37;
/// since 300 ms, b.py took 10 + 20 = 30 and a.py none.
#[test]
fn reads_and_tokens_are_added_up_by_file() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (a, b) = store(&mut writer)?;
    let mut transaction = writer.transaction()?;
    for row in [
        event(("s1", 200), EventKind::Read, (a, "a.py"), None)?,
        event(("s1", 210), EventKind::Read, (a, "a.py"), None)?,
        event(("s1", 300), EventKind::Edit, (b, "b.py"), None)?,
        event(
            ("s1", 400),
            EventKind::Edit,
            (a, "a.py"),
            Some(EventSource::Watcher),
        )?,
        event(("s2", 150), EventKind::Read, (b, "b.py"), None)?,
    ] {
        transaction.append_event(&row)?;
    }
    transaction.replace_usage(&[
        usage(("s1", 1, 150), (5, 5), None)?,
        usage(("s1", 2, 205), (1, 2), Some("shared"))?,
        usage(("s1", 3, 350), (5, 5), None)?,
        usage(("s1", 4, 500), (10, 10), None)?,
        usage(("s2", 1, 260), (3, 4), None)?,
        usage(("s2", 2, 270), (1_000, 1_000), Some("shared"))?,
    ])?;
    transaction.commit()?;
    let reader = scratch.reader()?;
    assert_eq!(
        reader.queries().file_reads()?,
        [
            FileReads {
                node: a,
                reads: 2,
                last_ms: 210
            },
            FileReads {
                node: b,
                reads: 1,
                last_ms: 300
            },
        ]
    );
    assert_eq!(reader.queries().file_tokens(0)?, [(a, 3), (b, 37)]);
    assert_eq!(reader.queries().file_tokens(300)?, [(b, 30)]);
    Ok(())
}

/// With nothing stored, no file has reads or tokens.
#[test]
fn an_empty_store_has_no_activity() -> TestResult {
    let scratch = Scratch::new()?;
    let _writer = scratch.writer(1)?;
    let reader = scratch.reader()?;
    assert_eq!(reader.queries().file_reads()?, []);
    assert_eq!(reader.queries().file_tokens(0)?, []);
    Ok(())
}
