//! What a task's work earns each node's interest, and the tier each score puts a node in: a
//! worked example of every increment, the decay of what was there before, and the landmarks.

mod common;

use std::collections::BTreeMap;

use common::{
    Given, GivenSpans, Store, TestResult, answered_task, edit, given_task, read, succeeded, tool,
};
use cortex_core::{
    Event, HookKind, InterestTier, RepoPath, ShellCommand, ToolKind, ToolResult, ToolTarget,
};
use cortex_learn::{Commands, EarnedInterest, Recorder, add_interest, interest_of, tier_now};
use cortex_store::{InterestRow, NodeId};

/// A search by `session` at `ts_ms` of the whole repository that found `hits`.
fn search(session: &str, ts_ms: i64, hits: &[&str]) -> TestResult<Event> {
    let target = ToolTarget {
        path: Some(RepoPath::root()),
        pattern: Some("divide".to_owned()),
        hits: hits
            .iter()
            .map(|path| RepoPath::new(path))
            .collect::<Result<_, _>>()?,
        ..ToolTarget::default()
    };
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Grep, target, succeeded(None, Some(80))),
    )
}

/// The shell command `line` run by `session` at `ts_ms`, which exited `exit_code`.
fn shell(session: &str, ts_ms: i64, line: &str, exit_code: i32) -> TestResult<Event> {
    let target = ToolTarget {
        command: Some(ShellCommand::cut(line)),
        ..ToolTarget::default()
    };
    let result = Some(ToolResult {
        exit_code: Some(exit_code),
        is_success: exit_code == 0,
        is_interrupted: false,
        output_bytes: Some(20),
        counts: None,
    });
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Shell, target, result),
    )
}

/// What `earned` holds for `nodes`, in that order, 0 for a node it holds nothing for, rounded to
/// the nearest thousandth so that the sums of tenths compare as written.
fn earned_for(earned: &EarnedInterest, nodes: &[NodeId]) -> Vec<f64> {
    nodes
        .iter()
        .map(|node| (earned.get(node).copied().unwrap_or(0.0) * 1_000.0).round() / 1_000.0)
        .collect()
}

/// The stored interest of `nodes`, in that order, as (score rounded to the nearest thousandth,
/// task, tier); none for a node with no row.
fn stored_for(
    store: &Store,
    nodes: &[NodeId],
) -> TestResult<Vec<Option<(f64, u64, InterestTier)>>> {
    let rows: BTreeMap<NodeId, (f64, u64, InterestTier)> = store
        .writer
        .queries()
        .interest_of(nodes)?
        .into_iter()
        .map(|row| {
            (
                row.node,
                (
                    (row.score * 1_000.0).round() / 1_000.0,
                    row.last_seq,
                    row.tier,
                ),
            )
        })
        .collect();
    Ok(nodes.iter().map(|node| rows.get(node).copied()).collect())
}

/// Adds `earned` to the stored interest at task `seq`, with the decay `decay`.
fn add(store: &mut Store, (seq, decay): (u64, f64), earned: &EarnedInterest) -> TestResult {
    let mut transaction = store.writer.transaction()?;
    add_interest(&mut transaction, (seq, decay), earned)?;
    transaction.commit()?;
    Ok(())
}

/// Interest by node, from `scores` read as pairs of a node and what it earns.
fn earned_of(scores: &[(NodeId, f64)]) -> EarnedInterest {
    scores.iter().copied().collect()
}

