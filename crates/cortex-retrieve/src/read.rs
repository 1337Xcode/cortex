//! The graph read from the store: every node and every link it holds, weighed for spreading, with
//! co-access links faded to the latest task.

use cortex_core::TypeWeights;
use cortex_store::{NodeId, Queries};

use crate::error::RetrieveError;
use crate::graph::{Graph, LinkInput, NodeInput, Weighing};

/// The graph of every node and link the store `queries` reads holds, weighed by `type_weights`,
/// co-access links faded by `link_decay` for each task since they last changed up to the latest.
///
/// # Errors
///
/// The store's errors, and a graph too large to number.
pub fn read_graph(
    queries: Queries<'_>,
    type_weights: TypeWeights,
    link_decay: f64,
) -> Result<Graph<NodeId>, RetrieveError> {
    let mut nodes: Vec<NodeInput<NodeId>> = Vec::new();
    queries.each_graph_node(|row| {
        nodes.push(NodeInput {
            id: row.node,
            kind: row.kind,
            file: row.file,
        });
        Ok(())
    })?;
    let mut links: Vec<LinkInput<NodeId>> = Vec::new();
    queries.each_edge(|row| {
        links.push(LinkInput {
            src: row.src,
            dst: row.edge.dst,
            kind: row.edge_type,
            weight: row.edge.weight,
            last_seq: u64::try_from(row.edge.last_seq).unwrap_or(0),
        });
        Ok(())
    })?;
    let weighing = Weighing {
        type_weights,
        latest_seq: queries.latest_task_seq()?,
        link_decay,
    };
    Ok(Graph::build(&nodes, &links, &weighing)?)
}
