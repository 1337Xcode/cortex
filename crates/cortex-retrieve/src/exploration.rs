//! The exploration bonus: code next to the strongest seeds that agents have not worked on yet,
//! given a little weight, so that retrieval does not only lead back to where agents already went.
//!
//! A neighbour is a node a structural link (a reference or an import, either way) joins to one of
//! the strongest seeds. It is dark when its standing as of the latest task puts it in no tier: it
//! is not a landmark and its interest is no more than the fog's, or it has none. Neighbours are
//! taken strongest seed first, then by the weight of the link, then by node.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, InterestTier};

use crate::graph::Graph;

/// What a dark neighbour weighs as a seed.
pub const EXPLORATION_WEIGHT: f64 = 0.05;

/// Most dark neighbours seeded.
pub const MAX_EXPLORED: usize = 20;

/// How many of the strongest seeds have their neighbours looked at.
const EXPLORED_FROM: usize = 10;

/// The dark structural neighbours of the strongest of `seeds` in `graph`, each weighed
/// [`EXPLORATION_WEIGHT`], at most [`MAX_EXPLORED`]; a seed is never its own or another's
/// neighbour.
#[must_use]
pub fn exploration<Id: Ord + Copy>(
    graph: &Graph<Id>,
    seeds: &BTreeMap<u32, f64>,
) -> BTreeMap<u32, f64> {
    let mut strongest: Vec<(u32, f64)> = seeds
        .iter()
        .map(|(node, weight)| (*node, *weight))
        .collect();
    strongest.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    let mut explored: BTreeMap<u32, f64> = BTreeMap::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for (seed, _) in strongest.into_iter().take(EXPLORED_FROM) {
        let mut links: Vec<(u32, f64)> = graph.links(seed, EdgeType::Structural).collect();
        links.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
        for (node, _) in links {
            if explored.len() >= MAX_EXPLORED {
                return explored;
            }
            if seeds.contains_key(&node) || !seen.insert(node) {
                continue;
            }
            let is_dark = graph
                .standing(node)
                .is_some_and(|standing| standing.tier == InterestTier::Dark);
            if is_dark {
                explored.insert(node, EXPLORATION_WEIGHT);
            }
        }
    }
    explored
}