/// A task's work earns each node it touched, worked out by hand. `calc/ops.py` holds the functions
/// `add` over lines 1 to 5, `wrap` over 7 to 20 and `mul` over 10 to 15; `calc/util.py` holds
/// none; `main.py` and `notes.md` are files of their own. The task, in order:
///
/// - reads `calc/ops.py` lines 10 to 12, which `wrap` and `mul` both cover: 0.4 each, and with
///   each of them the file takes half, 0.2, and the folder `calc` a quarter, 0.1;
/// - reads the whole of `main.py`: 0.3 to the file;
/// - searches, finding `calc/ops.py`, `calc/util.py`, `main.py` and `notes.md`: the first two the
///   task goes on to edit, so 0.2 each, while `main.py`, which it read before the search and never
///   opens again, and `notes.md`, which it never opens at all, earn 0.05 each;
/// - edits `calc/ops.py` lines 1 to 2, which only `add` covers: 1.0 to `add`, 0.5 to the file,
///   0.25 to `calc`;
/// - runs `cargo test`, which exits 0: 0.5 to `calc/ops.py`, edited before it, and nothing to
///   `calc/util.py`, which it has not edited yet;
/// - edits `calc/util.py` line 1, which no symbol covers: 1.0 to the file itself;
/// - runs `pytest`, which exits 0: 0.5 to `calc/util.py`, edited before it; `calc/ops.py` was
///   vouched for already, and a file earns that once a task, however many checks pass after it.
///
/// So `add` 1.0, `wrap` 0.4, `mul` 0.4, `calc/ops.py` 0.2 + 0.2 + 0.2 + 0.5 + 0.5 = 1.6,
/// `calc` 0.1 + 0.1 + 0.25 = 0.45, `calc/util.py` 0.2 + 1.0 + 0.5 = 1.7, `main.py` 0.3 + 0.05 =
/// 0.35 and `notes.md` 0.05.
#[test]
fn a_task_s_work_earns_the_nodes_it_touched() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let ops = store.index_symbols(
        "calc/ops.py",
        &[("add", 1, 5), ("wrap", 7, 20), ("mul", 10, 15)],
    )?;
    let (ops_file, add_symbol, wrap, mul) = (ops[0], ops[1], ops[2], ops[3]);
    let util = store.index_symbols("calc/util.py", &[])?[0];
    let others = store.index(&["main.py", "notes.md"])?;
    let (main, notes) = (others[0], others[1]);
    let folder = store.folder_node("calc")?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            read("s1", 1_100, "calc/ops.py", &[(10, 12)])?,
            read("s1", 1_200, "main.py", &[])?,
            search(
                "s1",
                1_300,
                &["calc/ops.py", "calc/util.py", "main.py", "notes.md"],
            )?,
            edit("s1", 1_400, "calc/ops.py", &[(1, 2)])?,
            shell("s1", 1_500, "cargo test", 0)?,
            edit("s1", 1_600, "calc/util.py", &[(1, 1)])?,
            shell("s1", 1_700, "pytest", 0)?,
        ],
    )?;
    let earned = interest_of(store.writer.queries(), &state)?;
    let nodes = [add_symbol, wrap, mul, ops_file, folder, util, main, notes];
    assert_eq!(
        earned_for(&earned, &nodes),
        [1.0, 0.4, 0.4, 1.6, 0.45, 1.7, 0.35, 0.05]
    );
    Ok(())
}

/// A check that did not pass, a lint that did, a failed read and a file with no node earn nothing.
/// The task reads `gone.py`, which the store does not hold, edits `a.py`, runs `cargo test` which
/// exits 1 and `ruff check .` which exits 0, so only the edit earns anything: 1.0 to `a.py`, which
/// has no symbols.
#[test]
fn work_that_vouches_for_nothing_earns_nothing() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let a = store.index(&["a.py"])?[0];
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            read("s1", 1_100, "gone.py", &[(1, 4)])?,
            edit("s1", 1_200, "a.py", &[(1, 1)])?,
            shell("s1", 1_300, "cargo test", 1)?,
            shell("s1", 1_400, "ruff check .", 0)?,
        ],
    )?;
    let earned = interest_of(store.writer.queries(), &state)?;
    assert_eq!(earned_for(&earned, &[a]), [1.0]);
    assert_eq!(earned.len(), 1, "only the edited file earned anything");
    Ok(())
}

/// What a node earns is added to what the decay of the tasks since leaves of its score, worked out
/// by hand: 1.0 earned at task 1 and 0.4 at task 11 leaves 0.98 to the tenth, 0.817, plus 0.4,
/// which is 1.217 as of task 11. A score of 0.2 is still fog; above it the node is interesting.
#[test]
fn what_is_earned_is_added_to_what_decay_leaves() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py"])?;
    let (a, b) = (nodes[0], nodes[1]);
    add(&mut store, (1, 0.98), &earned_of(&[(a, 1.0), (b, 0.2)]))?;
    assert_eq!(
        stored_for(&store, &[a, b])?,
        [
            Some((1.0, 1, InterestTier::Interesting)),
            Some((0.2, 1, InterestTier::Dark)),
        ]
    );
    add(&mut store, (11, 0.98), &earned_of(&[(a, 0.4)]))?;
    assert_eq!(
        stored_for(&store, &[a, b])?,
        [
            Some((1.217, 11, InterestTier::Interesting)),
            Some((0.2, 1, InterestTier::Dark)),
        ]
    );
    Ok(())
}

