//! Spreading activation: an approximate personalized `PageRank` from the seeds, computed by the
//! local push of Andersen, Chung and Lang.
//!
//! Each node holds activation it keeps and activation it has still to spread. Pushing a node keeps
//! the restart share (`retrieval.teleport`) of what it has to spread and hands the rest to its
//! links in proportion to their weights; what a node with no links would hand on is dropped. A
//! node is pushed while it has at least the threshold to spread, first [`SPREAD_THRESHOLD`], in the
//! order nodes came to have that much, so the outcome depends only on the graph and the seeds.
//!
//! The activation a node keeps is never more than its exact personalized `PageRank`, and the two
//! differ in all by at most what is left to spread. When no node has the threshold left but more
//! than [`L1_TOLERANCE`] is left in all, spread thinly over many nodes, the threshold is lowered
//! tenfold and pushing goes on, so that a spreading that ends on its own is within that tolerance
//! of the exact values. Pushing also ends after [`MAX_PUSHES`] pushes or at the deadline, cut
//! short, with what was left reported.
//!
//! Links are normalized per node as they are read: a node hands on what it spreads divided by the
//! sum of its links' weights, times each link's weight. The work depends on the graph around the
//! seeds, not on the graph's size, and the push cap and the deadline, looked at every 256 pushes,
//! bound it whatever that graph is.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use cortex_core::EdgeType;

use crate::graph::Graph;
use crate::node_hash::NodeHashing;

/// The first threshold: the least a node must have to spread for it to be pushed. Below it, what a
/// node holds is too little to change which nodes rank highest.
pub const SPREAD_THRESHOLD: f64 = 1e-5;

/// The most that may be left to spread, in all, when a spreading ends on its own: how far, summed
/// over every node, its activation may fall short of the exact values.
pub const L1_TOLERANCE: f64 = 1e-3;

/// The lowest the threshold is taken to: what is left is then too little to matter however many
/// nodes share it, and lowering stops.
const LOWEST_THRESHOLD: f64 = 1e-12;

/// The most pushes one spreading takes, whatever is left: it keeps the work bounded on a node whose
/// links fan out to thousands of others.
pub const MAX_PUSHES: u32 = 50_000;

/// How many pushes go between two looks at the deadline: looking costs a clock read.
const PUSHES_PER_CLOCK_READ: u32 = 256;

/// What activation spreads over: nodes numbered from 0, each with weighed links to others.
pub trait Network {
    /// How many nodes it numbers.
    fn node_count(&self) -> usize;
    /// The sum of the weights of node `node`'s links: what it spreads over.
    fn out_weight(&self, node: u32) -> f64;
    /// Calls `visit` with each link of node `node`: its target and its weight.
    fn each_link(&self, node: u32, visit: impl FnMut(u32, f64));
}

impl<Id: Ord + Copy> Network for Graph<Id> {
    fn node_count(&self) -> usize {
        self.len()
    }

    fn out_weight(&self, node: u32) -> f64 {
        Graph::out_weight(self, node)
    }

    fn each_link(&self, node: u32, mut visit: impl FnMut(u32, f64)) {
        for kind in EdgeType::ALL {
            for (target, weight) in self.links(node, *kind) {
                visit(target, weight);
            }
        }
    }
}

/// What spreading activation found.
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    /// Each node that kept any activation, with how much, by node.
    pub kept: Vec<(u32, f64)>,
    /// How many pushes it took.
    pub pushes: u32,
    /// How much was left to spread when it stopped: what the kept activation falls short of the
    /// exact values by, in all, at most.
    pub left: f64,
    /// Whether it stopped at [`MAX_PUSHES`] or the deadline with something still to push.
    pub is_cut_short: bool,
    /// Whether the deadline stopped it with something still to push: what it kept then hangs on
    /// how fast the machine was, unlike a stop at [`MAX_PUSHES`], which the inputs decide.
    pub is_out_of_time: bool,
}

impl Activation {
    /// The activation node `node` kept, 0 for a node that kept none.
    #[must_use]
    pub fn of(&self, node: u32) -> f64 {
        self.kept
            .binary_search_by_key(&node, |(at, _)| *at)
            .ok()
            .and_then(|at| self.kept.get(at))
            .map_or(0.0, |(_, kept)| *kept)
    }
}

/// What one node holds while activation spreads.
#[derive(Debug, Clone, Copy, Default)]
struct Held {
    /// What it keeps.
    kept: f64,
    /// What it has still to spread.
    to_spread: f64,
    /// Whether it waits in the queue to be pushed.
    is_queued: bool,
}

