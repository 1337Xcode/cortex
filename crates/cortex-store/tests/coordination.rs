//! Coordination as the store keeps it: a session's position on a node kept and replaced, the
//! positions since a time read back with when the task the session had then ended, what the store
//! knows of the sessions holding them, how an edited file is linked to the files they hold, and
//! notices queued once, told, read while they wait, and delivered once.

mod common;

use common::{Scratch, TestResult, index, symbol};
use cortex_core::{
    Agent, CommandKind, EdgeType, EventKind, EventMeta, Identifier, PositionMode, SymbolKind,
    TaskOutcome, Ulid,
};
use cortex_store::{
    Coupling, EdgeRecord, EventRecord, HandoffEnd, HandoffRecord, LastFailure, NodeId,
    NoticeRecord, PositionRecord, PositionRow, SeenSession, SessionRecord, TaskRecord, Writer,
};

/// Session `s1`'s first task.
const FIRST: &str = "01JC0000000000000000000000";

/// Session `s1`'s second task.
const SECOND: &str = "01JC0000000000000000000001";

/// Stores the files at `paths` and sessions `s1` (Claude Code; tasks at 1,000, ended at 1,500,
/// and at 2,000, open) and `s2` (Codex, started by `s1`, ended at 3,000; no task); returns the
/// files' nodes.
fn store_with(writer: &mut Writer, paths: &[&str]) -> TestResult<Vec<NodeId>> {
    let mut transaction = writer.transaction()?;
    let mut nodes = Vec::new();
    for path in paths {
        nodes.push(
            transaction
                .replace_file(&index(path, Vec::new(), Vec::new())?)?
                .file_node,
        );
    }
    for (session, agent, parent) in [
        ("s1", Agent::Claude, None),
        ("s2", Agent::Codex, Some("s1")),
    ] {
        transaction.record_session(&SessionRecord {
            session_id: Identifier::new(session)?,
            agent,
            agent_version: None,
            model: None,
            parent_session_id: parent.map(Identifier::new).transpose()?,
            cwd: None,
            transcript_path: None,
            started_at_ms: 500,
        })?;
    }
    transaction.end_session(&Identifier::new("s2")?, 3_000)?;
    for (task, at) in [(FIRST, 1_000), (SECOND, 2_000)] {
        transaction.start_task(&TaskRecord {
            task_id: Ulid::parse(task)?,
            session_id: Identifier::new("s1")?,
            started_at_ms: at,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
    }
    transaction.end_task(Ulid::parse(FIRST)?, 1_500, TaskOutcome::Success)?;
    transaction.commit()?;
    Ok(nodes)
}

/// A position of `session` on `node` in `mode`, taken at `since_ms` and last at `last_ms`.
fn position(
    session: &str,
    node: NodeId,
    mode: PositionMode,
    (since_ms, last_ms): (i64, i64),
) -> TestResult<PositionRecord> {
    Ok(PositionRecord {
        session_id: Identifier::new(session)?,
        node,
        mode,
        since_ms,
        last_ms,
    })
}

/// Worked by hand: `s1` reads `a.py` at 1,100 (its first task, which ended at 1,500), edits
/// `b.py` at 1,200 then again at 2,100 (its second task, open), and reads `c.py` at 1,700, after
/// its first task ended and before its second began; `s2` claims `a.py` at 900. Kept one per
/// session and node, the edit of `b.py` replaced by its later one. Read back since 1,000: `s1` on
/// `a.py` (task ended 1,500), on `b.py` (since 1,200, last 2,100, its task open) and on `c.py`
/// (no task end: the task it follows ended before it), not `s2`'s claim, last at 900. Read one by
/// one, the same, and nothing for `s2` on `c.py`.
#[test]
fn positions_are_kept_once_and_read_back_with_their_task_s_end() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [a, b, c] = store_with(&mut writer, &["a.py", "b.py", "c.py"])?[..] else {
        return Err("three nodes".into());
    };
    let mut transaction = writer.transaction()?;
    transaction.keep_position(&position("s1", a, PositionMode::Read, (1_100, 1_100))?)?;
    transaction.keep_position(&position("s1", b, PositionMode::Edit, (1_200, 1_200))?)?;
    transaction.keep_position(&position("s1", b, PositionMode::Edit, (1_200, 2_100))?)?;
    transaction.keep_position(&position("s1", c, PositionMode::Read, (1_700, 1_700))?)?;
    transaction.keep_position(&position("s2", a, PositionMode::Claim, (900, 900))?)?;
    transaction.commit()?;
    let reader = scratch.reader()?;
    let queries = reader.queries();
    assert_eq!(
        queries.position_of(&Identifier::new("s1")?, a)?,
        Some(PositionRow {
            record: position("s1", a, PositionMode::Read, (1_100, 1_100))?,
            task_ended_ms: Some(1_500),
        })
    );
    assert_eq!(
        queries.position_of(&Identifier::new("s1")?, b)?,
        Some(PositionRow {
            record: position("s1", b, PositionMode::Edit, (1_200, 2_100))?,
            task_ended_ms: None,
        })
    );
    assert_eq!(queries.position_of(&Identifier::new("s2")?, c)?, None);
    assert_eq!(
        queries.positions_since(1_000)?,
        [
            PositionRow {
                record: position("s1", a, PositionMode::Read, (1_100, 1_100))?,
                task_ended_ms: Some(1_500),
            },
            PositionRow {
                record: position("s1", b, PositionMode::Edit, (1_200, 2_100))?,
                task_ended_ms: None,
            },
            PositionRow {
                record: position("s1", c, PositionMode::Read, (1_700, 1_700))?,
                task_ended_ms: None,
            },
        ]
    );
    Ok(())
}

/// What the store knows of each session, by session: `s1` (Claude Code, no parent, open) with
/// events at 1,100 and 2,100 was last heard from at 2,100; `s2` (Codex, started by `s1`, ended at
/// 3,000) at 900; `s3`, never stored, is left out.
#[test]
fn each_session_s_agent_parent_end_and_last_event_are_read() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    store_with(&mut writer, &["a.py"])?;
    let mut transaction = writer.transaction()?;
    for (session, ts_ms) in [("s1", 1_100), ("s1", 2_100), ("s2", 900)] {
        transaction.append_event(&EventRecord {
            task_id: None,
            session_id: Some(Identifier::new(session)?),
            ts_ms,
            kind: EventKind::Read,
            node_id: None,
            path: None,
            range: None,
            meta: EventMeta::default(),
        })?;
    }
    transaction.commit()?;
    let reader = scratch.reader()?;
    let sessions = ["s1", "s2", "s3"].map(Identifier::new);
    let sessions = sessions.into_iter().collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        reader.queries().sessions_seen(&sessions)?,
        [
            SeenSession {
                session_id: Identifier::new("s1")?,
                agent: Agent::Claude,
                parent_session_id: None,
                ended_ms: None,
                last_event_ms: Some(2_100),
            },
            SeenSession {
                session_id: Identifier::new("s2")?,
                agent: Agent::Codex,
                parent_session_id: Some(Identifier::new("s1")?),
                ended_ms: Some(3_000),
                last_event_ms: Some(900),
            },
        ]
    );
    Ok(())
}

