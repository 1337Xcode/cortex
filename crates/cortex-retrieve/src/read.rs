//! The graph read from the store: every node and every link it holds, weighed for spreading, with
//! co-access links faded to the latest task, and every node's standing as of that task.

use cortex_core::{Config, FileFlags, TypeWeights};
use cortex_learn::{decayed, tier_now};
use cortex_store::{GraphNodeRow, NodeId, Queries};

use crate::error::RetrieveError;
use crate::graph::{Graph, LinkInput, NodeInput, Standing, Weighing};

/// What weighs the graph read from the store, from the settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphSettings {
    /// `retrieval.type_weights`.
    pub type_weights: TypeWeights,
    /// `learning.link_decay`: what a co-access link keeps of its weight for each task.
    pub link_decay: f64,
    /// `learning.interest_decay`: what a node keeps of its interest for each task.
    pub interest_decay: f64,
}

impl Default for GraphSettings {
    /// The settings' defaults.
    fn default() -> Self {
        let config = Config::default();
        Self {
            type_weights: config.retrieval.type_weights,
            link_decay: config.link_decay,
            interest_decay: config.interest_decay,
        }
    }
}

/// The graph of every node and link the store `queries` reads holds, weighed by `settings`: links
/// by kind, co-access links faded for each task since they last changed up to the latest, and each
/// node's interest brought forward to that task.
///
/// # Errors
///
/// The store's errors, and a graph too large to number.
pub fn read_graph(
    queries: Queries<'_>,
    settings: &GraphSettings,
) -> Result<Graph<NodeId>, RetrieveError> {
    let latest_seq = queries.latest_task_seq()?;
    let mut nodes: Vec<NodeInput<NodeId>> = Vec::new();
    queries.each_graph_node(|row| {
        nodes.push(NodeInput {
            id: row.node,
            kind: row.kind,
            file: row.file,
            standing: standing_of(&row, (latest_seq, settings.interest_decay)),
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
        type_weights: settings.type_weights,
        latest_seq,
        link_decay: settings.link_decay,
    };
    Ok(Graph::build(&nodes, &links, &weighing)?)
}

/// What `row` says of its node as of task `latest_seq`, interest kept by `interest_decay` for each
/// task since: a node with no interest is dark, and only a generated or vendored file is not
/// listed.
fn standing_of(row: &GraphNodeRow, (latest_seq, interest_decay): (u64, f64)) -> Standing {
    let is_listed = !row.flags.is_some_and(|flags| {
        flags.contains(FileFlags::GENERATED) || flags.contains(FileFlags::VENDORED)
    });
    let Some(interest) = &row.interest else {
        return Standing {
            is_listed,
            ..Standing::default()
        };
    };
    Standing {
        interest: decayed(
            interest.score,
            interest.last_seq,
            latest_seq,
            interest_decay,
        ),
        tier: tier_now(interest, (latest_seq, interest_decay)),
        is_listed,
    }
}
