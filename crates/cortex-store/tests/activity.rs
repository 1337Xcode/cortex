//! Sessions, tasks and events written through the writer and read back, and the rows the store
//! refuses to read back.

mod common;

use std::path::PathBuf;

use common::{Scratch, TestResult, index, name, path};
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
        holdout: None,
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

/// A task not ended counts as quiet once the latest of its start, its own events and its
/// session's events is at or before the time asked. Worked by hand: `open` (s1) started at 100,
/// its own last event at 400, its session's last event (outside the task) at 700, so its last
/// sign is 700; `fresh` (s2) started at 800 with no event, last sign 800; `ended` (s1) ended and
/// never counts. Asked at 699: none; at 700: `open` at 700; at 800: `open`, then `fresh`.
#[test]
fn tasks_of_quiet_sessions_are_found_by_their_last_sign_of_work() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let open = task(b"open", "s1", 100)?;
    let ended = task(b"ended", "s1", 50)?;
    let fresh = task(b"fresh", "s2", 800)?;
    committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 10)?)?;
        transaction.record_session(&session("s2", 20)?)?;
        transaction.start_task(&ended)?;
        transaction.end_task(ended.task_id, 90, TaskOutcome::Unknown)?;
        transaction.start_task(&open)?;
        transaction.start_task(&fresh)?;
        transaction.append_event(&event(Some(open.task_id), 400, EventKind::Prompt)?)?;
        transaction.append_event(&event(None, 700, EventKind::Compact)?)?;
        Ok(())
    })?;
    let quiet = |at_ms: i64| -> TestResult<Vec<(Ulid, String, i64)>> {
        Ok(writer
            .queries()
            .quiet_open_tasks(at_ms)?
            .into_iter()
            .map(|task| {
                (
                    task.task_id,
                    task.session_id.as_str().to_owned(),
                    task.last_ms,
                )
            })
            .collect())
    };
    assert_eq!(quiet(699)?, []);
    assert_eq!(quiet(700)?, [(open.task_id, "s1".to_owned(), 700)]);
    assert_eq!(
        quiet(800)?,
        [
            (open.task_id, "s1".to_owned(), 700),
            (fresh.task_id, "s2".to_owned(), 800)
        ]
    );
    Ok(())
}

/// A session's edits are those of its tasks, its subagent's included and another session's not,
/// newest first by time; its reads and compactions are its own, newest first by time even when
/// stored out of order. Worked by hand from the rows written: edits 8 (330), 4 (180), 1 (150);
/// reads and compactions 7 (320), 9 (310), 6 (190), 2 (160).
#[test]
fn a_sessions_edits_and_reads_read_back_newest_first() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let first = task(b"first", "s1", 100)?;
    let other = task(b"other", "s2", 110)?;
    let third = task(b"third", "s1", 300)?;
    let subagent = SessionRecord {
        parent_session_id: Some(name("s1")?),
        ..session("s1/agent", 70)?
    };
    let of = |session_id: &str, record: EventRecord| -> TestResult<EventRecord> {
        Ok(EventRecord {
            session_id: Some(name(session_id)?),
            ..record
        })
    };
    let rows = [
        event(Some(first.task_id), 150, EventKind::Edit)?,
        event(Some(first.task_id), 160, EventKind::Read)?,
        of("s2", event(Some(other.task_id), 170, EventKind::Edit)?)?,
        of(
            "s1/agent",
            event(Some(first.task_id), 180, EventKind::Edit)?,
        )?,
        of(
            "s1/agent",
            event(Some(first.task_id), 185, EventKind::Read)?,
        )?,
        event(Some(first.task_id), 190, EventKind::Compact)?,
        event(Some(third.task_id), 320, EventKind::Read)?,
        event(Some(third.task_id), 330, EventKind::Edit)?,
        event(Some(third.task_id), 310, EventKind::Read)?,
    ];
    committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 50)?)?;
        transaction.record_session(&session("s2", 60)?)?;
        transaction.record_session(&subagent)?;
        transaction.start_task(&first)?;
        transaction.start_task(&other)?;
        transaction.start_task(&third)?;
        for row in &rows {
            transaction.append_event(row)?;
        }
        Ok(())
    })?;
    let queries = writer.queries();
    let numbers = |rows: Vec<cortex_store::EventRow>| -> Vec<u64> {
        rows.iter().map(|row| row.seq).collect()
    };
    let s1 = name("s1")?;
    assert_eq!(numbers(queries.edits_of_session(&s1, 10)?), [8, 4, 1]);
    assert_eq!(numbers(queries.edits_of_session(&s1, 2)?), [8, 4]);
    assert_eq!(numbers(queries.reads_of_session(&s1, 10)?), [7, 9, 6, 2]);
    assert_eq!(numbers(queries.reads_of_session(&s1, 2)?), [7, 9]);
    assert_eq!(
        numbers(queries.edits_of_session(&name("s3")?, 10)?),
        Vec::<u64>::new()
    );
    Ok(())
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
    let events = queries.events_of_task(first.task_id, 10)?;
    let kinds: Vec<(u64, EventKind)> = events
        .iter()
        .map(|row| (row.seq, row.record.kind))
        .collect();
    assert_eq!(kinds, [(1, EventKind::Prompt), (2, EventKind::Edit)]);
    assert_eq!(events.last().map(|row| &row.record), Some(&edit));
    let first_only: Vec<u64> = queries
        .events_of_task(first.task_id, 1)?
        .iter()
        .map(|row| row.seq)
        .collect();
    assert_eq!(first_only, [1]);
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
    assert_eq!(first_row.record.holdout, None);
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