/// An edge of `edge_type` from `src` to `dst` weighing `weight`.
fn edge(src: NodeId, dst: NodeId, weight: f64) -> (NodeId, EdgeRecord) {
    (
        src,
        EdgeRecord {
            dst,
            weight,
            count: 1,
            last_seq: 0,
            is_directed: true,
        },
    )
}

/// Worked by hand. `api.py` defines `create_order` and `list_orders`; `web.py`'s `render` calls
/// both (0.9 and 0.6), `tests.py`'s `test_a` calls `create_order` (0.95), and `create_order` calls
/// `list_orders` (1.0, inside the file). In history `web.py` changes with `api.py` 0.7 of the
/// time and `api.py` with `web.py` 0.4; `other.py` changes with `api.py` 0.55; `lone.py` has no
/// link. Held `web.py`, `tests.py`, `other.py` and `lone.py`, the edited `api.py` is linked to
/// `web.py` by code at 0.9 (the heavier of its calls into `api.py`) and in history at 0.7 (from
/// `api.py`'s side: 0.4 runs the other way), to `tests.py` by code at 0.95, to `other.py` in
/// history at 0.55, and to `lone.py` not at all.
#[test]
fn an_edited_file_s_links_to_held_files_are_read_by_type() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let function = |name: &str| symbol(SymbolKind::Function, name, None, (name, name));
    let api = transaction.replace_file(&index(
        "api.py",
        vec![function("create_order")?, function("list_orders")?],
        Vec::new(),
    )?)?;
    let web = transaction.replace_file(&index("web.py", vec![function("render")?], Vec::new())?)?;
    let tests =
        transaction.replace_file(&index("tests.py", vec![function("test_a")?], Vec::new())?)?;
    let other = transaction
        .replace_file(&index("other.py", Vec::new(), Vec::new())?)?
        .file_node;
    let lone = transaction
        .replace_file(&index("lone.py", Vec::new(), Vec::new())?)?
        .file_node;
    let ([create, list], [render], [test_a]) = (
        &api.symbol_nodes[..],
        &web.symbol_nodes[..],
        &tests.symbol_nodes[..],
    ) else {
        return Err("api.py with two symbols, web.py and tests.py with one".into());
    };
    let (create, list, render, test_a) = (*create, *list, *render, *test_a);
    for (src, edge_type, edges) in [
        (
            render,
            EdgeType::Structural,
            vec![edge(render, create, 0.9), edge(render, list, 0.6)],
        ),
        (
            test_a,
            EdgeType::Structural,
            vec![edge(test_a, create, 0.95)],
        ),
        (create, EdgeType::Structural, vec![edge(create, list, 1.0)]),
        (
            api.file_node,
            EdgeType::CoChange,
            vec![
                edge(api.file_node, web.file_node, 0.7),
                edge(api.file_node, other, 0.55),
            ],
        ),
        (
            web.file_node,
            EdgeType::CoChange,
            vec![edge(web.file_node, api.file_node, 0.4)],
        ),
    ] {
        let records: Vec<EdgeRecord> = edges.into_iter().map(|(_, record)| record).collect();
        transaction.replace_edges(src, edge_type, &records)?;
    }
    transaction.commit()?;
    let reader = scratch.reader()?;
    let held = [web.file_node, tests.file_node, other, lone];
    let coupling = |held, edge_type, weight| Coupling {
        held,
        edge_type,
        weight,
    };
    assert_eq!(
        reader.queries().couplings(api.file_node, &held)?,
        [
            coupling(web.file_node, EdgeType::Structural, 0.9),
            coupling(web.file_node, EdgeType::CoChange, 0.7),
            coupling(tests.file_node, EdgeType::Structural, 0.95),
            coupling(other, EdgeType::CoChange, 0.55),
        ]
    );
    Ok(())
}

