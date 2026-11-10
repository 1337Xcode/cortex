//! What tasks read next, from their steps: the ways a task walks, what each task's outcome adds
//! over a worked history, and what a revert takes back.

mod common;

use common::{Store, TestResult, edit, event, prompt, read, start};
use cortex_core::{HookKind, StartSource, TaskOutcome};
use cortex_learn::{
    Commands, MAX_WALKED_READS, Recorder, TaskState, add_transitions, take_back_transitions,
    transitions_of,
};
use cortex_store::{NodeId, TransitionRow};

/// The state of a task of session `session` that read `reads` in order and then edited `edits`.
fn task(
    store: &mut Store,
    session: &str,
    (reads, edits): (&[&str], &[&str]),
) -> TestResult<TaskState> {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let mut events = vec![
        start(session, 100, StartSource::Startup)?,
        prompt(session, 101, "fix it")?,
    ];
    let mut at = 110;
    for path in reads {
        events.push(read(session, at, path, &[])?);
        at += 10;
    }
    for path in edits {
        events.push(edit(session, at, path, &[(1, 1)])?);
        at += 10;
    }
    events.push(event(HookKind::Stop, session, at + 100)?);
    let ended = store.record(&mut recorder, &events)?;
    ended
        .into_iter()
        .next()
        .ok_or_else(|| "no task ended".into())
}

/// The ways, worked by hand on indexed files a to d: reads a, b, c then edits of d and c walk
/// a-b, b-c, c-d (only the first change counts); reads a, b then an edit of b walk a-b alone (b to
/// itself is no step); reads a, b then an edit of `new.py`, a file the index does not hold, walk
/// a-b and no step to it; an edit alone walks nothing. Reads of 21 files before an edit walk the
/// first 20 (19 ways) and take no step to the edit: the 20th was not read just before it.
#[test]
fn a_task_walks_its_first_reads_then_its_first_change() -> TestResult {
    let mut store = Store::new()?;
    let names: Vec<String> = (0..22).map(|at| format!("f{at:02}.py")).collect();
    let mut paths: Vec<&str> = vec!["a.py", "b.py", "c.py", "d.py"];
    paths.extend(names.iter().map(String::as_str));
    let nodes = store.index(&paths)?;
    let [a, b, c, d, ..] = nodes[..] else {
        return Err("the nodes".into());
    };
    let walk = |store: &mut Store, session, reads, edits| -> TestResult<Vec<(NodeId, NodeId)>> {
        Ok(transitions_of(&task(store, session, (reads, edits))?))
    };
    assert_eq!(
        walk(
            &mut store,
            "s1",
            &["a.py", "b.py", "c.py"],
            &["d.py", "c.py"]
        )?,
        [(a, b), (b, c), (c, d)]
    );
    assert_eq!(
        walk(&mut store, "s2", &["a.py", "b.py"], &["b.py"])?,
        [(a, b)]
    );
    assert_eq!(
        walk(&mut store, "s3", &["a.py", "b.py"], &["new.py"])?,
        [(a, b)]
    );
    assert_eq!(walk(&mut store, "s4", &[], &["d.py"])?, []);
    let reads: Vec<&str> = names.iter().take(21).map(String::as_str).collect();
    let many = walk(&mut store, "s5", &reads, &[names[21].as_str()])?;
    let files = &nodes[4..];
    let expected: Vec<(NodeId, NodeId)> = files
        .windows(2)
        .take(MAX_WALKED_READS - 1)
        .map(|pair| (pair[0], pair[1]))
        .collect();
    assert_eq!(many, expected);
    Ok(())
}

/// The ways from a, held as `(dst, weight, count, last task)`, the weight to 12 places.
fn ways_from(store: &Store, from: NodeId) -> TestResult<Vec<(NodeId, f64, u32, u64)>> {
    Ok(store
        .writer
        .queries()
        .transitions_from(&[from])?
        .into_iter()
        .map(
            |TransitionRow {
                 dst,
                 weight,
                 count,
                 last_seq,
                 ..
             }| { (dst, (weight * 1e12).round() / 1e12, count, last_seq) },
        )
        .collect())
}

/// Worked by hand, ways fading to 0.995 a task. Task 1, a success, reads a, b and edits c: a-b and
/// b-c 1.0, once, at 1. Task 2, outcome unknown, reads a, b and edits b: a-b faded one task to
/// 0.995, plus 0.3, 1.295, twice, at 2. A failure reading a, c at 3 adds nothing. Task 1 found
/// reverted at task 4: a-b faded two tasks, 1.295 x 0.995^2 = 1.282082375, less task 1's 1.0
/// faded three tasks, 0.985074875, leaves 0.2970075, once, at 4; and b-c,
/// counting task 1 alone, removed. Task 2 found reverted too at 5: a-b counts no task, removed.
#[test]
fn outcomes_add_to_faded_ways_and_a_revert_takes_its_share_back() -> TestResult {
    let mut store = Store::new()?;
    let [a, b, c] = store.index(&["a.py", "b.py", "c.py"])?[..] else {
        return Err("three nodes".into());
    };
    let first = task(&mut store, "s1", (&["a.py", "b.py"], &["c.py"]))?;
    let second = task(&mut store, "s2", (&["a.py", "b.py"], &["b.py"]))?;
    let failed = task(&mut store, "s3", (&["a.py", "c.py"], &[]))?;
    for (state, seq, outcome) in [
        (&first, 1, TaskOutcome::Success),
        (&second, 2, TaskOutcome::Unknown),
        (&failed, 3, TaskOutcome::Failure),
    ] {
        let mut transaction = store.writer.transaction()?;
        add_transitions(&mut transaction, state, (seq, outcome, 0.995))?;
        transaction.commit()?;
    }
    assert_eq!(ways_from(&store, a)?, [(b, 1.295, 2, 2)]);
    assert_eq!(ways_from(&store, b)?, [(c, 1.0, 1, 1)]);

    let mut transaction = store.writer.transaction()?;
    take_back_transitions(
        &mut transaction,
        &first,
        (1, TaskOutcome::Success, 4, 0.995),
    )?;
    transaction.commit()?;
    assert_eq!(ways_from(&store, a)?, [(b, 0.2970075, 1, 4)]);
    assert_eq!(ways_from(&store, b)?, []);
    let mut transaction = store.writer.transaction()?;
    take_back_transitions(
        &mut transaction,
        &second,
        (2, TaskOutcome::Unknown, 5, 0.995),
    )?;
    transaction.commit()?;
    assert_eq!(ways_from(&store, a)?, []);
    Ok(())
}
