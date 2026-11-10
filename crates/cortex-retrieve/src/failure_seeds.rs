//! What the session's last failed check seeds: the definitions (or files) its stack ran through in
//! the repository, as the failure map grounded them. A frame another source also points at (the
//! prompt's words, the session's work, the uncommitted files, similar tasks), at itself or at its
//! file, seeds as strongly as an edit: the prompt is about that failure. A frame no other source
//! points at is one signal among the rest, seeded as a read is.

use std::collections::{BTreeMap, BTreeSet};

use cortex_store::NodeId;

use crate::graph::Graph;

/// A seed's weight for a frame of the last failure that another source points at too.
pub const FAILURE_AGREED_WEIGHT: f64 = 1.0;

/// A seed's weight for a frame of the last failure that no other source points at.
pub const FAILURE_ALONE_WEIGHT: f64 = 0.4;

/// What the last failure seeds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FailureMatch {
    /// Each node it seeds, with its weight, by node.
    pub seeds: BTreeMap<u32, f64>,
    /// How many of the nodes it seeds another source points at.
    pub agreed: usize,
}

/// The seeds of `frames`, the nodes of the last failure's grounded frames (each a definition, or a
/// file where no definition holds the line), as nodes of `graph`, judged against `others`, what
/// every other source seeds. A frame the graph does not hold seeds nothing; a node named twice
/// seeds once.
#[must_use]
pub fn failure_seeds(
    graph: &Graph<NodeId>,
    frames: &[NodeId],
    others: &BTreeMap<u32, f64>,
) -> FailureMatch {
    let other_files: BTreeSet<u32> = others
        .keys()
        .filter_map(|node| graph.file_of(*node))
        .collect();
    let mut found = FailureMatch::default();
    for node in frames {
        let Some(index) = graph.index_of(*node) else {
            continue;
        };
        if found.seeds.contains_key(&index) {
            continue;
        }
        let is_agreed = others.contains_key(&index)
            || graph
                .file_of(index)
                .is_some_and(|file| other_files.contains(&file));
        let weight = if is_agreed {
            found.agreed += 1;
            FAILURE_AGREED_WEIGHT
        } else {
            FAILURE_ALONE_WEIGHT
        };
        found.seeds.insert(index, weight);
    }
    found
}
