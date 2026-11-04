//! The lists receipts read: sessions by start, a session's subagents and tasks, the tool calls
//! each task made and each session made outside its tasks, and the arms of the held-out tasks.

mod common;

use common::{Scratch, TestResult, name, path};
use cortex_core::{Agent, ContentHash, EventKind, EventMeta, EventSource, LineRange, Ulid};
use cortex_store::{EventRecord, SessionRecord, TaskRecord, WriteTx};

/// A Claude Code session started at `started_at_ms`, started by `parent` when one is given.
fn session(id: &str, parent: Option<&str>, started_at_ms: i64) -> TestResult<SessionRecord> {
    Ok(SessionRecord {
        session_id: name(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: parent.map(name).transpose()?,
        cwd: None,
        transcript_path: None,
        started_at_ms,
    })
}

/// A task of `session_id` started at `started_at_ms`, its id made from `seed`.
fn task(seed: &[u8], session_id: &str, started_at_ms: i64) -> TestResult<TaskRecord> {
    Ok(TaskRecord {
        task_id: Ulid::from_seed(u64::try_from(started_at_ms)?, seed),
        session_id: name(session_id)?,
        started_at_ms,
        prompt_hash: Some(ContentHash::of(b"rename the parser")),
        prompt_excerpt: None,
        holdout: None,
    })
}

/// An event of `kind` in `task_id` at `ts_ms` on `path_text`, if any, with `lines`, if any.
fn event(
    task_id: Ulid,
    ts_ms: i64,
    kind: EventKind,
    place: Option<(&str, Option<(u32, u32)>)>,
) -> TestResult<EventRecord> {
    let (path, range) = match place {
        Some((text, lines)) => (
            Some(path(text)?),
            lines
                .map(|(start, end)| LineRange::new(start, end))
                .transpose()?,
        ),
        None => (None, None),
    };
    Ok(EventRecord {
        task_id: Some(task_id),
        session_id: Some(name("s1")?),
        ts_ms,
        kind,
        node_id: None,
        path,
        range,
        meta: EventMeta::default(),
    })
}

/// The ids of `sessions`, in order.
fn ids(sessions: &[cortex_store::SessionRow]) -> Vec<&str> {
    sessions
        .iter()
        .map(|row| row.record.session_id.as_str())
        .collect()
}

/// Sessions list by start time and then by id, from the given time on; a session's subagents are
/// those it started, in the same order; a session's tasks come in the order they started.
#[test]
fn sessions_and_tasks_list_in_order() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let tasks = [
        task(b"one", "s1", 100)?,
        task(b"other", "s2", 150)?,
        task(b"two", "s1", 200)?,
    ];
    let mut transaction = writer.transaction()?;
    for record in [
        session("s0", None, 40)?,
        session("s1", None, 50)?,
        session("sub-b", Some("s1"), 70)?,
        session("sub-a", Some("s1"), 70)?,
        session("s2", None, 60)?,
        session("sub-c", Some("s2"), 65)?,
    ] {
        transaction.record_session(&record)?;
    }
    for record in &tasks {
        transaction.start_task(record)?;
    }
    transaction.commit()?;
    let queries = writer.queries();
    assert_eq!(
        ids(&queries.sessions_started_since(50)?),
        ["s1", "s2", "sub-c", "sub-a", "sub-b"]
    );
    assert_eq!(
        ids(&queries.sessions_started_since(71)?),
        Vec::<&str>::new()
    );
    assert_eq!(
        ids(&queries.sessions_started_by(&name("s1")?)?),
        ["sub-a", "sub-b"]
    );
    assert_eq!(
        ids(&queries.sessions_started_by(&name("s0")?)?),
        Vec::<&str>::new()
    );
    let of_s1: Vec<(u64, Ulid)> = queries
        .tasks_of_session(&name("s1")?)?
        .iter()
        .map(|row| (row.seq, row.record.task_id))
        .collect();
    assert_eq!(of_s1, [(1, tasks[0].task_id), (3, tasks[2].task_id)]);
    assert!(queries.tasks_of_session(&name("s0")?)?.is_empty());
    Ok(())
}

