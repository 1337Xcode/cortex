//! The seeds of one retrieval: what each source finds, and the one vector activation starts from.
//!
//! The sources are the prompt's words ([`lexical`]), what the session has been doing
//! ([`session`]), earlier work like the prompt's ([`similar`]), and, around what those found,
//! code agents have not worked on yet ([`exploration`]). Each weighs the nodes it finds on its own
//! scale. Per node the sources' weights add, and the whole is divided by its sum, so the vector
//! sums to 1. Where no source finds anything there is no vector: activation never starts from
//! nothing.

use std::collections::BTreeMap;

use cortex_core::Identifier;
use cortex_store::{NodeId, Queries, StoreError};

use crate::exploration::exploration;
use crate::graph::Graph;
use crate::lexical::{LexicalMatch, lexical};
use crate::prompt::PromptTerms;
use crate::session::{SessionMatch, session};
use crate::similar::{SimilarMatch, similar};

/// What a retrieval seeds from.
#[derive(Debug, Clone, Copy)]
pub struct SeedQuery<'a> {
    /// What the prompt points at.
    pub terms: &'a PromptTerms,
    /// The session the prompt comes in.
    pub session: &'a Identifier,
}

/// What each source found, and the vector they make.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seeds {
    /// What the prompt's words found.
    pub lexical: LexicalMatch,
    /// What the session has been doing.
    pub session: SessionMatch,
    /// Earlier work like the prompt's.
    pub similar: SimilarMatch,
    /// The dark neighbours of the strongest of the others.
    pub exploration: BTreeMap<u32, f64>,
    /// Every seed with its share of the whole, by node: the shares sum to 1, or there are none.
    pub vector: BTreeMap<u32, f64>,
}

/// The seeds `query` gives in the store `queries` reads, as nodes of `graph`.
///
/// # Errors
///
/// The store's errors.
pub fn seeds(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    query: &SeedQuery<'_>,
) -> Result<Seeds, StoreError> {
    let lexical = lexical(queries, graph, query.terms)?;
    let session = session(queries, graph, query.session)?;
    let similar = similar(queries, graph, query.terms)?;
    let found = summed(&[&lexical.seeds, &session.seeds, &similar.seeds]);
    let exploration = exploration(graph, &found);
    let vector = shares(&summed(&[&found, &exploration]));
    Ok(Seeds {
        lexical,
        session,
        similar,
        exploration,
        vector,
    })
}

/// Each node of `parts` with the sum of its weights in them.
#[must_use]
pub fn summed(parts: &[&BTreeMap<u32, f64>]) -> BTreeMap<u32, f64> {
    let mut whole: BTreeMap<u32, f64> = BTreeMap::new();
    for part in parts {
        for (node, weight) in *part {
            *whole.entry(*node).or_insert(0.0) += weight;
        }
    }
    whole
}

/// Each node of `weights` with its weight over the sum of all of them, those of no weight left
/// out: shares that sum to 1, or none when nothing weighs anything.
#[must_use]
pub fn shares(weights: &BTreeMap<u32, f64>) -> BTreeMap<u32, f64> {
    let total: f64 = weights.values().filter(|weight| **weight > 0.0).sum();
    if total <= 0.0 || !total.is_finite() {
        return BTreeMap::new();
    }
    weights
        .iter()
        .filter(|(_, weight)| **weight > 0.0)
        .map(|(node, weight)| (*node, weight / total))
        .collect()
}
