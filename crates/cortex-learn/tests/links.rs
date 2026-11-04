//! The links a task's work draws between the nodes it used together: what each pair gains by what
//! became of the task, the roles the nodes played, the bounds on a weight, and the working set's
//! cap.

mod common;

use std::collections::BTreeMap;

use common::{Store, TestResult, answered_task, edit, read};
use cortex_core::{EdgeType, TaskOutcome};
use cortex_learn::{Commands, EarnedLinks, Linking, Recorder, TaskState, add_links, links_of};
use cortex_store::NodeId;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

/// The settings the tests learn with: the documented defaults, at task `seq`, after `outcome`.
fn linking(seq: u64, outcome: TaskOutcome) -> Linking {
    Linking {
        seq,
        outcome,
        eta: 0.1,
        link_decay: 0.995,
        interest_decay: 0.98,
    }
}

/// What `links` holds for each pair, rounded to the nearest ten-thousandth so that products of
/// tenths compare as written, by pair.
fn rounded(links: &EarnedLinks) -> BTreeMap<(NodeId, NodeId), f64> {
    links
        .iter()
        .map(|(pair, weight)| (*pair, (weight * 10_000.0).round() / 10_000.0))
        .collect()
}

/// The first two of `nodes`: a file's node and its one symbol's, as the index gives them.
fn two(nodes: Vec<NodeId>) -> TestResult<(NodeId, NodeId)> {
    let mut nodes = nodes.into_iter();
    let first = nodes.next().ok_or("no node")?;
    let second = nodes.next().ok_or("no second node")?;
    Ok((first, second))
}

/// The pair of `left` and `right`, the smaller node first, as the links hold it.
fn pair(left: NodeId, right: NodeId) -> (NodeId, NodeId) {
    if left.get() <= right.get() {
        (left, right)
    } else {
        (right, left)
    }
}

/// The stored link between `left` and `right`, as (weight rounded to the nearest ten-thousandth,
/// observations, task, whether it points one way); none when there is no link.
fn stored(
    store: &Store,
    (left, right): (NodeId, NodeId),
) -> TestResult<Option<(f64, u32, i64, bool)>> {
    let pair = pair(left, right);
    Ok(store
        .writer
        .queries()
        .edges_among(&[pair.0, pair.1], EdgeType::CoAccess)?
        .into_iter()
        .find(|row| (row.src, row.edge.dst) == pair)
        .map(|row| {
            (
                (row.edge.weight * 10_000.0).round() / 10_000.0,
                row.edge.count,
                row.edge.last_seq,
                row.edge.is_directed,
            )
        }))
}

/// A repository of three files with a symbol each, `main.py` naming `calc/ops.py` and that in its
/// turn naming `calc/util.py`, with their symbols naming each other the same way, as the index
/// writes it: the files' nodes and then the symbols', in the order `calc/ops.py`,
/// `calc/util.py`, `main.py`.
struct Three {
    /// `calc/ops.py` and its `add`.
    ops: (NodeId, NodeId),
    /// `calc/util.py` and its `fmt`.
    util: (NodeId, NodeId),
    /// `main.py` and its `run`.
    main: (NodeId, NodeId),
}

impl Three {
    /// Indexes them.
    fn index(store: &mut Store) -> TestResult<Self> {
        let three = Self {
            ops: two(store.index_symbols("calc/ops.py", &[("add", 1, 5)])?)?,
            util: two(store.index_symbols("calc/util.py", &[("fmt", 1, 8)])?)?,
            main: two(store.index_symbols("main.py", &[("run", 1, 20)])?)?,
        };
        store.names(three.main.0, three.ops.0)?;
        store.names(three.main.1, three.ops.1)?;
        // A file the task only read names another of them, which raises nothing: a read is raised
        // only by a file the task edited.
        store.names(three.ops.0, three.util.0)?;
        store.names(three.ops.1, three.util.1)?;
        Ok(three)
    }

    /// A task that edits `main.py`, reads `calc/ops.py`, which `main.py` names, and reads
    /// `calc/util.py`, which nothing it edited names.
    fn task(
        &self,
        store: &mut Store,
        recorder: &mut Recorder,
        session: &str,
    ) -> TestResult<TaskState> {
        answered_task(
            (store, recorder),
            (session, 1_000),
            &[
                edit(session, 1_100, "main.py", &[(1, 3)])?,
                read(session, 1_200, "calc/ops.py", &[(1, 5)])?,
                read(session, 1_300, "calc/util.py", &[(1, 8)])?,
            ],
        )
    }
}