/// A notice for `to` saying `text`, made at `at_ms`.
fn notice(to: &str, text: &str, at_ms: i64) -> TestResult<NoticeRecord> {
    Ok(NoticeRecord {
        to_session: Identifier::new(to)?,
        text: text.to_owned(),
        at_ms,
    })
}

/// Worked by hand. Queued for `s1`: "first" at 1,000, "second" at 1,100, "first" again at 1,500
/// unless sent since 600 (it was, at 1,000: not queued), and "first" at 5,000 unless sent since
/// 2,000 (queued); for `s2`, "other" at 1,050; and "told" recorded as told to `s1` at 1,200.
/// Waiting for `s1`: both "first"s and "second", oldest first; made since 1,050: "second" and the
/// later "first"; "told" never waits. Sent or given to `s1` since 1,100, in the order recorded:
/// "second", "told", "first". The first "first" delivered no longer waits, and delivering it again
/// changes nothing.
#[test]
fn notices_are_queued_once_and_wait_until_delivered() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    store_with(&mut writer, &["a.py"])?;
    let (s1, s2) = (Identifier::new("s1")?, Identifier::new("s2")?);
    let mut transaction = writer.transaction()?;
    let first = transaction
        .queue_notice(&notice("s1", "first", 1_000)?, 0)?
        .ok_or("the first notice is queued")?;
    assert!(
        transaction
            .queue_notice(&notice("s1", "second", 1_100)?, 0)?
            .is_some()
    );
    assert!(
        transaction
            .queue_notice(&notice("s2", "other", 1_050)?, 0)?
            .is_some()
    );
    assert_eq!(
        transaction.queue_notice(&notice("s1", "first", 1_500)?, 600)?,
        None
    );
    transaction.record_told(&notice("s1", "told", 1_200)?)?;
    assert!(
        transaction
            .queue_notice(&notice("s1", "first", 5_000)?, 2_000)?
            .is_some()
    );
    transaction.commit()?;
    let waiting = |session: &Identifier, since_ms| -> TestResult<Vec<(String, i64)>> {
        let reader = scratch.reader()?;
        Ok(reader
            .queries()
            .notices_waiting(session, since_ms)?
            .into_iter()
            .map(|notice| (notice.text, notice.created_ms))
            .collect())
    };
    let texts = |found: &[(&str, i64)]| -> Vec<(String, i64)> {
        found
            .iter()
            .map(|(text, at)| ((*text).to_owned(), *at))
            .collect()
    };
    assert_eq!(
        waiting(&s1, 0)?,
        texts(&[("first", 1_000), ("second", 1_100), ("first", 5_000)])
    );
    assert_eq!(
        waiting(&s1, 1_050)?,
        texts(&[("second", 1_100), ("first", 5_000)])
    );
    assert_eq!(waiting(&s2, 0)?, texts(&[("other", 1_050)]));
    assert_eq!(
        scratch.reader()?.queries().notices_since(&s1, 1_100)?,
        ["second", "told", "first"]
    );
    let mut transaction = writer.transaction()?;
    assert!(transaction.deliver_notice(first, 1_200)?);
    assert!(!transaction.deliver_notice(first, 1_300)?);
    transaction.commit()?;
    assert_eq!(
        waiting(&s1, 0)?,
        texts(&[("second", 1_100), ("first", 5_000)])
    );
    Ok(())
}