/// Counted by hand. The first task made an edit stored as two rows (two stretches of one file at
/// one time), a read, two shell commands run side by side in the same millisecond, and its
/// subagent made a read just like the session's own: five calls. Its prompt, its stop and the
/// watcher's edit are no calls. The second made one search. The third made none and is left out,
/// and the fourth made calls but was not asked about. Outside any task the session made a shell
/// command and a read of two stretches, two calls, beside an edit the watcher saw; the subagent
/// made none.
#[test]
fn tool_calls_count_each_report_once() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let tasks = [
        task(b"first", "s1", 100)?,
        task(b"second", "s1", 200)?,
        task(b"third", "s1", 300)?,
        task(b"fourth", "s1", 400)?,
    ];
    let [first, second, third, fourth] = [0, 1, 2, 3].map(|index| tasks[index].task_id);
    let watched = |record: EventRecord| EventRecord {
        meta: EventMeta {
            source: Some(EventSource::Watcher),
            ..EventMeta::default()
        },
        ..record
    };
    let outside = |record: EventRecord| EventRecord {
        task_id: None,
        ..record
    };
    let subagent = |record: EventRecord| -> TestResult<EventRecord> {
        Ok(EventRecord {
            session_id: Some(name("s1/agent-1")?),
            ..record
        })
    };
    let read = |task_id, ts_ms| {
        event(
            task_id,
            ts_ms,
            EventKind::Read,
            Some(("src/lib.rs", Some((1, 20)))),
        )
    };
    let mut transaction = writer.transaction()?;
    transaction.record_session(&session("s1", None, 50)?)?;
    transaction.record_session(&session("s1/agent-1", Some("s1"), 60)?)?;
    append(
        &mut transaction,
        &tasks,
        &[
            outside(event(first, 55, EventKind::Shell, None)?),
            outside(event(
                first,
                60,
                EventKind::Read,
                Some(("src/lib.rs", Some((1, 2)))),
            )?),
            outside(event(
                first,
                60,
                EventKind::Read,
                Some(("src/lib.rs", Some((8, 9)))),
            )?),
            outside(watched(event(
                first,
                70,
                EventKind::Edit,
                Some(("src/lib.rs", None)),
            )?)),
            event(first, 110, EventKind::Prompt, None)?,
            event(
                first,
                120,
                EventKind::Edit,
                Some(("src/lib.rs", Some((3, 4)))),
            )?,
            event(
                first,
                120,
                EventKind::Edit,
                Some(("src/lib.rs", Some((9, 9)))),
            )?,
            read(first, 130)?,
            subagent(read(first, 130)?)?,
            event(first, 140, EventKind::Shell, None)?,
            event(first, 140, EventKind::Shell, None)?,
            watched(event(
                first,
                160,
                EventKind::Edit,
                Some(("src/lib.rs", None)),
            )?),
            event(first, 170, EventKind::Stop, None)?,
            event(second, 210, EventKind::Search, None)?,
            event(fourth, 410, EventKind::Tool, None)?,
        ],
    )?;
    transaction.commit()?;
    let queries = writer.queries();
    let counted = queries.tool_calls_of_tasks(&[first, second, third])?;
    assert_eq!(
        counted.into_iter().collect::<Vec<_>>(),
        sorted([(first, 5), (second, 1)])
    );
    assert!(queries.tool_calls_of_tasks(&[])?.is_empty());
    assert_eq!(queries.tool_calls_outside_tasks(&name("s1")?)?, 2);
    assert_eq!(queries.tool_calls_outside_tasks(&name("s1/agent-1")?)?, 0);
    Ok(())
}

/// The arms count every decided task of the repository, whatever its session or time: of four
/// tasks, two held out, one not and one undecided, the arms are two and one; with none decided,
/// both are empty.
#[test]
fn holdout_arms_count_every_decided_task() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let tasks = [
        task(b"first", "s1", 100)?,
        task(b"second", "s2", 200)?,
        task(b"third", "s1", 300)?,
        task(b"fourth", "s2", 400)?,
    ];
    let mut transaction = writer.transaction()?;
    transaction.record_session(&session("s1", None, 50)?)?;
    transaction.record_session(&session("s2", None, 60)?)?;
    append(&mut transaction, &tasks, &[])?;
    transaction.commit()?;
    assert_eq!(writer.queries().holdout_arms()?, (0, 0));
    // Nothing writes a task's arm yet, so the test sets the column as the writer will.
    let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
    for (index, arm) in [(0, Some(1)), (1, Some(0)), (2, Some(1)), (3, None)] {
        let id = tasks[index].task_id.to_string();
        connection.execute(
            "UPDATE tasks SET holdout = ?1 WHERE task_id = ?2",
            rusqlite::params![arm, id],
        )?;
    }
    assert_eq!(writer.queries().holdout_arms()?, (2, 1));
    Ok(())
}

/// Starts `tasks` and appends `events` in `transaction`.
fn append(
    transaction: &mut WriteTx<'_>,
    tasks: &[TaskRecord],
    events: &[EventRecord],
) -> TestResult {
    for record in tasks {
        transaction.start_task(record)?;
    }
    for record in events {
        transaction.append_event(record)?;
    }
    Ok(())
}

/// `pairs` in the order of their task ids, as a map lists them.
fn sorted<const N: usize>(mut pairs: [(Ulid, u64); N]) -> Vec<(Ulid, u64)> {
    pairs.sort_by_key(|(task, _)| task.to_string());
    pairs.to_vec()
}
