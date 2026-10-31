//! Sessions, tasks and events written through the writer and read back, and the rows the store
//! refuses to read back.

mod common;

use std::path::PathBuf;

use common::{Scratch, TestResult, name, path};
use cortex_core::{
    Agent, ContentHash, EventKind, EventMeta, EventSource, LineRange, PromptExcerpt, SessionStatus,
    TaskOutcome, ToolKind, Ulid,
};
use cortex_store::{EventRecord, SessionRecord, StoreError, TaskRecord, Writer};

/// A Claude Code session started at `started_at_ms`, with nothing optional known.
fn session(id: &str, started_at_ms: i64) -> TestResult<SessionRecord> {
    Ok(SessionRecord {
        session_id: name(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms,
    })
}

/// A task of `session_id` started at `started_at_ms`, its id made from `seed`, with a prompt hash.
fn task(seed: &[u8], session_id: &str, started_at_ms: i64) -> TestResult<TaskRecord> {
    Ok(TaskRecord {
        task_id: Ulid::from_seed(u64::try_from(started_at_ms)?, seed),
        session_id: name(session_id)?,
        started_at_ms,
        prompt_hash: Some(ContentHash::of(b"fix the divide")),
        prompt_excerpt: None,
    })
}

/// An event of `kind` in `task_id`'s session `s1` at `ts_ms`, with no path, lines or facts.
fn event(task_id: Option<Ulid>, ts_ms: i64, kind: EventKind) -> TestResult<EventRecord> {
    Ok(EventRecord {
        task_id,
        session_id: Some(name("s1")?),
        ts_ms,
        kind,
        node_id: None,
        path: None,
        range: None,
        meta: EventMeta::default(),
    })
}

/// Writes `write` in one committed transaction.
fn committed<T>(
    writer: &mut Writer,
    write: impl FnOnce(&mut cortex_store::WriteTx<'_>) -> TestResult<T>,
) -> TestResult<T> {
    let mut transaction = writer.transaction()?;
    let value = write(&mut transaction)?;
    transaction.commit()?;
    Ok(value)
}

/// Tasks are numbered across the repository, not per session: two sessions' tasks get 1, 2 and
/// 3 in the order they start. A session's open task is its latest unended one; ending it closes
/// it once. Events read back per task and per session, in order, with their paths, lines and
/// facts.
#[test]
fn tasks_and_events_read_back_in_order() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let first = task(b"first", "s1", 100)?;
    let second = task(b"second", "s2", 200)?;
    let third = task(b"third", "s1", 300)?;
    let edit = EventRecord {
        path: Some(path("calc/ops.py")?),
        range: Some(LineRange::new(10, 13)?),
        meta: EventMeta {
            tool: Some(ToolKind::Edit),
            is_success: Some(true),
            ..EventMeta::default()
        },
        ..event(Some(first.task_id), 150, EventKind::Edit)?
    };
    let numbers = committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 50)?)?;
        transaction.record_session(&session("s2", 60)?)?;
        let numbers = (
            transaction.start_task(&first)?,
            transaction.start_task(&second)?,
        );
        transaction.append_event(&event(Some(first.task_id), 120, EventKind::Prompt)?)?;
        transaction.append_event(&edit)?;
        Ok(numbers)
    })?;
    assert_eq!(numbers, (1, 2));
    let queries = writer.queries();
    assert_eq!(
        queries
            .open_task(&name("s1")?)?
            .map(|row| row.record.task_id),
        Some(first.task_id)
    );
    let events = queries.events_of_task(first.task_id)?;
    let kinds: Vec<(u64, EventKind)> = events
        .iter()
        .map(|row| (row.seq, row.record.kind))
        .collect();
    assert_eq!(kinds, [(1, EventKind::Prompt), (2, EventKind::Edit)]);
    assert_eq!(events.last().map(|row| &row.record), Some(&edit));
    let (ended, ended_again, third_number) = committed(&mut writer, |transaction| {
        let ended = transaction.end_task(first.task_id, 250, TaskOutcome::Unknown)?;
        let ended_again = transaction.end_task(first.task_id, 260, TaskOutcome::Success)?;
        Ok((ended, ended_again, transaction.start_task(&third)?))
    })?;
    assert_eq!((ended, ended_again, third_number), (true, false, 3));
    let queries = writer.queries();
    let first_row = queries.task(first.task_id)?.ok_or("the first task")?;
    assert_eq!(
        (first_row.seq, first_row.ended_at_ms, first_row.outcome),
        (1, Some(250), Some(TaskOutcome::Unknown))
    );
    assert_eq!(first_row.record, first);
    assert_eq!(first_row.holdout, None);
    assert_eq!(
        queries
            .open_task(&name("s1")?)?
            .map(|row| row.record.task_id),
        Some(third.task_id)
    );
    let of_session: Vec<u64> = queries
        .events_of_session(&name("s1")?)?
        .iter()
        .map(|row| row.seq)
        .collect();
    assert_eq!(of_session, [1, 2]);
    Ok(())
}

