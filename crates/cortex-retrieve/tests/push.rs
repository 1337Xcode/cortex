//! Local push against exact personalized PageRank, computed by power iteration over a petgraph
//! copy of the same graph, on random graphs and on a graph worked out by hand.

use cortex_core::{EdgeType, NodeKind, TypeWeights};
use cortex_retrieve::{
    Activation, Graph, L1_TOLERANCE, LinkInput, MAX_PUSHES, NodeInput, SPREAD_THRESHOLD, Standing,
    Weighing, spread,
};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use proptest::prelude::*;

/// The restart probability the settings default to.
const TELEPORT: f64 = 0.5;

/// The settings' weights, nothing faded.
fn weighing() -> Weighing {
    Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    }
}

/// A graph of `count` files, numbered from 1, with `rows` as its links: each a source, a target,
/// a kind's number and a stored weight. No graph this small is too large to build.
fn graph_of(count: i64, rows: &[(i64, i64, u8, f64)]) -> Graph<i64> {
    let nodes: Vec<NodeInput<i64>> = (1..=count)
        .map(|id| NodeInput {
            id,
            kind: NodeKind::File,
            file: None,
            standing: Standing::default(),
        })
        .collect();
    let links: Vec<LinkInput<i64>> = rows
        .iter()
        .filter_map(|(src, dst, kind, weight)| {
            Some(LinkInput {
                src: *src,
                dst: *dst,
                kind: EdgeType::from_number(*kind)?,
                weight: *weight,
                last_seq: 0,
            })
        })
        .collect();
    #[expect(
        clippy::expect_used,
        reason = "a test graph of a few thousand nodes always builds"
    )]
    Graph::build(&nodes, &links, &weighing()).expect("a test graph builds")
}

/// Exact personalized PageRank from `seeds` over `graph`, by power iteration on a petgraph copy
/// whose edges carry each link's weight over its node's out-weight: p = a s + (1 - a) p W from
/// p = a s, until an iteration moves no node by more than 1e-15 in all. A node with no links passes
/// nothing on, as the push drops what such a node would pass.
fn exact(graph: &Graph<i64>, seeds: &[(u32, f64)], teleport: f64) -> Vec<f64> {
    let mut copy: DiGraph<(), f64> = DiGraph::new();
    let nodes: Vec<NodeIndex> = (0..graph.len()).map(|_| copy.add_node(())).collect();
    for (from, source) in nodes.iter().enumerate() {
        let node = u32::try_from(from).unwrap_or(u32::MAX);
        let total = graph.out_weight(node);
        if total <= 0.0 {
            continue;
        }
        for kind in EdgeType::ALL {
            for (target, weight) in graph.links(node, *kind) {
                if let Some(target) = usize::try_from(target).ok().and_then(|at| nodes.get(at)) {
                    copy.add_edge(*source, *target, weight / total);
                }
            }
        }
    }
    let mut start = vec![0.0; graph.len()];
    for (node, share) in seeds {
        if let Some(slot) = usize::try_from(*node).ok().and_then(|at| start.get_mut(at)) {
            *slot += share;
        }
    }
    let mut rank: Vec<f64> = start.iter().map(|share| teleport * share).collect();
    for _ in 0..10_000 {
        let mut next: Vec<f64> = start.iter().map(|share| teleport * share).collect();
        for edge in copy.edge_references() {
            let from = rank.get(edge.source().index()).copied().unwrap_or(0.0);
            if let Some(slot) = next.get_mut(edge.target().index()) {
                *slot += (1.0 - teleport) * from * edge.weight();
            }
        }
        let moved: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if moved < 1e-15 {
            break;
        }
    }
    rank
}

/// The activation the push kept, one value per node.
fn kept(activation: &Activation, nodes: usize) -> Vec<f64> {
    (0..nodes)
        .map(|node| activation.of(u32::try_from(node).unwrap_or(u32::MAX)))
        .collect()
}

/// A chain worked out by hand: A links to B and B back to A, both structurally at confidence 1,
/// and the seed is A. Each node passes everything on to the other, so exact PageRank with restart
/// a = 0.5 solves p(A) = a + (1 - a) p(B) and p(B) = (1 - a) p(A): p(A) = 0.5 / 0.75 = 2/3 and
/// p(B) = 1/3. The push gets within its threshold of both.
#[test]
fn a_pair_spreads_as_worked_out() {
    let graph = graph_of(2, &[(1, 2, 0, 1.0)]);
    let activation = spread(&graph, &[(0, 1.0)], TELEPORT, None);
    assert!(
        (activation.of(0) - 2.0 / 3.0).abs() < 1e-4,
        "{activation:?}"
    );
    assert!(
        (activation.of(1) - 1.0 / 3.0).abs() < 1e-4,
        "{activation:?}"
    );
    assert!(!activation.is_cut_short);
    assert!(activation.left < SPREAD_THRESHOLD * 2.0);
}

/// A node with no links keeps its restart share of what reaches it and passes on nothing: a seed
/// alone keeps 0.5 of its share, and nothing more is left to spread.
#[test]
fn a_node_without_links_keeps_its_share_and_drops_the_rest() {
    let graph = graph_of(1, &[]);
    let activation = spread(&graph, &[(0, 1.0)], TELEPORT, None);
    assert_eq!(activation.kept, [(0, 0.5)]);
    assert_eq!(activation.pushes, 1);
    assert_eq!(activation.left, 0.0);
}