/// What became of a task is set once it has ended, with the words that say why, and a later
/// judgement replaces it: a task ended as unknown is judged a success, then put back by the next
/// task. A task still open, and one that was never started, take no outcome.
#[test]
fn an_ended_task_is_given_its_outcome_and_why() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (ended, open) = (task(b"ended", "s1", 100)?, task(b"open", "s1", 200)?);
    let never = Ulid::from_seed(300, b"never");
    let set = committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 50)?)?;
        transaction.start_task(&ended)?;
        transaction.end_task(ended.task_id, 150, TaskOutcome::Unknown)?;
        transaction.start_task(&open)?;
        Ok([
            transaction.set_task_outcome(
                ended.task_id,
                TaskOutcome::Success,
                Some("tests by cargo exited 0 after the last edit"),
            )?,
            transaction.set_task_outcome(open.task_id, TaskOutcome::Success, None)?,
            transaction.set_task_outcome(never, TaskOutcome::Success, None)?,
        ])
    })?;
    assert_eq!(set, [true, false, false]);
    let judged = |writer: &Writer, task_id| -> TestResult<_> {
        let row = writer.queries().task(task_id)?.ok_or("no task")?;
        Ok((row.ended_at_ms, row.outcome, row.outcome_evidence))
    };
    assert_eq!(
        judged(&writer, ended.task_id)?,
        (
            Some(150),
            Some(TaskOutcome::Success),
            Some("tests by cargo exited 0 after the last edit".to_owned())
        )
    );
    assert_eq!(judged(&writer, open.task_id)?, (None, None, None));
    committed(&mut writer, |transaction| {
        transaction.set_task_outcome(
            ended.task_id,
            TaskOutcome::Reverted,
            Some(
                "every edited file is back as it was before the task, by the end of the next task",
            ),
        )?;
        Ok(())
    })?;
    assert_eq!(
        judged(&writer, ended.task_id)?,
        (
            Some(150),
            Some(TaskOutcome::Reverted),
            Some(
                "every edited file is back as it was before the task, by the end of the next task"
                    .to_owned()
            )
        )
    );
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
        let ended = transaction.end_session(&name("s1")?, (1_000, SessionStatus::Ended))?;
        let unknown = transaction.end_session(&name("nobody")?, (1_000, SessionStatus::Ended))?;
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

/// A task touched a file when one of its own events read or edited it: task `first` read a.py,
/// edited b.py and searched with c.py's node on its event; task `other` read c.py. So `first`
/// touched a.py and b.py and not c.py (a search is no read, and the other task's read is not its
/// own); `other` touched c.py alone. Looking at `first`'s latest event only (the search), it
/// touched none; at its latest two (the edit and the search), b.py alone.
#[test]
fn a_task_touched_the_files_it_read_or_edited() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let first = task(b"first", "s1", 100)?;
    let other = task(b"other", "s1", 200)?;
    let nodes = committed(&mut writer, |transaction| {
        let mut nodes = Vec::new();
        for file in ["a.py", "b.py", "c.py"] {
            nodes.push(
                transaction
                    .replace_file(&index(file, Vec::new(), Vec::new())?)?
                    .file_node,
            );
        }
        transaction.record_session(&session("s1", 50)?)?;
        transaction.start_task(&first)?;
        transaction.start_task(&other)?;
        Ok(nodes)
    })?;
    let [a, b, c] = nodes[..] else {
        return Err("three nodes".into());
    };
    let on = |task_id, kind, node| -> TestResult<EventRecord> {
        Ok(EventRecord {
            node_id: Some(node),
            ..event(Some(task_id), 300, kind)?
        })
    };
    let rows = [
        on(first.task_id, EventKind::Read, a)?,
        on(first.task_id, EventKind::Edit, b)?,
        on(first.task_id, EventKind::Search, c)?,
        on(other.task_id, EventKind::Read, c)?,
    ];
    committed(&mut writer, |transaction| {
        for row in &rows {
            transaction.append_event(row)?;
        }
        Ok(())
    })?;
    let queries = writer.queries();
    let touched = |task_id, within| -> TestResult<Vec<bool>> {
        [a, b, c]
            .into_iter()
            .map(|node| Ok(queries.has_touched(task_id, node, within)?))
            .collect()
    };
    assert_eq!(touched(first.task_id, 10)?, [true, true, false]);
    assert_eq!(touched(other.task_id, 10)?, [false, false, true]);
    assert_eq!(touched(first.task_id, 1)?, [false, false, false]);
    assert_eq!(touched(first.task_id, 2)?, [false, true, false]);
    Ok(())
}

/// Whether a task is held out is kept as its start gave it, held out, helped or not decided, and
/// read back as given; the repository's arms count the decided tasks only: one held out, one
/// helped.
#[test]
fn whether_a_task_is_held_out_reads_back() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let held = TaskRecord {
        holdout: Some(true),
        ..task(b"held", "s1", 10)?
    };
    let helped = TaskRecord {
        holdout: Some(false),
        ..task(b"helped", "s1", 20)?
    };
    let undecided = task(b"undecided", "s1", 30)?;
    committed(&mut writer, |transaction| {
        transaction.record_session(&session("s1", 5)?)?;
        for record in [&held, &helped, &undecided] {
            transaction.start_task(record)?;
        }
        Ok(())
    })?;
    let queries = writer.queries();
    for record in [held, helped, undecided] {
        let row = queries.task(record.task_id)?.ok_or("the task")?;
        assert_eq!(row.record, record);
    }
    assert_eq!(queries.holdout_arms()?, (1, 1));
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
        queries.events_of_task(first.task_id, 10).err().map(corrupt),
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
