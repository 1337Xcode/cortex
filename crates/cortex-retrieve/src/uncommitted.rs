//! What the work tree is changing and has not committed: the files of the uncommitted diff, which a
//! prompt is likely about, each seeded at [`UNCOMMITTED_WEIGHT`] on the file whatever the session.
//! The daemon reads them in the background with `git status` and keeps their file nodes; a
//! retrieval takes them as they stand.

use std::collections::{BTreeMap, BTreeSet};

use cortex_store::NodeId;

use crate::graph::Graph;

/// A seed's weight for a file of the uncommitted diff.
pub const UNCOMMITTED_WEIGHT: f64 = 0.8;

/// What the uncommitted diff seeds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UncommittedMatch {
    /// Each file it seeds, with its weight, by node.
    pub seeds: BTreeMap<u32, f64>,
    /// The same files.
    pub files: BTreeSet<u32>,
}

/// The files `nodes`, the uncommitted diff's file nodes, as nodes of `graph`, each seeded
/// [`UNCOMMITTED_WEIGHT`]; a node the graph does not hold is left out.
#[must_use]
pub fn uncommitted(graph: &Graph<NodeId>, nodes: &[NodeId]) -> UncommittedMatch {
    let files: BTreeSet<u32> = nodes
        .iter()
        .filter_map(|node| graph.index_of(*node))
        .collect();
    UncommittedMatch {
        seeds: files
            .iter()
            .map(|file| (*file, UNCOMMITTED_WEIGHT))
            .collect(),
        files,
    }
}
