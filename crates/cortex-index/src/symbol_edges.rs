//! The graph's structural edges between definitions: from each definition to each definition its
//! references are tied to, as calls, uses of a type, inheritance or imports.
//!
//! An edge points one way, from the definition the references sit in. It is as strong as the
//! strongest reference behind it, by how that reference was tied, and its count is how many
//! references it stands for. A reference outside every definition, at the top of a file, gives no
//! edge: the file's own edges, to the files it imports, stand for it. A definition referring to
//! itself gives none either.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::EdgeType;
use cortex_store::{EdgeRecord, NodeId, Queries, StoreError, SymbolId, Target, Writer};

/// How many definitions have their edges written in one transaction: a commit costs little next to
/// this many definitions' edges, and the transaction stays short.
const SOURCES_PER_TRANSACTION: usize = 4096;

/// The nodes of definitions: every one read at once when every definition's edges are rewritten,
/// else each read when first needed.
struct Nodes<'q> {
    /// Where nodes are read from.
    queries: Queries<'q>,
    /// The nodes read so far; a definition without a node maps to `None`.
    known: BTreeMap<SymbolId, Option<NodeId>>,
    /// Whether every definition's node was read at once.
    is_complete: bool,
}

impl<'q> Nodes<'q> {
    /// The nodes `queries` reads, every one at once when `is_complete`.
    fn new(queries: Queries<'q>, is_complete: bool) -> Result<Self, StoreError> {
        let known = if is_complete {
            queries
                .symbol_nodes()?
                .into_iter()
                .map(|(symbol, node)| (symbol, Some(node)))
                .collect()
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            queries,
            known,
            is_complete,
        })
    }

    /// The node of `symbol`, if it has one.
    fn get(&mut self, symbol: SymbolId) -> Result<Option<NodeId>, StoreError> {
        if let Some(node) = self.known.get(&symbol) {
            return Ok(*node);
        }
        if self.is_complete {
            return Ok(None);
        }
        let node = self.queries.symbol_node(symbol)?;
        self.known.insert(symbol, node);
        Ok(node)
    }
}

/// Replaces the structural edges of every definition, or of those in `only`, with edges to the
/// definitions `ties`, each a reference's definition and target, point to; such a definition that
/// had edges and is in no tie now loses them. A definition without a node gets no edge.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn write_symbol_edges(
    writer: &mut Writer,
    ties: &[(SymbolId, Target)],
    only: Option<&BTreeSet<SymbolId>>,
) -> Result<(), StoreError> {
    let queries = writer.queries();
    let mut nodes = Nodes::new(queries, only.is_none())?;
    let mut edges: BTreeMap<NodeId, BTreeMap<NodeId, EdgeRecord>> = BTreeMap::new();
    for (source, target) in ties {
        if only.is_some_and(|only| !only.contains(source)) {
            continue;
        }
        let (Some(src), Some(dst)) = (nodes.get(*source)?, nodes.get(target.symbol)?) else {
            continue;
        };
        if src == dst {
            continue;
        }
        let weight = target.resolution.confidence();
        let edge = edges
            .entry(src)
            .or_default()
            .entry(dst)
            .or_insert(EdgeRecord {
                dst,
                weight,
                count: 0,
                // No task observed it: the code itself says so.
                last_seq: 0,
                is_directed: true,
            });
        edge.weight = edge.weight.max(weight);
        edge.count = edge.count.saturating_add(1);
    }
    // A definition to rewrite that is in no tie loses its edges: one of `only`, or else any that
    // has some now.
    if let Some(only) = only {
        for source in only {
            if let Some(node) = nodes.get(*source)? {
                edges.entry(node).or_default();
            }
        }
    } else {
        let definitions: BTreeSet<NodeId> = nodes.known.into_values().flatten().collect();
        for stale in queries.edge_sources(EdgeType::Structural)? {
            if definitions.contains(&stale) {
                edges.entry(stale).or_default();
            }
        }
    }
    let sources: Vec<(NodeId, Vec<EdgeRecord>)> = edges
        .into_iter()
        .map(|(src, targets)| (src, targets.into_values().collect()))
        .collect();
    for batch in sources.chunks(SOURCES_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        for (src, records) in batch {
            transaction.replace_edges(*src, EdgeType::Structural, records)?;
        }
        transaction.commit()?;
    }
    Ok(())
}