/// Seeds the graph does not hold, and shares of zero or below, are left out; with nothing to
/// start from, nothing spreads.
#[test]
fn strange_seeds_are_left_out() {
    let graph = graph_of(2, &[(1, 2, 0, 1.0)]);
    let activation = spread(&graph, &[(7, 1.0), (1, 0.0), (0, -1.0)], TELEPORT, None);
    assert_eq!(activation.kept, []);
    assert_eq!(activation.pushes, 0);
}

/// 60,000 nodes in linked pairs, each a seed with an equal share of the start, 1 / 60,000, above
/// the threshold: every one waits to be pushed, so pushing stops at the cap of 50,000, says it was
/// cut short, and keeps within its bounds all the same: no node above its exact value, the
/// shortfall within what is left to spread (the 10,000 nodes never pushed hold theirs). A deadline
/// already passed stops it before its first push.
#[test]
fn spreading_stops_at_its_cap_and_its_deadline() {
    let pairs: Vec<(i64, i64, u8, f64)> = (1..=30_000)
        .map(|pair| (2 * pair - 1, 2 * pair, 0, 1.0))
        .collect();
    let graph = graph_of(60_000, &pairs);
    let share = 1.0 / 60_000.0;
    let seeds: Vec<(u32, f64)> = (0..60_000).map(|node| (node, share)).collect();
    let activation = spread(&graph, &seeds, TELEPORT, None);
    assert_eq!(activation.pushes, MAX_PUSHES);
    assert!(activation.is_cut_short);
    assert!(!activation.is_out_of_time);
    let truth = exact(&graph, &seeds, TELEPORT);
    let found = kept(&activation, graph.len());
    let short: f64 = truth.iter().zip(&found).map(|(t, f)| t - f).sum();
    assert!(found.iter().zip(&truth).all(|(f, t)| *f <= t + 1e-12));
    assert!(
        short <= activation.left + 1e-9,
        "{short} > {}",
        activation.left
    );
    let passed = std::time::Instant::now();
    let late = spread(&graph, &seeds, TELEPORT, Some(passed));
    assert_eq!(late.pushes, 0);
    assert!(late.is_cut_short);
    assert!(late.is_out_of_time);
}

/// Worked by hand: a file with co-change links to 200 others, which link to nothing (co-change
/// runs one way), given a start of 0.0036. It keeps 0.5 x 0.0036 = 0.0018 and hands each of the
/// 200 its share of the rest by weight, 0.0018 / 200 = 9e-6, below the first threshold of 1e-5; the
/// 200 then hold 0.0018 in all, more than the tolerance of 1e-3, so the threshold is lowered to
/// 1e-6 and each is pushed: it keeps 4.5e-6 and, with no links, drops the rest. Nothing is left
/// after 201 pushes, and every node holds its exact value.
#[test]
fn a_thin_spread_is_pushed_past_its_first_threshold() {
    let rows: Vec<(i64, i64, u8, f64)> = (2..=201).map(|leaf| (1, leaf, 1, 1.0)).collect();
    let graph = graph_of(201, &rows);
    let activation = spread(&graph, &[(0, 0.0036)], TELEPORT, None);
    assert!(!activation.is_cut_short);
    assert_eq!(activation.pushes, 201);
    assert!(activation.left.abs() < 1e-15, "{} left", activation.left);
    assert!((activation.of(0) - 0.0018).abs() < 1e-15);
    for leaf in 1..=200 {
        assert!((activation.of(leaf) - 4.5e-6).abs() < 1e-15, "{leaf}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// On random graphs of up to 300 nodes, with links of every kind and random weights, from up
    /// to five seeds sharing a start of 1: the push keeps no node above its exact value, falls
    /// short of the exact values by at most what it left to spread, ends on its own, and is within
    /// 1e-3 of them in all (L1), float rounding aside. The seeds in another order give the very
    /// same result.
    #[test]
    fn push_is_within_its_bound_of_exact_pagerank(
        count in 2..300_i64,
        rows in proptest::collection::vec((1..300_i64, 1..300_i64, 0..5_u8, 0.0..1.0_f64), 0..900),
        picks in proptest::collection::vec((0..300_u32, 0.01..1.0_f64), 1..5),
    ) {
        let within = |id: i64| 1 + (id - 1) % count;
        let rows: Vec<(i64, i64, u8, f64)> = rows
            .into_iter()
            .map(|(src, dst, kind, weight)| (within(src), within(dst), kind, weight))
            .collect();
        let graph = graph_of(count, &rows);
        let nodes = u32::try_from(count).unwrap_or(1);
        let total: f64 = picks.iter().map(|(_, share)| share).sum();
        let seeds: Vec<(u32, f64)> = picks
            .iter()
            .map(|(node, share)| (node % nodes, share / total))
            .collect();
        let activation = spread(&graph, &seeds, TELEPORT, None);
        let truth = exact(&graph, &seeds, TELEPORT);
        let found = kept(&activation, graph.len());
        for (f, t) in found.iter().zip(&truth) {
            prop_assert!(*f <= t + 1e-12, "{f} above {t}");
        }
        let distance: f64 = truth.iter().zip(&found).map(|(t, f)| (t - f).abs()).sum();
        prop_assert!(distance <= activation.left + 1e-9, "{distance} > {}", activation.left);
        prop_assert!(!activation.is_cut_short);
        prop_assert!(distance <= L1_TOLERANCE + 1e-9, "L1 {distance}");
        let mut reversed = seeds.clone();
        reversed.reverse();
        prop_assert_eq!(&activation, &spread(&graph, &reversed, TELEPORT, None));
    }
}
