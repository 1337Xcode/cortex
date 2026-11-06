//! Warnings recorded and read back: one row for each file a warning named, with its kind, what it
//! was about and its strength, and whether its task's end found it resolved.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{Agent, Identifier, Ulid, WarningKind};
use cortex_store::{
    NodeId, SessionRecord, StoreError, TaskRecord, WarningId, WarningRecord, WarningRow, Writer,
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
