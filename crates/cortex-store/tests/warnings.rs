//! Warnings recorded and read back: one row for each file a warning named, with its kind, what it
//! was about and its strength, and whether its task's end found it resolved.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{Agent, Identifier, Ulid, WarningKind};
use cortex_store::{
    NodeId, SessionRecord, StoreError, TaskRecord, WarningCounts, WarningId, WarningRecord,
    WarningRow, Writer,
};

/// The first task's id.
const FIRST: &str = "01JC0000000000000000000000";

/// The second task's id.
const SECOND: &str = "01JC0000000000000000000001";

/// Stores the files at `paths` and, in session `s1`, the tasks `tasks`; returns the files' nodes.
fn store_with(writer: &mut Writer, paths: &[&str], tasks: &[Ulid]) -> TestResult<Vec<NodeId>> {
    let mut transaction = writer.transaction()?;
    let mut nodes = Vec::new();
    for path in paths {
        let change = transaction.replace_file(&index(path, Vec::new(), Vec::new())?)?;
        nodes.push(change.file_node);
    }
    let session = Identifier::new("s1")?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    for (at, task) in tasks.iter().enumerate() {
        transaction.start_task(&TaskRecord {
            task_id: *task,
            session_id: session.clone(),
            started_at_ms: 1_000 + i64::try_from(at)?,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
    }
    transaction.commit()?;
    Ok(nodes)
}

/// Records `records` in one transaction, returning their rows in order.
fn record_all(writer: &mut Writer, records: &[WarningRecord]) -> TestResult<Vec<WarningId>> {
    let mut transaction = writer.transaction()?;
    let ids = records
        .iter()
        .map(|record| transaction.record_warning(record))
        .collect::<Result<Vec<_>, _>>()?;
    transaction.commit()?;
    Ok(ids)
}

/// Three warnings, two in the first task and one in the second, read back by task as they were
/// recorded and unresolved; then two resolved, one each way, and an unknown row not found.
#[test]
fn warnings_are_read_back_and_resolved() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let (first, second) = (Ulid::parse(FIRST)?, Ulid::parse(SECOND)?);
    let paths = ["src/orders.py", "src/schema.py", "tests/test_orders.py"];
    let [orders, schema, test] = store_with(&mut writer, &paths, &[first, second])?[..] else {
        return Err("three nodes".into());
    };
    let companion = WarningRecord {
        task_id: first,
        at_ms: 2_000,
        kind: WarningKind::CoChange,
        source: orders,
        target: schema,
        strength: 0.75,
    };
    let caller = WarningRecord {
        kind: WarningKind::CallSite,
        source: schema,
        target: orders,
        strength: 0.9,
        ..companion
    };
    let tests = WarningRecord {
        task_id: second,
        at_ms: 3_000,
        kind: WarningKind::Test,
        source: orders,
        target: test,
        strength: 1.0,
    };
    let [companion_id, caller_id, tests_id] =
        record_all(&mut writer, &[companion, caller, tests])?[..]
    else {
        return Err("three rows".into());
    };
    let unresolved = |id, record| WarningRow {
        id,
        record,
        resolved: None,
    };
    assert_eq!(
        writer.queries().warnings_of_task(first)?,
        [
            unresolved(companion_id, companion),
            unresolved(caller_id, caller)
        ]
    );
    assert_eq!(
        writer.queries().warnings_of_task(second)?,
        [unresolved(tests_id, tests)]
    );

    // A warning recorded in a transaction that is let go is not held, so it cannot be resolved.
    let mut transaction = writer.transaction()?;
    let missing = transaction.record_warning(&tests)?;
    drop(transaction);
    let mut transaction = writer.transaction()?;
    assert!(transaction.resolve_warning(companion_id, true)?);
    assert!(transaction.resolve_warning(caller_id, false)?);
    assert!(!transaction.resolve_warning(missing, true)?);
    transaction.commit()?;
    let resolved: Vec<Option<bool>> = writer
        .queries()
        .warnings_of_task(first)?
        .iter()
        .map(|row| row.resolved)
        .collect();
    assert_eq!(resolved, [Some(true), Some(false)]);
    assert_eq!(
        writer.queries().warnings_of_task(second)?,
        [unresolved(tests_id, tests)]
    );
    Ok(())
}

/// A warning of a task the store does not hold is refused, and nothing is kept of it.
#[test]
fn a_warning_of_an_unknown_task_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let [orders] = store_with(&mut writer, &["src/orders.py"], &[])?[..] else {
        return Err("one node".into());
    };
    let unknown = Ulid::parse(FIRST)?;
    let mut transaction = writer.transaction()?;
    let refused = transaction.record_warning(&WarningRecord {
        task_id: unknown,
        at_ms: 2_000,
        kind: WarningKind::Test,
        source: orders,
        target: orders,
        strength: 1.0,
    });
    assert!(
        matches!(refused, Err(StoreError::Sqlite { .. })),
        "{refused:?}"
    );
    transaction.commit()?;
    assert_eq!(writer.queries().warnings_of_task(unknown)?, []);
    Ok(())
}