/// A successful task links every pair of the nodes it used, at file level and at symbol level
/// separately, by what each node was to it. Worked out by hand: `main.py` was edited, so it counts
/// 1.0; `calc/ops.py` was read and is named by `main.py`, which the task edited, so it counts 0.6;
/// `calc/util.py` was only read, so it counts 0.2. With the learning rate 0.1 and a successful
/// task's full share, the files gain 0.1 x 1.0 x 0.6 = 0.06 for `main.py` with `calc/ops.py`,
/// 0.1 x 1.0 x 0.2 = 0.02 for `main.py` with `calc/util.py`, and 0.1 x 0.6 x 0.2 = 0.012 for
/// `calc/ops.py` with `calc/util.py`. The symbols the lines touched gain the same three, `run`
/// counting 1.0, `add` 0.6 since `run` names it, and `fmt` 0.2. No pair joins a file to a symbol.
#[test]
fn a_successful_task_links_what_it_used_together() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let three = Three::index(&mut store)?;
    let state = three.task(&mut store, &mut recorder, "s1")?;
    let links = links_of(
        store.writer.queries(),
        &state,
        &linking(1, TaskOutcome::Success),
    )?;
    let expected: BTreeMap<(NodeId, NodeId), f64> = [
        (pair(three.main.0, three.ops.0), 0.06),
        (pair(three.main.0, three.util.0), 0.02),
        (pair(three.ops.0, three.util.0), 0.012),
        (pair(three.main.1, three.ops.1), 0.06),
        (pair(three.main.1, three.util.1), 0.02),
        (pair(three.ops.1, three.util.1), 0.012),
    ]
    .into_iter()
    .collect();
    assert_eq!(rounded(&links), expected);
    Ok(())
}

/// What became of a task decides what its pairs gain: a task that was seen to work teaches in
/// full, one nobody can vouch for three tenths as much, and one that failed or was left teaches
/// nothing. Worked out by hand for `main.py` with `calc/ops.py`, which a successful task gains
/// 0.06: unknown gains 0.3 of that, 0.018, while failure and abandonment gain nothing at all, so
/// those tasks draw no links.
#[test]
fn what_became_of_a_task_decides_what_its_links_gain() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let three = Three::index(&mut store)?;
    let cases = [
        (TaskOutcome::Success, Some(0.06)),
        (TaskOutcome::Unknown, Some(0.018)),
        (TaskOutcome::Failure, None),
        (TaskOutcome::Abandoned, None),
    ];
    for (at, (outcome, expected)) in cases.into_iter().enumerate() {
        let session = format!("s{at}");
        let state = three.task(&mut store, &mut recorder, &session)?;
        let links = links_of(store.writer.queries(), &state, &linking(1, outcome))?;
        let main_with_ops = rounded(&links)
            .get(&pair(three.main.0, three.ops.0))
            .copied();
        assert_eq!(main_with_ops, expected, "{outcome:?}");
        if expected.is_none() {
            assert!(links.is_empty(), "{outcome:?} drew links");
        }
    }
    Ok(())
}

/// A reverted task weakens the links between the files it edited together and leaves every other
/// pair alone: what those files had in common was undone. Worked out by hand: the task edits
/// `main.py` and `calc/ops.py` and reads `calc/util.py`, so the one pair of edited files loses
/// 0.05 x 1.0 x 1.0, and neither the pairs with the file it only read nor the symbols it touched
/// change at all.
#[test]
fn a_reverted_task_weakens_the_files_it_edited_together() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let three = Three::index(&mut store)?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            edit("s1", 1_100, "main.py", &[(1, 3)])?,
            edit("s1", 1_200, "calc/ops.py", &[(1, 5)])?,
            read("s1", 1_300, "calc/util.py", &[(1, 8)])?,
        ],
    )?;
    let links = links_of(
        store.writer.queries(),
        &state,
        &linking(1, TaskOutcome::Reverted),
    )?;
    let expected: BTreeMap<(NodeId, NodeId), f64> = [(pair(three.main.0, three.ops.0), -0.05)]
        .into_iter()
        .collect();
    assert_eq!(rounded(&links), expected);
    Ok(())
}