/// The landmarks are the highest scoring files that reach 3.0, no more of them than the
/// repository's size allows: seven files allow five. Worked out by hand: of the scores 9.0, 3.0,
/// 5.0, 3.1, 3.5 and 4.0 for `f0` to `f5`, the five highest at or above 3.0 are `f0`, `f5`, `f2`,
/// `f4` and `f3`, and `f1` at exactly 3.0 is left out of the landmarks but is still interesting.
/// `f6`, which earned nothing, has no score at all.
#[test]
fn the_landmarks_are_the_highest_scoring_files_the_repository_allows() -> TestResult {
    let mut store = Store::new()?;
    let files = store.index(&[
        "f0.py", "f1.py", "f2.py", "f3.py", "f4.py", "f5.py", "f6.py",
    ])?;
    let scores = [9.0, 3.0, 5.0, 3.1, 3.5, 4.0];
    let earned = earned_of(
        &files
            .iter()
            .copied()
            .zip(scores)
            .collect::<Vec<(NodeId, f64)>>(),
    );
    add(&mut store, (1, 0.98), &earned)?;
    let tiers: Vec<Option<InterestTier>> = stored_for(&store, &files)?
        .into_iter()
        .map(|row| row.map(|(_, _, tier)| tier))
        .collect();
    use InterestTier::{Interesting, Landmark};
    assert_eq!(
        tiers,
        [
            Some(Landmark),
            Some(Interesting),
            Some(Landmark),
            Some(Landmark),
            Some(Landmark),
            Some(Landmark),
            None,
        ]
    );
    Ok(())
}

/// Files of equal score are taken in node order, so the same scores always give the same
/// landmarks: of six files at 4.0 in a repository of seven, which allows five, the five indexed
/// first are the landmarks.
#[test]
fn files_of_equal_score_become_landmarks_in_node_order() -> TestResult {
    let mut store = Store::new()?;
    let files = store.index(&[
        "f0.py", "f1.py", "f2.py", "f3.py", "f4.py", "f5.py", "f6.py",
    ])?;
    let earned = earned_of(
        &files
            .iter()
            .copied()
            .take(6)
            .map(|node| (node, 4.0))
            .collect::<Vec<(NodeId, f64)>>(),
    );
    add(&mut store, (1, 0.98), &earned)?;
    let tiers: Vec<Option<InterestTier>> = stored_for(&store, &files)?
        .into_iter()
        .map(|row| row.map(|(_, _, tier)| tier))
        .collect();
    use InterestTier::{Interesting, Landmark};
    assert_eq!(
        tiers,
        [
            Some(Landmark),
            Some(Landmark),
            Some(Landmark),
            Some(Landmark),
            Some(Landmark),
            Some(Interesting),
            None,
        ]
    );
    Ok(())
}

/// A landmark fades back into the fog as the tasks go by: what it earned is still its stored
/// score, as of the task it earned it at, but the decay of the tasks since is what its tier is
/// read from. Worked out by hand: `a.py` earns 3.5 at task 1 and is the repository's landmark;
/// at task 200, 198 tasks later, 3.5 times 0.98 to the hundred and ninety-ninth is 0.063, which
/// is below the 0.2 a node must stand above, so `a.py` is written back as fog while its score
/// stays 3.5 as of task 1.
#[test]
fn a_landmark_fades_back_into_the_fog() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py"])?;
    let (a, b) = (nodes[0], nodes[1]);
    add(&mut store, (1, 0.98), &earned_of(&[(a, 3.5)]))?;
    assert_eq!(
        stored_for(&store, &[a])?,
        [Some((3.5, 1, InterestTier::Landmark))]
    );
    add(&mut store, (200, 0.98), &earned_of(&[(b, 0.5)]))?;
    assert_eq!(
        stored_for(&store, &[a, b])?,
        [
            Some((3.5, 1, InterestTier::Dark)),
            Some((0.5, 200, InterestTier::Interesting)),
        ]
    );
    Ok(())
}

/// A file that has fallen out of the landmarks is written back as merely interesting: `a.py` is
/// the only file at first and is a landmark; six higher scoring files later push it out, and the
/// repository of seven files allows only five.
#[test]
fn a_file_that_falls_out_of_the_landmarks_is_written_back() -> TestResult {
    let mut store = Store::new()?;
    let files = store.index(&["a.py", "f1.py", "f2.py", "f3.py", "f4.py", "f5.py", "f6.py"])?;
    let a = files[0];
    add(&mut store, (1, 0.98), &earned_of(&[(a, 3.5)]))?;
    assert_eq!(
        stored_for(&store, &[a])?,
        [Some((3.5, 1, InterestTier::Landmark))]
    );
    let higher = earned_of(
        &files[1..]
            .iter()
            .copied()
            .map(|node| (node, 9.0))
            .collect::<Vec<(NodeId, f64)>>(),
    );
    add(&mut store, (2, 0.98), &higher)?;
    assert_eq!(
        stored_for(&store, &[a])?,
        [Some((3.5, 1, InterestTier::Interesting))]
    );
    Ok(())
}