/// A spreading under way: what each node holds, the nodes waiting to be pushed, and the threshold.
struct Spreading<'a, N> {
    /// What is spread over.
    network: &'a N,
    /// What each node touched holds.
    held: HashMap<u32, Held, NodeHashing>,
    /// The nodes waiting to be pushed, in the order they came to have the threshold.
    queue: VecDeque<u32>,
    /// What a node must have to spread to be pushed.
    threshold: f64,
    /// The restart probability.
    teleport: f64,
    /// How much is left to spread, in all, kept up as pushes take and hand on.
    left: f64,
    /// How many pushes it has taken.
    pushes: u32,
}

impl<N: Network> Spreading<'_, N> {
    /// Adds `share` to what `node` has to spread, queueing it once it has the threshold.
    fn give(&mut self, node: u32, share: f64) {
        let node_held = self.held.entry(node).or_default();
        node_held.to_spread += share;
        if node_held.to_spread >= self.threshold && !node_held.is_queued {
            node_held.is_queued = true;
            self.queue.push_back(node);
        }
    }

    /// Pushes `node`: it keeps the restart share of what it has to spread and hands the rest to its
    /// links, or drops it when it has none.
    fn push(&mut self, node: u32) {
        self.pushes += 1;
        let to_spread = {
            let node_held = self.held.entry(node).or_default();
            node_held.is_queued = false;
            let to_spread = std::mem::take(&mut node_held.to_spread);
            node_held.kept += self.teleport * to_spread;
            to_spread
        };
        let total = self.network.out_weight(node);
        if total <= 0.0 {
            self.left -= to_spread;
            return;
        }
        self.left -= self.teleport * to_spread;
        let share = (1.0 - self.teleport) * to_spread / total;
        // The network is a reference of its own, so its links are read while `self` takes them.
        let network = self.network;
        network.each_link(node, |target, weight| self.give(target, share * weight));
    }

    /// Lowers the threshold tenfold and queues, in node order, every node that now has it.
    fn lower_threshold(&mut self) {
        self.threshold /= 10.0;
        let mut nodes: Vec<u32> = self
            .held
            .iter()
            .filter(|(_, node_held)| node_held.to_spread >= self.threshold)
            .map(|(node, _)| *node)
            .collect();
        nodes.sort_unstable();
        for node in nodes {
            if let Some(node_held) = self.held.get_mut(&node) {
                node_held.is_queued = true;
                self.queue.push_back(node);
            }
        }
    }
}

/// Spreads activation over `network` from `seeds`, each a node and its share of the start, with
/// the restart probability `teleport`, stopping at `deadline` if it comes first. Shares of one node
/// add up; a share for a node the network does not number, not above zero or not finite, is left
/// out.
#[must_use]
pub fn spread<N: Network>(
    network: &N,
    seeds: &[(u32, f64)],
    teleport: f64,
    deadline: Option<Instant>,
) -> Activation {
    let mut start: Vec<(u32, f64)> = seeds
        .iter()
        .copied()
        .filter(|(node, share)| {
            usize::try_from(*node).is_ok_and(|at| at < network.node_count())
                && share.is_finite()
                && *share > 0.0
        })
        .collect();
    // INVARIANT: deterministic (the queue starts in node order, whatever order the seeds came in)
    start.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.total_cmp(&right.1)));
    let mut spreading = Spreading {
        network,
        held: HashMap::default(),
        queue: VecDeque::new(),
        threshold: SPREAD_THRESHOLD,
        teleport,
        left: start.iter().map(|(_, share)| share).sum(),
        pushes: 0,
    };
    for (node, share) in start {
        spreading.give(node, share);
    }
    let mut is_cut_short = false;
    let mut is_out_of_time = false;
    loop {
        while let Some(node) = spreading.queue.pop_front() {
            let is_late = spreading.pushes.is_multiple_of(PUSHES_PER_CLOCK_READ)
                && deadline.is_some_and(|deadline| Instant::now() >= deadline);
            // INVARIANT: budgeted (spreading stops at its step cap or its deadline, so a prompt's
            // hook keeps its latency budget whatever the graph around the seeds)
            if spreading.pushes >= MAX_PUSHES || is_late {
                is_cut_short = true;
                is_out_of_time = is_late;
                break;
            }
            spreading.push(node);
        }
        if is_cut_short || spreading.left <= L1_TOLERANCE || spreading.threshold <= LOWEST_THRESHOLD
        {
            break;
        }
        spreading.lower_threshold();
    }
    let mut nodes: Vec<(u32, Held)> = spreading.held.into_iter().collect();
    nodes.sort_by_key(|(node, _)| *node);
    let left = nodes.iter().map(|(_, node_held)| node_held.to_spread).sum();
    Activation {
        kept: nodes
            .into_iter()
            .filter(|(_, node_held)| node_held.kept > 0.0)
            .map(|(node, node_held)| (node, node_held.kept))
            .collect(),
        pushes: spreading.pushes,
        left,
        is_cut_short,
        is_out_of_time,
    }
}
