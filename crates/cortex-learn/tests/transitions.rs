//! What tasks read next, from their episodes: the pairs an episode walks, and what each task's
//! outcome adds to them over a worked history.

mod common;

use common::{Store, TestResult};
use cortex_core::{TaskOutcome, Ulid};
use cortex_learn::{add_transitions, transitions_of};
use cortex_store::{ChangedFile, EpisodeRecord, NodeId, Pitfalls, ReadFile, TransitionRow};

/// An episode that first read `read` in that order, then changed `changed`, with `outcome`.
fn episode(read: &[NodeId], changed: &[NodeId], outcome: TaskOutcome) -> EpisodeRecord {
    EpisodeRecord {
        task_id: Ulid::from_seed(1, b"transitions"),
        localize: read
            .iter()
            .map(|&node| ReadFile { node, reads: 1 })
            .collect(),
        change: changed
            .iter()
            .map(|&node| ChangedFile {
                node,
                lines: 1,
                symbols: Vec::new(),
            })
            .collect(),
        verify: Vec::new(),
        pitfalls: Pitfalls::default(),
        outcome,
        created_at_ms: 1,
    }
}

/// The pairs, worked by hand on files a, b, c, d: reads a, b, c then changes of d and c walk a-b,
/// b-c, c-d (only the first change counts); reads a, b then a change of b walk a-b alone (b to
/// itself is no step); reads a, b then a change of a walk a-b and b-a; a change alone, or one read
/// alone, walks nothing.
#[test]
fn an_episode_walks_its_reads_then_its_first_change() -> TestResult {
    let mut store = Store::new()?;
    let [a, b, c, d] = store.index(&["a.py", "b.py", "c.py", "d.py"])?[..] else {
        return Err("four nodes".into());
    };
    let walk = |read: &[NodeId], changed: &[NodeId]| {
        transitions_of(&episode(read, changed, TaskOutcome::Success))
    };
    assert_eq!(walk(&[a, b, c], &[d, c]), [(a, b), (b, c), (c, d)]);
    assert_eq!(walk(&[a, b], &[b]), [(a, b)]);
    assert_eq!(walk(&[a, b], &[a]), [(a, b), (b, a)]);
    assert_eq!(walk(&[], &[d]), []);
    assert_eq!(walk(&[a], &[]), []);
    Ok(())
}

/// Worked by hand, with links fading to 0.995 a task. Task 1 succeeds reading a, b, c and changing
/// d: a-b, b-c and c-d each 1.0, once, at 1. Task 2, outcome unknown, reads a, b and changes b:
/// a-b faded one task to 0.995, plus 0.3, 1.295, twice, at 2. Task 3 fails reading a, c: nothing
/// added, a-c never kept. Task 5 succeeds reading b, c: b-c faded four tasks, 0.995^4 =
/// 0.980149500625, plus 1, 1.980149500625, twice, at 5.
#[test]
fn each_outcome_adds_to_the_faded_weight() -> TestResult {
    let mut store = Store::new()?;
    let [a, b, c, d] = store.index(&["a.py", "b.py", "c.py", "d.py"])?[..] else {
        return Err("four nodes".into());
    };
    let tasks = [
        (episode(&[a, b, c], &[d], TaskOutcome::Success), 1),
        (episode(&[a, b], &[b], TaskOutcome::Unknown), 2),
        (episode(&[a, c], &[], TaskOutcome::Failure), 3),
        (episode(&[b, c], &[], TaskOutcome::Success), 5),
    ];
    for (record, seq) in &tasks {
        let mut transaction = store.writer.transaction()?;
        add_transitions(&mut transaction, record, (*seq, 0.995))?;
        transaction.commit()?;
    }
    let rows = store.writer.queries().transitions_after(None, 10)?;
    let expected = [
        (a, b, 1.295, 2, 2),
        (b, c, 1.980_149_500_625, 2, 5),
        (c, d, 1.0, 1, 1),
    ];
    assert_eq!(rows.len(), expected.len(), "{rows:?}");
    for (row, (src, dst, weight, count, last_seq)) in rows.iter().zip(expected) {
        let TransitionRow {
            src: got_src,
            dst: got_dst,
            weight: got_weight,
            count: got_count,
            last_seq: got_seq,
        } = *row;
        assert_eq!(
            (got_src, got_dst, got_count, got_seq),
            (src, dst, count, last_seq)
        );
        assert!((got_weight - weight).abs() < 1e-12, "{row:?}");
    }
    Ok(())
}
