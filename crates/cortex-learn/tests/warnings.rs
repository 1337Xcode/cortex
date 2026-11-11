//! What a task's end says of the warnings its edits got, on a task recorded event by event and
//! learnt from as the daemon learns: each resolved when the file it named was read or edited after
//! the warning came, worked out by hand from the task's steps.

mod common;

use common::{Store, TestResult, asked_task, edit, failed, read};
use cortex_core::{Config, Event, Ulid, WarningKind};
use cortex_learn::{Commands, Learner, Recorder};
use cortex_store::{NodeId, WarningRecord};

/// The task of session `session`, started at `at_ms` with `steps`, recorded in `store`; the
/// warnings of `warned`, each a kind, what it is about and the file it names, given in it at
/// `warned_at_ms`; and the task learnt from as the daemon does. Each warning's resolution, as the
/// store then holds it, in the order given.
fn resolved_after(
    store: &mut Store,
    (session, at_ms, steps): (&str, i64, &[Event]),
    (warned_at_ms, warned): (i64, &[(WarningKind, NodeId, NodeId)]),
) -> TestResult<(Ulid, Vec<Option<bool>>)> {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let state = asked_task((store, &mut recorder), (session, at_ms), "fix it", steps)?;
    let task_id = state.task_id();
    let mut transaction = store.writer.transaction()?;
    for (kind, source, target) in warned {
        transaction.record_warning(&WarningRecord {
            task_id,
            at_ms: warned_at_ms,
            kind: *kind,
            source: *source,
            target: *target,
            strength: 0.5,
        })?;
    }
    Learner::new()
        .learn(&mut transaction, &state, &Config::default())?
        .ok_or("nothing learnt")?;
    transaction.commit()?;
    let resolved = store
        .writer
        .queries()
        .warnings_of_task(task_id)?
        .iter()
        .map(|row| row.resolved)
        .collect();
    Ok((task_id, resolved))
}

/// The edit of `a.py` at 1,010 brings four warnings, naming `b.py`, `c.py`, `d.py` and `e.py`. By
/// hand, from the steps: `b.py` read at 1,020, after: resolved; `c.py` read at 1,005, before the
/// warning, and not again: not resolved; `d.py` edited at 1,030: resolved; `e.py` read at 1,040 by
/// a read that failed, which touched nothing: not resolved; and `a.py` itself, named by a warning
/// at the very moment it was edited, and not touched after: not resolved. An origin note about
/// `b.py` is not resolved by the read of `b.py` after it: it asks for no follow-up.
#[test]
fn a_warning_is_resolved_by_a_later_read_or_edit_of_its_file() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py", "c.py", "d.py", "e.py"])?;
    let [a, b, c, d, e] = nodes[..] else {
        return Err("five nodes".into());
    };
    let steps = [
        read("s1", 1_005, "c.py", &[])?,
        edit("s1", 1_010, "a.py", &[(1, 2)])?,
        read("s1", 1_020, "b.py", &[])?,
        edit("s1", 1_030, "d.py", &[(1, 2)])?,
        Event {
            tool_result: failed(),
            ..read("s1", 1_040, "e.py", &[])?
        },
    ];
    let warned = [
        (WarningKind::CoChange, a, b),
        (WarningKind::Test, a, c),
        (WarningKind::CallSite, a, d),
        (WarningKind::CoChange, a, e),
        (WarningKind::CallSite, b, a),
        (WarningKind::Origin, b, b),
    ];
    let (_, resolved) = resolved_after(&mut store, ("s1", 1_000, &steps), (1_010, &warned))?;
    assert_eq!(
        resolved,
        [
            Some(true),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
            Some(false)
        ]
    );
    Ok(())
}

/// Only the ended task's warnings are marked: a warning of another task, still open, stays
/// unmarked though the ended task read the file it names.
#[test]
fn only_the_ended_task_s_warnings_are_marked() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py"])?;
    let [a, b] = nodes[..] else {
        return Err("two nodes".into());
    };
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let open = common::prompt("s2", 500, "another task")?;
    store.record(
        &mut recorder,
        &[
            common::start("s2", 499, cortex_core::StartSource::Startup)?,
            open.clone(),
        ],
    )?;
    let open_task = common::task_of(&open)?;
    let mut transaction = store.writer.transaction()?;
    transaction.record_warning(&WarningRecord {
        task_id: open_task,
        at_ms: 600,
        kind: WarningKind::Test,
        source: a,
        target: b,
        strength: 0.5,
    })?;
    transaction.commit()?;
    let steps = [read("s1", 1_020, "b.py", &[])?];
    let (_, resolved) = resolved_after(
        &mut store,
        ("s1", 1_000, &steps),
        (1_010, &[(WarningKind::Test, a, b)]),
    )?;
    assert_eq!(resolved, [Some(true)]);
    let other = store.writer.queries().warnings_of_task(open_task)?;
    assert_eq!(
        other.iter().map(|row| row.resolved).collect::<Vec<_>>(),
        [None]
    );
    Ok(())
}

/// An edit of `a.py` at 1,010 brings a co-change warning naming `b.py` and an origin note naming
/// `a.py`, and the task ends with neither file touched after, its outcome unknown: the handoff it
/// leaves lists the co-change warning as open and not the origin note, which asks for no follow-up.
#[test]
fn a_handoff_lists_no_origin_note_as_open() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py"])?;
    let [a, b] = nodes[..] else {
        return Err("two nodes".into());
    };
    let steps = [edit("s1", 1_010, "a.py", &[(1, 2)])?];
    let warned = [(WarningKind::CoChange, a, b), (WarningKind::Origin, a, a)];
    let (task_id, _) = resolved_after(&mut store, ("s1", 1_000, &steps), (1_010, &warned))?;
    let given = store.writer.queries().warnings_of_task(task_id)?;
    let cochange = given
        .iter()
        .find(|row| row.record.kind == WarningKind::CoChange)
        .ok_or("no co-change warning")?
        .id;
    let handoffs = store.writer.queries().open_handoffs_since(0)?;
    let [handoff] = &handoffs[..] else {
        return Err(format!("one handoff, not {}", handoffs.len()).into());
    };
    assert_eq!(handoff.record.open_warnings, [cochange]);
    Ok(())
}
