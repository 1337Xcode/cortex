//! The seeds of one retrieval: what each source finds, and the one vector activation starts from.
//!
//! The sources are the prompt's words ([`lexical`]), what the session has been doing
//! ([`session`]), what the work tree is changing and has not committed ([`uncommitted`]), earlier
//! work like the prompt's ([`similar`]), the code the session's last failed check ran through
//! ([`failure_seeds`], weighed by whether the others agree), and, around what those found, code
//! agents have not worked on yet ([`exploration`]). Each weighs the nodes it finds on its own
//! scale. Per node the sources' weights add, and the whole is divided by its sum, so the vector
//! sums to 1. Where no source finds anything there is no vector: activation never starts from
//! nothing.

use std::collections::BTreeMap;

use cortex_core::Identifier;
use cortex_store::{NodeId, Queries, StoreError};

use crate::exploration::exploration;
use crate::failure_seeds::{FailureMatch, failure_seeds};
use crate::graph::Graph;
use crate::lexical::{LexicalMatch, lexical};
use crate::prompt::PromptTerms;
use crate::session::{SessionMatch, session};
use crate::similar::{SimilarMatch, similar};
use crate::uncommitted::{UncommittedMatch, uncommitted};

/// What a retrieval seeds from.
#[derive(Debug, Clone, Copy)]
pub struct SeedQuery<'a> {
    /// What the prompt points at.
    pub terms: &'a PromptTerms,
    /// The session the prompt comes in; none for a question asked outside one, which seeds
    /// nothing from a session.
    pub session: Option<&'a Identifier>,
    /// The file nodes of the uncommitted diff, as the daemon last read them; empty when it has not.
    pub uncommitted: &'a [NodeId],
    /// The nodes the session's last failed check ran through, as its failure map grounded them;
    /// empty when there is none.
    pub failure: &'a [NodeId],
}

/// What each source found, and the vector they make.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seeds {
    /// What the prompt's words found.
    pub lexical: LexicalMatch,
    /// What the session has been doing.
    pub session: SessionMatch,
    /// What the work tree is changing and has not committed.
    pub uncommitted: UncommittedMatch,
    /// Earlier work like the prompt's.
    pub similar: SimilarMatch,
    /// The code the session's last failed check ran through.
    pub failure: FailureMatch,
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
    let session = match query.session {
        Some(id) => session(queries, graph, id)?,
        None => SessionMatch::default(),
    };
    let uncommitted = uncommitted(graph, query.uncommitted);
    let similar = similar(queries, graph, query.terms)?;
    let others = summed(&[
        &lexical.seeds,
        &session.seeds,
        &uncommitted.seeds,
        &similar.seeds,
    ]);
    let failure = failure_seeds(graph, query.failure, &others);
    let found = summed(&[&others, &failure.seeds]);
    let exploration = exploration(graph, &found);
    let vector = shares(&summed(&[&found, &exploration]));
    Ok(Seeds {
        lexical,
        session,
        uncommitted,
        similar,
        failure,
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
