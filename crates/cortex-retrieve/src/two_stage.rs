//! Spreading in two stages, as huge repositories need: first over files, each file standing for
//! itself and its symbols, then over the symbols of the best files alone.
//!
//! In the first stage a file's links are those of the file and its symbols to other files, each
//! taken to the file it lands in, the weights of links that land in one file added; a seed on a
//! symbol starts at its file. The files score as in one stage. The second stage spreads from the
//! seeds again, over the nodes of the [`STAGED_FILES`] best files and the links among them alone,
//! and gives those files' symbols their activation, from which their spans are chosen. A seed
//! outside those files starts nothing there.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use cortex_core::EdgeType;

use crate::graph::Graph;
use crate::push::{Activation, Network, spread};
use crate::score::{ScoredFile, scored_files, with_symbols};

/// How many of the best files of the first stage the second spreads over.
pub const STAGED_FILES: usize = 30;

/// `graph` as the first stage sees it: its files, each with the links of the file and its symbols
/// to other files.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FilesOf<'a, Id> {
    /// The graph.
    pub(crate) graph: &'a Graph<Id>,
}

impl<Id: Ord + Copy> FilesOf<'_, Id> {
    /// The links of file `file` and its symbols, each taken to the file it lands in, those within
    /// `file` left out, the weights landing in one file added, by file.
    fn file_links(self, file: u32) -> BTreeMap<u32, f64> {
        let mut found: BTreeMap<u32, f64> = BTreeMap::new();
        for node in std::iter::once(file).chain(self.graph.symbols_of(file).iter().copied()) {
            for kind in EdgeType::ALL {
                for (target, weight) in self.graph.links(node, *kind) {
                    let lands_in = self.graph.file_of(target).unwrap_or(target);
                    if lands_in != file {
                        *found.entry(lands_in).or_insert(0.0) += weight;
                    }
                }
            }
        }
        found
    }
}

impl<Id: Ord + Copy> Network for FilesOf<'_, Id> {
    fn node_count(&self) -> usize {
        self.graph.len()
    }

    fn out_weight(&self, node: u32) -> f64 {
        self.file_links(node).values().sum()
    }

    fn each_link(&self, node: u32, mut visit: impl FnMut(u32, f64)) {
        for (target, weight) in self.file_links(node) {
            visit(target, weight);
        }
    }
}

/// `graph` as the second stage sees it: the nodes of `members` and the links among them alone.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Within<'a, Id> {
    /// The graph.
    pub(crate) graph: &'a Graph<Id>,
    /// The nodes spread over.
    pub(crate) members: &'a BTreeSet<u32>,
}

impl<Id: Ord + Copy> Network for Within<'_, Id> {
    fn node_count(&self) -> usize {
        self.graph.len()
    }

    fn out_weight(&self, node: u32) -> f64 {
        let mut total = 0.0;
        self.each_link(node, |_, weight| total += weight);
        total
    }

    fn each_link(&self, node: u32, mut visit: impl FnMut(u32, f64)) {
        if !self.members.contains(&node) {
            return;
        }
        for kind in EdgeType::ALL {
            for (target, weight) in self.graph.links(node, *kind) {
                if self.members.contains(&target) {
                    visit(target, weight);
                }
            }
        }
    }
}

/// What two stages of spreading found: each stage's activation, and the files the first reached,
/// scored, the best first, the best [`STAGED_FILES`] with their symbols' scores from the second.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    /// The first stage's activation, over files.
    pub activation: Activation,
    /// The second stage's, over the nodes of the best files.
    pub within: Activation,
    /// The files reached, scored.
    pub scored: Vec<ScoredFile>,
}

/// Spreads over `graph` from `seeds` in two stages, with the restart probability `teleport`, each
/// stage stopping at `deadline` if it comes first.
#[must_use]
pub fn two_stage<Id: Ord + Copy>(
    graph: &Graph<Id>,
    seeds: &[(u32, f64)],
    teleport: f64,
    deadline: Option<Instant>,
) -> Staged {
    let mut at_files: BTreeMap<u32, f64> = BTreeMap::new();
    for (node, share) in seeds {
        let file = graph.file_of(*node).unwrap_or(*node);
        *at_files.entry(file).or_insert(0.0) += share;
    }
    let file_seeds: Vec<(u32, f64)> = at_files.into_iter().collect();
    let activation = spread(&FilesOf { graph }, &file_seeds, teleport, deadline);
    let scored = scored_files(graph, &activation);
    let members: BTreeSet<u32> = scored
        .iter()
        .take(STAGED_FILES)
        .flat_map(|file| {
            std::iter::once(file.file).chain(graph.symbols_of(file.file).iter().copied())
        })
        .collect();
    let within = spread(
        &Within {
            graph,
            members: &members,
        },
        seeds,
        teleport,
        deadline,
    );
    Staged {
        scored: with_symbols(graph, scored, &within, STAGED_FILES),
        activation,
        within,
    }
}