/// A session's handoff is kept with its files, warnings and failure as written; a second one of the
/// same session takes the first's place; open handoffs read back newest first from a time; closing
/// one, by the session given it, leaves it out.
#[test]
fn handoffs_are_left_one_a_session_and_closed() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let nodes = store_with(&mut writer, &["a.py", "b.py", "c.py"])?;
    let [a, b, c] = nodes.as_slice() else {
        return Err("three nodes".into());
    };
    let s1 = Identifier::new("s1")?;
    let s2 = Identifier::new("s2")?;
    let left = |session: &Identifier, task: &str, at_ms| -> TestResult<HandoffRecord> {
        Ok(HandoffRecord {
            from_session: session.clone(),
            task_id: Ulid::parse(task)?,
            end: HandoffEnd::Left,
            edited: vec![*a, *c],
            read: vec![*b],
            open_warnings: Vec::new(),
            last_failure: Some(LastFailure {
                runner: Identifier::new("pytest")?,
                kind: CommandKind::Test,
                exit_code: 1,
            }),
            created_at_ms: at_ms,
        })
    };
    let first = left(&s1, FIRST, 1_500)?;
    let mut transaction = writer.transaction()?;
    transaction.leave_handoff(&first)?;
    transaction.commit()?;
    assert_eq!(
        writer.queries().open_handoff_of(&s1)?.map(|row| row.record),
        Some(first)
    );
    let second = HandoffRecord {
        end: HandoffEnd::Answered,
        last_failure: None,
        read: Vec::new(),
        ..left(&s1, SECOND, 2_500)?
    };
    let other = HandoffRecord {
        end: HandoffEnd::Stopped,
        ..left(&s2, FIRST, 2_000)?
    };
    let mut transaction = writer.transaction()?;
    transaction.leave_handoff(&second)?;
    transaction.leave_handoff(&other)?;
    transaction.commit()?;
    let open: Vec<HandoffRecord> = writer
        .queries()
        .open_handoffs_since(1_000)?
        .into_iter()
        .map(|row| row.record)
        .collect();
    assert_eq!(open, [second.clone(), other.clone()]);
    assert_eq!(writer.queries().open_handoffs_since(2_200)?.len(), 1);
    let mut transaction = writer.transaction()?;
    transaction.close_handoff(&s1, &s2)?;
    transaction.commit()?;
    assert_eq!(writer.queries().open_handoff_of(&s1)?, None);
    let open: Vec<HandoffRecord> = writer
        .queries()
        .open_handoffs_since(0)?
        .into_iter()
        .map(|row| row.record)
        .collect();
    assert_eq!(open, [other]);
    Ok(())
}
