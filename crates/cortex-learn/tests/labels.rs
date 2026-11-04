//! What a task's end says of the injections its prompt got, on tasks recorded event by event and
//! learnt from as the daemon learns: the label, whether one of the first three files was edited, or
//! read and named by a file the task edited, and the used count, how many of the files the task
//! read or edited, each worked out by hand from the task's steps.

mod common;

use common::{Store, TestResult, edit, event, prompt, read, start, task_of};
use cortex_core::{Config, Event, HookKind, Identifier, StartSource, Ulid};
use cortex_learn::{Commands, Learner, Recorder};
use cortex_store::{InjectedFile, InjectionRecord, NodeId};

/// Five files, `a.py` naming `b.py`, as the index writes them: their nodes, in path order.
fn five(store: &mut Store) -> TestResult<[NodeId; 5]> {
    let nodes = store.index(&["a.py", "b.py", "c.py", "d.py", "e.py"])?;
    let [a, b, c, d, e] = nodes[..] else {
        return Err("five nodes".into());
    };
    store.names(a, b)?;
    Ok([a, b, c, d, e])
}

/// An injection for the task `task`, which a prompt of `session` began at `at_ms`, of `files`, the
/// best first, kept back by the gate or held out as `(is_gated_out, is_holdout)` say.
fn injection(
    (session, task, at_ms): (&str, Ulid, i64),
    files: &[NodeId],
    (is_gated_out, is_holdout): (bool, bool),
) -> TestResult<InjectionRecord> {
    Ok(InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new(session)?,
        hook: HookKind::Prompt,
        at_ms,
        files: files
            .iter()
            .map(|node| InjectedFile {
                node: *node,
                spans: vec![(1, 2)],
                reason: None,
            })
            .collect(),
        tokens: 50,
        confidence: None,
        is_gated_out,
        is_holdout,
        text_hash: None,
        features: None,
    })
}

/// Records in `store` a session `session` started at `at_ms` and a prompt, then an injection of
/// `given`'s files kept as it says, then `steps` and the end of the answer, and learns from the
/// task as the daemon does; the injection's used count and label as the store then holds them.
fn labelled(
    store: &mut Store,
    (session, at_ms): (&str, i64),
    given: (&[NodeId], (bool, bool)),
    steps: &[Event],
) -> TestResult<(Option<u32>, Option<bool>)> {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let asked = prompt(session, at_ms + 1, "fix it")?;
    let started = start(session, at_ms, StartSource::Startup)?;
    store.record(&mut recorder, &[started, asked.clone()])?;
    let task = task_of(&asked)?;
    let mut transaction = store.writer.transaction()?;
    transaction.record_injection(&injection((session, task, at_ms + 1), given.0, given.1)?)?;
    transaction.commit()?;
    let mut rest = steps.to_vec();
    rest.push(event(HookKind::Stop, session, at_ms + 1_000)?);
    let ended = store.record(&mut recorder, &rest)?;
    let [state] = &ended[..] else {
        return Err("one task ended".into());
    };
    let mut transaction = store.writer.transaction()?;
    Learner::new()
        .learn(&mut transaction, state, &Config::default())?
        .ok_or("nothing learnt")?;
    transaction.commit()?;
    let rows = store.writer.queries().injections_of_task(task)?;
    let [row] = &rows[..] else {
        return Err(format!("{rows:?}").into());
    };
    Ok((row.used_count, row.label))
}

/// One case: the injection's files, the best first, how it was kept, the task's steps, and the
/// used count and label expected.
type Case<'a> = (&'a [NodeId], (bool, bool), Vec<Event>, (u32, bool));

