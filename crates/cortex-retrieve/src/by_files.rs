//! The graph seen by files, which the first stage of a huge repository's spreading runs over: each
//! file's links and those of its symbols, each taken to the file it lands in, those within the file
//! left out, the weights landing in one file added. A node that belongs to no file keeps its own
//! links, taken the same way; a symbol of a file has none, since what it links to belongs to its
//! file.
//!
//! It is built from the graph once and kept with it ([`ByFiles`]): on first use, or before, when
//! the daemon prepares it after rebuilding a huge repository's graph, so that no prompt waits for
//! it. Merging a file's links again on every push of the first stage made that stage five times as
//! slow as spreading over the whole graph.

use std::sync::OnceLock;

use cortex_core::EdgeType;

use crate::graph::{Graph, GraphError, Rows};
use crate::push::Network;

/// The graph seen by files.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileView {
    /// Each node's links to other files, by target: one set of rows over every kind of link.
    rows: Rows,
    /// The sum of each node's link weights here.
    out_weights: Vec<f64>,
}

impl FileView {
    /// The bytes this view holds: its rows and the weight of each node.
    pub(crate) fn size_bytes(&self) -> usize {
        self.rows.size_bytes() + crate::graph::size_of_vec(&self.out_weights)
    }

    /// The view of `graph` by files.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`] for more links between files than a 32-bit offset reaches.
    pub(crate) fn of<Id: Ord + Copy>(graph: &Graph<Id>) -> Result<Self, GraphError> {
        let nodes =
            u32::try_from(graph.len()).map_err(|_| GraphError::TooLarge { what: "nodes" })?;
        let mut links: Vec<(u32, u32, f64)> = Vec::new();
        for node in 0..nodes {
            if graph.file_of(node).is_some_and(|file| file != node) {
                continue;
            }
            for member in std::iter::once(node).chain(graph.symbols_of(node).iter().copied()) {
                for kind in EdgeType::ALL {
                    for (target, weight) in graph.links(member, *kind) {
                        let lands_in = graph.file_of(target).unwrap_or(target);
                        if lands_in != node {
                            links.push((node, lands_in, weight));
                        }
                    }
                }
            }
        }
        let rows = Rows::of(graph.len(), links)?;
        let out_weights = (0..nodes)
            .map(|node| rows.of_node(node).map(|(_, weight)| weight).sum())
            .collect();
        Ok(Self { rows, out_weights })
    }
}

impl Network for FileView {
    fn node_count(&self) -> usize {
        self.out_weights.len()
    }

    fn out_weight(&self, node: u32) -> f64 {
        self.out_weights
            .get(usize::try_from(node).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0.0)
    }

    fn each_link(&self, node: u32, mut visit: impl FnMut(u32, f64)) {
        for (target, weight) in self.rows.of_node(node) {
            visit(target, weight);
        }
    }
}

/// A graph's view by files, once built, kept beside the graph it comes from. It is derived from the
/// graph, so it takes no part in whether two graphs are equal.
#[derive(Debug, Clone, Default)]
pub(crate) struct ByFiles(OnceLock<Result<FileView, GraphError>>);

impl ByFiles {
    /// The bytes the view holds, or none when nothing has asked for it yet.
    pub(crate) fn size_bytes(&self) -> usize {
        match self.0.get() {
            Some(Ok(view)) => view.size_bytes(),
            _ => 0,
        }
    }

    /// The view of `graph`, which this one keeps, built now if it was not yet.
    ///
    /// # Errors
    ///
    /// [`GraphError::TooLarge`], as [`FileView::of`] gives it.
    pub(crate) fn of<'a, Id: Ord + Copy>(
        &'a self,
        graph: &Graph<Id>,
    ) -> Result<&'a FileView, GraphError> {
        self.0
            .get_or_init(|| FileView::of(graph))
            .as_ref()
            .map_err(|error| *error)
    }

    /// Whether the view was built.
    pub(crate) fn is_built(&self) -> bool {
        self.0.get().is_some()
    }
}

impl PartialEq for ByFiles {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
