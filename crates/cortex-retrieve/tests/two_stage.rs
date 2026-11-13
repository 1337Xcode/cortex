//! Spreading in two stages, as huge repositories spread: over files first, then over the symbols of
//! the best files alone, worked out by hand on small graphs.

use cortex_core::{EdgeType, NodeKind, TypeWeights};
use cortex_retrieve::{
    Graph, GraphError, L1_TOLERANCE, LinkInput, NodeInput, STAGED_FILES, Standing, Weighing,
    two_stage,
};

/// The restart probability the settings default to.
const TELEPORT: f64 = 0.5;

/// The settings' weights, nothing faded: a reference weighs its confidence, a file's hold on its
/// symbols 0.1.
fn weighing() -> Weighing {
    Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    }
}

/// A graph of `files`, numbered as given, each with the symbols given as (symbol, file), and the
/// references given as (from, to), each tied at 1.0.
fn graph_of(files: &[i64], symbols: &[(i64, i64)], refs: &[(i64, i64)]) -> Graph<i64> {
    let mut nodes: Vec<NodeInput<i64>> = files
        .iter()
        .map(|id| NodeInput {
            id: *id,
            kind: NodeKind::File,
            file: Some(*id),
            standing: Standing::default(),
        })
        .collect();
    nodes.extend(symbols.iter().map(|(id, file)| NodeInput {
        id: *id,
        kind: NodeKind::Symbol,
        file: Some(*file),
        standing: Standing::default(),
    }));
    let links: Vec<LinkInput<i64>> = refs
        .iter()
        .map(|(src, dst)| LinkInput {
            src: *src,
            dst: *dst,
            kind: EdgeType::Structural,
            weight: 1.0,
            last_seq: 0,
        })
        .collect();
    #[expect(
        clippy::expect_used,
        reason = "a test graph of a few dozen nodes always builds"
    )]
    Graph::build(nodes, links, &weighing()).expect("a test graph builds")
}

/// The index of node `id` in `graph`.
fn at(graph: &Graph<i64>, id: i64) -> u32 {
    graph.index_of(id).unwrap_or(u32::MAX)
}

/// Whether `actual` is `expected` within what the push may leave unspread.
fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= L1_TOLERANCE
}

/// Files `A` (1), `B` (2) and `C` (3), each with one symbol, `a` (11), `b` (12) and `c` (13); `a`
/// calls `b`. The seed is `a`.
///
/// First stage, over files: the seed starts at `A`; `A`'s links to other files are `a`'s call,
/// landing in `B`, and `B`'s are the call back, landing in `A`, so each hands all it spreads to the
/// other. With restart 0.5: `A` = 0.5 + 0.5 `B`, `B` = 0.5 `A`, so `A` 2/3 and `B` 1/3; `C` gets
/// nothing, and no symbol holds anything.
///
/// Second stage, over `A`, `a`, `B` and `b`: `a` links to `b` (1) and `A` (0.1), `b` to `a` (1) and
/// `B` (0.1), each file to its symbol (0.1). With k = 1/1.1 and m = 0.1/1.1: `A` = 0.5 m `a`, `B` =
/// 0.5 m `b`, `b` = 0.5 k `a` + 0.5 `B`, `a` = 0.5 + 0.5 k `b` + 0.5 `A`; so `b` = 0.4651163 `a`
/// and `a` = 0.5 / 0.7658562 = 0.6528578, `b` = 0.3036548.
#[test]
fn files_first_then_the_symbols_of_the_best() -> Result<(), GraphError> {
    let graph = graph_of(&[1, 2, 3], &[(11, 1), (12, 2), (13, 3)], &[(11, 12)]);
    let staged = two_stage(&graph, &[(at(&graph, 11), 1.0)], TELEPORT, None)?;
    let (a_file, b_file) = (at(&graph, 1), at(&graph, 2));
    assert!(
        staged
            .activation
            .kept
            .iter()
            .all(|(node, _)| *node == a_file || *node == b_file),
        "{:?}",
        staged.activation.kept
    );
    assert!(near(staged.activation.of(a_file), 2.0 / 3.0));
    assert!(near(staged.activation.of(b_file), 1.0 / 3.0));
    let files: Vec<u32> = staged.scored.iter().map(|file| file.file).collect();
    assert_eq!(files, [a_file, b_file]);
    let symbols: Vec<Vec<u32>> = staged
        .scored
        .iter()
        .map(|file| file.symbols.iter().map(|(node, _)| *node).collect())
        .collect();
    assert_eq!(symbols, [vec![at(&graph, 11)], vec![at(&graph, 12)]]);
    let score = |file: usize| {
        staged
            .scored
            .get(file)
            .and_then(|file| file.symbols.first())
            .map_or(0.0, |(_, score)| *score)
    };
    assert!(near(score(0), 0.652_857_8), "{}", score(0));
    assert!(near(score(1), 0.303_654_8), "{}", score(1));
    Ok(())
}