/// A link's weight is added to what the decay of the tasks since leaves of it, and is held between
/// nothing and five. Worked out by hand: a pair linked at 1.0 as of task 1 and gaining 0.06 at
/// task 11 keeps 0.995 to the tenth of it, 0.9511, for 1.0111, with two observations behind it; a
/// pair at 4.98 as of task 1 gaining 0.06 at task 2 would stand at 4.9551 + 0.06 = 5.0151, so it
/// is held at 5.0; a pair at 1.0111 losing 2.0 stands at nothing rather than below it; and a pair
/// with no link yet and nothing to gain is left unwritten.
#[test]
fn a_link_is_added_to_what_decay_leaves_and_held_within_bounds() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py", "c.py", "d.py"])?;
    let (a, b, c, d) = (nodes[0], nodes[1], nodes[2], nodes[3]);
    let first: EarnedLinks = [(pair(a, b), 1.0), (pair(c, d), 4.98)]
        .into_iter()
        .collect();
    let mut transaction = store.writer.transaction()?;
    let written = add_links(&mut transaction, &linking(1, TaskOutcome::Success), &first)?;
    assert_eq!(written, first, "nothing near a bound: written as earned");
    transaction.commit()?;
    assert_eq!(stored(&store, (a, b))?, Some((1.0, 1, 1, false)));
    assert_eq!(stored(&store, (c, d))?, Some((4.98, 1, 1, false)));
    let ceiling: EarnedLinks = [(pair(c, d), 0.06)].into_iter().collect();
    let mut transaction = store.writer.transaction()?;
    add_links(
        &mut transaction,
        &linking(2, TaskOutcome::Success),
        &ceiling,
    )?;
    transaction.commit()?;
    assert_eq!(stored(&store, (c, d))?, Some((5.0, 2, 2, false)));
    let again: EarnedLinks = [(pair(a, b), 0.06)].into_iter().collect();
    let mut transaction = store.writer.transaction()?;
    add_links(&mut transaction, &linking(11, TaskOutcome::Success), &again)?;
    transaction.commit()?;
    assert_eq!(stored(&store, (a, b))?, Some((1.0111, 2, 11, false)));
    let weaken: EarnedLinks = [(pair(a, b), -2.0), (pair(b, c), -0.05)]
        .into_iter()
        .collect();
    let mut transaction = store.writer.transaction()?;
    add_links(
        &mut transaction,
        &linking(12, TaskOutcome::Reverted),
        &weaken,
    )?;
    transaction.commit()?;
    assert_eq!(stored(&store, (a, b))?, Some((0.0, 3, 12, false)));
    assert_eq!(stored(&store, (b, c))?, None, "a pair with nothing to lose");
    Ok(())
}

/// The weights of one pair after each task of `deltas` draws or weakens it in turn, one task
/// after another, read back from the store.
fn drawn_over_tasks(deltas: &[f64]) -> TestResult<Vec<f64>> {
    let mut store = Store::new()?;
    let (a, b) = two(store.index(&["a.py", "b.py"])?)?;
    let mut weights = Vec::new();
    for (at, delta) in deltas.iter().enumerate() {
        let seq = u64::try_from(at)? + 1;
        let links: EarnedLinks = [(pair(a, b), *delta)].into_iter().collect();
        let mut transaction = store.writer.transaction()?;
        add_links(
            &mut transaction,
            &linking(seq, TaskOutcome::Success),
            &links,
        )?;
        transaction.commit()?;
        if let Some((weight, _, _, _)) = stored(&store, (a, b))? {
            weights.push(weight);
        }
    }
    Ok(weights)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// However many tasks draw a link and weaken it again, its weight never falls below nothing
    /// nor rises above five.
    #[test]
    fn a_link_s_weight_stays_within_its_bounds(
        deltas in proptest::collection::vec(-3.0..3.0_f64, 1..12),
    ) {
        let weights = drawn_over_tasks(&deltas)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        for weight in weights {
            prop_assert!(weight >= 0.0, "{weight} fell below nothing");
            prop_assert!(weight <= 5.0, "{weight} rose above five");
        }
    }
}

/// A task that touched more nodes than the working set holds keeps the nodes that mattered most:
/// the ones it edited first, then by interest, then the smaller node. Worked out by hand: the task
/// reads 44 files and edits the last of them, so the set keeps the edited file and the 39 read
/// files with the smallest nodes, which is 40 nodes and 780 pairs; the five read files with the
/// largest nodes are in none of them.
#[test]
fn a_task_too_large_keeps_the_nodes_that_mattered_most() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let paths: Vec<String> = (0..45).map(|at| format!("f{at:02}.py")).collect();
    let nodes = store.index(&paths.iter().map(String::as_str).collect::<Vec<&str>>())?;
    let mut steps = Vec::new();
    for (at, path) in paths.iter().enumerate() {
        steps.push(read("s1", 1_100 + i64::try_from(at)?, path, &[])?);
    }
    steps.push(edit("s1", 1_200, "f44.py", &[])?);
    let state = answered_task((&mut store, &mut recorder), ("s1", 1_000), &steps)?;
    let links = links_of(
        store.writer.queries(),
        &state,
        &linking(1, TaskOutcome::Success),
    )?;
    assert_eq!(links.len(), 780, "every pair of the 40 nodes kept");
    assert!(
        links.keys().all(|(left, right)| left.get() < right.get()),
        "every pair is held with the smaller node first"
    );
    let edited = nodes[44];
    let with_edited = links
        .keys()
        .filter(|(left, right)| *left == edited || *right == edited)
        .count();
    assert_eq!(with_edited, 39, "the edited file pairs with every other");
    for dropped in &nodes[39..44] {
        assert!(
            !links
                .keys()
                .any(|(left, right)| left == dropped || right == dropped),
            "a file past the cap was kept"
        );
    }
    Ok(())
}