/// A session seen again keeps its first start time, takes the facts newly given and keeps those
/// not given again; ended, it reads as ended, and seen once more it is live again.
#[test]
fn a_session_seen_again_keeps_its_start_and_gains_facts() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let full = SessionRecord {
        agent_version: Some(name("2.1.312")?),
        model: Some(name("sonnet")?),
        cwd: Some(PathBuf::from("/repo")),
        transcript_path: Some(PathBuf::from("/home/dev/s1.jsonl")),
        ..session("s1", 100)?
    };
    let later = SessionRecord {
        model: Some(name("opus")?),
        ..session("s1", 900)?
    };
    committed(&mut writer, |transaction| {
        transaction.record_session(&full)?;
        Ok(transaction.record_session(&later)?)
    })?;
    let row = writer
        .queries()
        .session(&name("s1")?)?
        .ok_or("the session")?;
    let expected = SessionRecord {
        model: Some(name("opus")?),
        ..full.clone()
    };
    assert_eq!(row.record, expected);
    assert_eq!((row.ended_at_ms, row.status), (None, SessionStatus::Live));
    let ended = committed(&mut writer, |transaction| {
        let ended = transaction.end_session(&name("s1")?, 1_000)?;
        let unknown = transaction.end_session(&name("nobody")?, 1_000)?;
        Ok((ended, unknown))
    })?;
    assert_eq!(ended, (true, false));
    let row = writer
        .queries()
        .session(&name("s1")?)?
        .ok_or("the session")?;
    assert_eq!(
        (row.ended_at_ms, row.status),
        (Some(1_000), SessionStatus::Ended)
    );
    committed(&mut writer, |transaction| {
        Ok(transaction.record_session(&later)?)
    })?;
    let row = writer
        .queries()
        .session(&name("s1")?)?
        .ok_or("the session")?;
    assert_eq!((row.ended_at_ms, row.status), (None, SessionStatus::Live));
    assert_eq!(row.record.started_at_ms, 100);
    Ok(())
}

/// A task needs its session to be stored, and a task id is used once.
#[test]
fn a_task_needs_its_session_and_a_new_id() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    assert!(
        transaction
            .start_task(&task(b"orphan", "nobody", 10)?)
            .is_err()
    );
    transaction.record_session(&session("s1", 5)?)?;
    let first = task(b"first", "s1", 10)?;
    assert_eq!(transaction.start_task(&first)?, 1);
    assert!(transaction.start_task(&first).is_err());
    transaction.commit()?;
    Ok(())
}

/// A prompt excerpt and a watcher's event facts are kept as given and read back; an event with
/// no task and no session is stored too.
#[test]
fn excerpts_and_watcher_events_read_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let with_excerpt = TaskRecord {
        prompt_excerpt: Some(PromptExcerpt::of("rename parse to read")),
        ..task(b"excerpt", "s1", 10)?
    };
    let watcher_edit = EventRecord {
        task_id: None,
        session_id: None,
        path: Some(path("notes.md")?),
        meta: EventMeta {
            source: Some(EventSource::Watcher),
            ..EventMeta::default()
        },
        ..event(None, 20, EventKind::Edit)?
    };
    committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 5)?)?;
        transaction.start_task(&with_excerpt)?;
        Ok(transaction.append_event(&watcher_edit)?)
    })?;
    let queries = writer.queries();
    let row = queries.task(with_excerpt.task_id)?.ok_or("the task")?;
    assert_eq!(row.record, with_excerpt);
    let reader = scratch.reader()?;
    let queries = reader.queries();
    assert_eq!(queries.events_of_session(&name("s1")?)?, []);
    let stored: Vec<EventRecord> = queries
        .events_after(0, 10)?
        .into_iter()
        .map(|row| row.record)
        .collect();
    assert_eq!(stored, [watcher_edit]);
    assert_eq!(queries.events_after(1, 10)?, []);
    Ok(())
}

/// Rows damaged outside Cortex are reported as damaged, naming the column: an unknown event
/// kind, an event whose range has a start and no end, a task id that is not an id, and an
/// unknown session status.
#[test]
fn damaged_activity_rows_are_reported() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let first = task(b"first", "s1", 10)?;
    committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 5)?)?;
        transaction.start_task(&first)?;
        transaction.append_event(&event(Some(first.task_id), 11, EventKind::Read)?)?;
        Ok(transaction.append_event(&event(Some(first.task_id), 12, EventKind::Read)?)?)
    })?;
    drop(writer);
    let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
    connection.execute("UPDATE events SET kind = 'reed' WHERE seq = 1", [])?;
    connection.execute("UPDATE sessions SET status = 'gone'", [])?;
    drop(connection);
    let reader = scratch.reader()?;
    let queries = reader.queries();
    let corrupt = |error: StoreError| match error {
        StoreError::Corrupt { column, .. } => column,
        _ => "another error",
    };
    assert_eq!(
        queries.events_of_task(first.task_id).err().map(corrupt),
        Some("events.kind")
    );
    assert_eq!(
        queries.session(&name("s1")?).err().map(corrupt),
        Some("sessions.status")
    );
    drop(reader);
    let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
    connection.execute("UPDATE events SET kind = 'read', range_start = 3", [])?;
    connection.execute("UPDATE tasks SET task_id = 'not-an-id'", [])?;
    drop(connection);
    let reader = scratch.reader()?;
    let queries = reader.queries();
    assert_eq!(
        queries.events_of_session(&name("s1")?).err().map(corrupt),
        Some("events.range_end")
    );
    assert_eq!(
        queries.open_task(&name("s1")?).err().map(corrupt),
        Some("tasks.task_id")
    );
    Ok(())
}