/// Only the best files' symbols are spread over in the second stage: a hub file whose symbol calls
/// the symbols of 35 others reaches all 36 files in the first, the hub first and the others alike,
/// by node; the second spreads over the hub and the 29 files after it, whose symbols get their
/// share, and reaches no node of the last six files, which keep no symbol.
#[test]
fn the_second_stage_holds_to_the_best_files() -> Result<(), GraphError> {
    let files: Vec<i64> = (1..=36).collect();
    let symbols: Vec<(i64, i64)> = files.iter().map(|file| (100 + file, *file)).collect();
    let refs: Vec<(i64, i64)> = (2..=36).map(|file| (101, 100 + file)).collect();
    let graph = graph_of(&files, &symbols, &refs);
    let staged = two_stage(&graph, &[(at(&graph, 101), 1.0)], TELEPORT, None)?;
    assert_eq!(staged.scored.len(), 36);
    assert_eq!(
        staged.scored.first().map(|file| file.file),
        Some(at(&graph, 1))
    );
    let with_symbols = staged
        .scored
        .iter()
        .take_while(|file| !file.symbols.is_empty())
        .count();
    assert_eq!(with_symbols, STAGED_FILES);
    assert!(
        staged
            .scored
            .iter()
            .skip(STAGED_FILES)
            .all(|file| file.symbols.is_empty())
    );
    let last_staged: Vec<u32> = staged
        .scored
        .iter()
        .take(STAGED_FILES)
        .map(|file| file.file)
        .collect();
    let expected: Vec<u32> = (1..=30).map(|file| at(&graph, file)).collect();
    assert_eq!(last_staged, expected);
    let left_out: Vec<u32> = (31..=36)
        .flat_map(|file| [at(&graph, file), at(&graph, 100 + file)])
        .collect();
    assert!(
        staged
            .within
            .kept
            .iter()
            .all(|(node, _)| !left_out.contains(node)),
        "{:?}",
        staged.within.kept
    );
    assert!(staged.within.kept.len() >= 2 * STAGED_FILES);
    Ok(())
}

/// The graph's view by files is built once and kept: a graph is not prepared until it is, by
/// [`Graph::prepare_by_files`] or the first two-stage spreading, and a prepared graph spreads as an
/// unprepared one does and is equal to it.
#[test]
fn the_view_by_files_is_built_once_and_kept() -> Result<(), GraphError> {
    let graph = graph_of(
        &[1, 2, 3],
        &[(11, 1), (12, 2), (13, 3)],
        &[(11, 12), (13, 11)],
    );
    let seeds = [(at(&graph, 13), 1.0)];
    let prepared = graph.clone();
    assert!(!prepared.is_prepared_by_files());
    prepared.prepare_by_files()?;
    assert!(prepared.is_prepared_by_files());
    assert!(!graph.is_prepared_by_files());
    let unprepared_staged = two_stage(&graph, &seeds, TELEPORT, None)?;
    assert!(graph.is_prepared_by_files());
    assert_eq!(
        two_stage(&prepared, &seeds, TELEPORT, None)?,
        unprepared_staged
    );
    assert_eq!(
        prepared,
        graph_of(
            &[1, 2, 3],
            &[(11, 1), (12, 2), (13, 3)],
            &[(11, 12), (13, 11)]
        )
    );
    Ok(())
}