/// The tier a stored row stands in later, worked out by hand at a decay of 0.98 a task: 0.3 stored
/// as interesting at task 1 is 0.3 x 0.98^99 = 0.0406 at task 100, fog; 5.0 at task 99 is 4.9 at
/// task 100, interesting; a landmark stays one however its score has faded (3.5 at task 1 is 0.47 at
/// task 100), until the landmarks are chosen again; 0.2 at the task it was stored is at the
/// threshold, fog; 0.21 is above it.
#[test]
fn a_stored_tier_is_read_as_of_the_task_that_asks() -> TestResult {
    let mut store = Store::new()?;
    let nodes = store.index(&["a.py", "b.py", "c.py", "d.py", "e.py"])?;
    let row = |at: usize, (score, last_seq): (f64, u64), tier: InterestTier| {
        nodes.get(at).map(|node| InterestRow {
            node: *node,
            score,
            last_seq,
            tier,
        })
    };
    let cases = [
        (
            row(0, (0.3, 1), InterestTier::Interesting),
            100,
            InterestTier::Dark,
        ),
        (
            row(1, (5.0, 99), InterestTier::Interesting),
            100,
            InterestTier::Interesting,
        ),
        (
            row(2, (3.5, 1), InterestTier::Landmark),
            100,
            InterestTier::Landmark,
        ),
        (row(3, (0.2, 7), InterestTier::Dark), 7, InterestTier::Dark),
        (
            row(4, (0.21, 7), InterestTier::Interesting),
            7,
            InterestTier::Interesting,
        ),
    ];
    for (row, seq, expected) in cases {
        let row = row.ok_or("a node")?;
        assert_eq!(
            tier_now(&row, (seq, 0.98)),
            expected,
            "{row:?} at task {seq}"
        );
    }
    Ok(())
}

/// A span the task's prompt was given earns more once the task reads or edits it, worked out by
/// hand. `calc/ops.py` holds `add` over lines 1 to 5, `wrap` over 7 to 20 and `mul` over 10 to
/// 15; `main.py` and `calc/util.py` hold none. The work order gave `calc/ops.py` lines 1 to 5 and
/// 10 to 15, `main.py` lines 1 to 3 and `calc/util.py` lines 1 to 2. The task reads `calc/ops.py`
/// lines 2 to 3 and edits `main.py` line 2:
///
/// - the read earns `add`, which covers it, 0.4, the file 0.2 and `calc` 0.1; it reaches the span
///   1 to 5, which earns `add` 0.3 more, the file 0.15 and `calc` 0.075; nothing reaches 10 to 15;
/// - the edit earns `main.py` 1.0, and reaches its span, which covers no symbol: 0.3 more to it;
/// - `calc/util.py` was given and never touched: nothing.
///
/// So `add` 0.7, `wrap` and `mul` 0, `calc/ops.py` 0.35, `calc` 0.175, `main.py` 1.3 and
/// `calc/util.py` 0. The same work after a work order the gate kept back, or in a task held out,
/// earns only what the steps earn: `add` 0.4, the file 0.2, `calc` 0.1 and `main.py` 1.0.
#[test]
fn a_span_given_earns_once_the_task_reads_or_edits_it() -> TestResult {
    let mut store = Store::new()?;
    let ops = store.index_symbols(
        "calc/ops.py",
        &[("add", 1, 5), ("wrap", 7, 20), ("mul", 10, 15)],
    )?;
    let (ops_file, add_symbol, wrap, mul) = (ops[0], ops[1], ops[2], ops[3]);
    let util = store.index_symbols("calc/util.py", &[])?[0];
    let main = store.index(&["main.py"])?[0];
    let folder = store.folder_node("calc")?;
    let nodes = [add_symbol, wrap, mul, ops_file, folder, main, util];
    let given: [GivenSpans<'_>; 3] = [
        (ops_file, &[(1, 5), (10, 15)]),
        (main, &[(1, 3)]),
        (util, &[(1, 2)]),
    ];
    let unaided = [0.4, 0.0, 0.0, 0.2, 0.1, 1.0, 0.0];
    let cases = [
        (Given::Shown, [0.7, 0.0, 0.0, 0.35, 0.175, 1.3, 0.0]),
        (Given::KeptBack, unaided),
        (Given::HeldOut, unaided),
    ];
    for (at, (fared, expected)) in (1_i64..).zip(cases) {
        let session = format!("s{at}");
        let at_ms = at * 1_000;
        let mut recorder = Recorder::new(false, 0.0, Commands::default());
        let state = given_task(
            (&mut store, &mut recorder),
            (&session, at_ms),
            (&given, fared),
            &[
                read(&session, at_ms + 100, "calc/ops.py", &[(2, 3)])?,
                edit(&session, at_ms + 200, "main.py", &[(2, 2)])?,
            ],
        )?;
        let earned = interest_of(store.writer.queries(), &state)?;
        assert_eq!(earned_for(&earned, &nodes), expected, "{fared:?}");
    }
    Ok(())
}