/// A row damaged outside Cortex is reported as damaged, naming the column: a kind no warning has,
/// then a resolution that is neither 0 nor 1.
#[test]
fn a_damaged_warning_row_is_reported() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let first = Ulid::parse(FIRST)?;
    let [orders] = store_with(&mut writer, &["src/orders.py"], &[first])?[..] else {
        return Err("one node".into());
    };
    record_all(
        &mut writer,
        &[WarningRecord {
            task_id: first,
            at_ms: 2_000,
            kind: WarningKind::Test,
            source: orders,
            target: orders,
            strength: 1.0,
        }],
    )?;
    drop(writer);
    for (damage, column) in [
        ("UPDATE warnings SET kind = 'tests'", "warnings.kind"),
        (
            "UPDATE warnings SET kind = 'test', resolved = 2",
            "warnings.resolved",
        ),
    ] {
        let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
        connection.execute(damage, [])?;
        drop(connection);
        let reader = scratch.reader()?;
        let read = reader.queries().warnings_of_task(first);
        assert!(
            matches!(read, Err(StoreError::Corrupt { column: found, row: 1 }) if found == column),
            "{read:?}"
        );
    }
    Ok(())
}

/// The counts: three files named, two of them marked, one of those resolved; with none stored,
/// all three counts are 0. An origin note, marked and resolved too, counts in none of them.
#[test]
fn warnings_are_counted() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    assert_eq!(writer.queries().warning_counts()?, WarningCounts::default());
    let first = Ulid::parse(FIRST)?;
    let [orders, schema, test] = store_with(
        &mut writer,
        &["src/orders.py", "src/schema.py", "tests/test_orders.py"],
        &[first],
    )?[..] else {
        return Err("three nodes".into());
    };
    let warned = |target| WarningRecord {
        task_id: first,
        at_ms: 2_000,
        kind: WarningKind::CoChange,
        source: orders,
        target,
        strength: 0.5,
    };
    let origin = WarningRecord {
        kind: WarningKind::Origin,
        ..warned(orders)
    };
    let ids = record_all(
        &mut writer,
        &[origin, warned(schema), warned(test), warned(orders)],
    )?;
    let mut transaction = writer.transaction()?;
    for (id, is_resolved) in ids.iter().zip([true, true, false]) {
        transaction.resolve_warning(*id, is_resolved)?;
    }
    transaction.commit()?;
    assert_eq!(
        writer.queries().warning_counts()?,
        WarningCounts {
            given: 3,
            labelled: 2,
            resolved: 1,
        }
    );
    Ok(())
}

/// Worked by hand. Files `a.py`, `b.py`, `c.py`; warnings of the first task: 1 from an edit of
/// `a.py` naming `b.py` at 10, 2 from `b.py` naming `a.py` at 20, 3 from `c.py` naming `b.py` at
/// 30, 4 from `a.py` naming `c.py` at 40, then resolved. Open about `a.py`: 2 then 1, the latest
/// first; 3 is about other files and 4 is resolved.
#[test]
fn the_open_warnings_about_a_file_are_read_latest_first() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let task = Ulid::parse(FIRST)?;
    let nodes = store_with(&mut writer, &["a.py", "b.py", "c.py"], &[task])?;
    let (a, b, c) = (nodes[0], nodes[1], nodes[2]);
    let warning = |at_ms, (source, target)| WarningRecord {
        task_id: task,
        at_ms,
        kind: WarningKind::CoChange,
        source,
        target,
        strength: 0.5,
    };
    let ids = record_all(
        &mut writer,
        &[
            warning(10, (a, b)),
            warning(20, (b, a)),
            warning(30, (c, b)),
            warning(40, (a, c)),
        ],
    )?;
    let mut transaction = writer.transaction()?;
    transaction.resolve_warning(ids[3], true)?;
    transaction.commit()?;
    let open: Vec<WarningId> = writer
        .queries()
        .open_warnings_about(a)?
        .iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(open, [ids[1], ids[0]]);
    Ok(())
}

/// The warnings that named a file, the latest first and within a limit, the origin notes left out:
/// orders.py was named by a cochange warning at 2,000 (resolved), a call-site warning at 2,500
/// (not marked yet), a test warning at 1,500 and an origin note at 9,000; schema.py by one
/// cochange warning at 2,000. So orders.py was named at 2,500, 2,000 and 1,500, with the
/// resolution of each as stored, two with a limit of 2, and schema.py once.
#[test]
fn the_warnings_that_named_a_file_come_latest_first() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let task = Ulid::parse(FIRST)?;
    let paths = ["src/orders.py", "src/schema.py"];
    let [orders, schema] = store_with(&mut writer, &paths, &[task])?[..] else {
        return Err("two nodes".into());
    };
    let warning = |kind, at_ms, source, target| WarningRecord {
        task_id: task,
        at_ms,
        kind,
        source,
        target,
        strength: 0.5,
    };
    let ids = record_all(
        &mut writer,
        &[
            warning(WarningKind::CoChange, 2_000, schema, orders),
            warning(WarningKind::CallSite, 2_500, schema, orders),
            warning(WarningKind::Test, 1_500, schema, orders),
            warning(WarningKind::Origin, 9_000, orders, orders),
            warning(WarningKind::CoChange, 2_000, orders, schema),
        ],
    )?;
    let first = *ids.first().ok_or("no rows")?;
    let mut transaction = writer.transaction()?;
    assert!(transaction.resolve_warning(first, true)?);
    transaction.commit()?;
    let queries = writer.queries();
    let seen = |node, limit| -> TestResult<Vec<(i64, Option<bool>)>> {
        Ok(queries
            .warnings_naming(node, limit)?
            .iter()
            .map(|row| (row.record.at_ms, row.resolved))
            .collect())
    };
    assert_eq!(
        seen(orders, 10)?,
        [(2_500, None), (2_000, Some(true)), (1_500, None)]
    );
    assert_eq!(seen(orders, 2)?, [(2_500, None), (2_000, Some(true))]);
    assert_eq!(seen(schema, 10)?, [(2_000, None)]);
    Ok(())
}