/// Each case worked by hand from its steps, `a.py` naming `b.py`. An edited file among the first
/// three labels the injection 1 (c edited; a, fourth, only read: 2 used). A read file the task
/// then used labels it too (b read, a, which names b, edited: b is used; a was not given: 1 used).
/// Files only read do not (b and c read: 2 used, 0). A used file past the first three does not
/// either (b used, fourth: 1 used, 0). Nothing of it touched is 0 used, 0. A retrieval the gate
/// kept back and a held-out task's are labelled the same way: c edited, 1 used, 1; d only read,
/// 1 used, 0.
#[test]
fn an_injection_is_labelled_by_what_its_task_did() -> TestResult {
    let mut store = Store::new()?;
    let [a, b, c, d, e] = five(&mut store)?;
    let shown = (false, false);
    let cases: [Case<'_>; 7] = [
        (
            &[c, d, b, a],
            shown,
            vec![
                edit("s1", 1_010, "c.py", &[(1, 2)])?,
                read("s1", 1_020, "a.py", &[])?,
            ],
            (2, true),
        ),
        (
            &[b, c, d],
            shown,
            vec![
                read("s2", 2_010, "b.py", &[])?,
                edit("s2", 2_020, "a.py", &[(1, 2)])?,
            ],
            (1, true),
        ),
        (
            &[b, c, d],
            shown,
            vec![
                read("s3", 3_010, "b.py", &[])?,
                read("s3", 3_020, "c.py", &[])?,
            ],
            (2, false),
        ),
        (
            &[c, d, e, b],
            shown,
            vec![
                read("s4", 4_010, "b.py", &[])?,
                edit("s4", 4_020, "a.py", &[(1, 2)])?,
            ],
            (1, false),
        ),
        (
            &[c, d],
            shown,
            vec![read("s5", 5_010, "e.py", &[])?],
            (0, false),
        ),
        (
            &[c, d],
            (true, false),
            vec![edit("s6", 6_010, "c.py", &[(1, 2)])?],
            (1, true),
        ),
        (
            &[d],
            (false, true),
            vec![read("s7", 7_010, "d.py", &[])?],
            (1, false),
        ),
    ];
    for (at, (files, kept, steps, (used, label))) in (1_i64..).zip(cases) {
        let session = format!("s{at}");
        let found = labelled(&mut store, (&session, at * 1_000), (files, kept), &steps)?;
        assert_eq!(found, (Some(used), Some(label)), "{session}");
    }
    Ok(())
}

/// Only a task's injections are labelled: a session's primer, which no task got, stays unlabelled
/// while a task's work order is labelled 1 used, 1 (c edited); and a task whose prompt got no
/// injection is learnt from with nothing labelled.
#[test]
fn only_a_task_s_injections_are_labelled() -> TestResult {
    let mut store = Store::new()?;
    let [_, _, c, _, _] = five(&mut store)?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.record(&mut recorder, &[start("s0", 500, StartSource::Startup)?])?;
    let primer = InjectionRecord {
        task_id: None,
        session_id: Identifier::new("s0")?,
        hook: HookKind::SessionStart,
        at_ms: 500,
        files: Vec::new(),
        tokens: 40,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: None,
        features: None,
    };
    let mut transaction = store.writer.transaction()?;
    transaction.record_injection(&primer)?;
    transaction.commit()?;
    let given = labelled(
        &mut store,
        ("s1", 1_000),
        (&[c], (false, false)),
        &[edit("s1", 1_010, "c.py", &[(1, 2)])?],
    )?;
    assert_eq!(given, (Some(1), Some(true)));
    let primers = store
        .writer
        .queries()
        .injections_of_session(&Identifier::new("s0")?)?;
    let [kept] = &primers[..] else {
        return Err(format!("{primers:?}").into());
    };
    assert_eq!((kept.used_count, kept.label), (None, None));
    let asked = prompt("s2", 2_001, "fix it")?;
    let ended = store.record(
        &mut recorder,
        &[
            start("s2", 2_000, StartSource::Startup)?,
            asked.clone(),
            edit("s2", 2_010, "c.py", &[(1, 2)])?,
            event(HookKind::Stop, "s2", 3_000)?,
        ],
    )?;
    let [state] = &ended[..] else {
        return Err("one task ended".into());
    };
    let mut transaction = store.writer.transaction()?;
    let learnt = Learner::new().learn(&mut transaction, state, &Config::default())?;
    transaction.commit()?;
    assert!(learnt.is_some());
    let rows = store
        .writer
        .queries()
        .injections_of_task(task_of(&asked)?)?;
    assert!(rows.is_empty());
    Ok(())
}
