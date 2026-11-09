//! Earlier work like the prompt's: the clusters of past tasks whose words overlap the prompt's
//! words, each giving the files its tasks changed.
//!
//! A cluster is found by the words of the files and symbols its tasks changed, the words that tell
//! it from the others first. Its first words, as many as the prompt has and at least five, are
//! compared with the prompt's, as a Jaccard score: what the two sets share over what they hold
//! between them. A cluster at or above the bar gives each of its files a seed of 0.6 times its
//! score; a file of several such clusters takes the best. Taking at least five of a cluster's words
//! keeps one word that a short prompt shares with a cluster of five words or more from making a
//! match: against the cluster's first two words it would score a third, against five a sixth.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::jaccard;
use cortex_store::{NodeId, Queries, StoreError};

use crate::graph::Graph;
use crate::prompt::PromptTerms;

/// The least Jaccard score between the prompt's words and a cluster's for the cluster to count.
pub const SIMILAR_BAR: f64 = 0.3;

/// What a cluster's files weigh as seeds, times its score.
pub const SIMILAR_WEIGHT: f64 = 0.6;

/// The fewest of a cluster's words compared with the prompt's, so that one word a short prompt
/// shares with a cluster's first words scores below the bar.
const MIN_TERMS_COMPARED: usize = 5;

/// What earlier work like the prompt's gives.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimilarMatch {
    /// Each file seeded, with its weight, by node.
    pub seeds: BTreeMap<u32, f64>,
    /// Each file seeded, with the best score of a cluster that gave it and how many tasks that
    /// cluster covers.
    pub files: BTreeMap<u32, (f64, u32)>,
    /// The best score of any cluster, at the bar or not: 0 with no cluster.
    pub best: f64,
}

/// The clusters the store `queries` reads holds that are like `terms`, as nodes of `graph`.
///
/// # Errors
///
/// The store's errors.
pub fn similar(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    terms: &PromptTerms,
) -> Result<SimilarMatch, StoreError> {
    let mut found = SimilarMatch::default();
    let words: BTreeSet<&str> = terms.words.iter().map(String::as_str).collect();
    if words.is_empty() {
        return Ok(found);
    }
    let compared = words.len().max(MIN_TERMS_COMPARED);
    for cluster in queries.archetypes()? {
        let theirs: BTreeSet<&str> = cluster
            .seed_terms
            .iter()
            .take(compared)
            .map(cortex_core::Identifier::as_str)
            .collect();
        let score = jaccard(&words, &theirs);
        found.best = found.best.max(score);
        if score < SIMILAR_BAR {
            continue;
        }
        for file in &cluster.files {
            let Some(index) = graph.index_of(*file) else {
                continue;
            };
            let weight = SIMILAR_WEIGHT * score;
            let held = found.seeds.entry(index).or_insert(0.0);
            *held = held.max(weight);
            let given = found.files.entry(index).or_insert((0.0, 0));
            if score > given.0 {
                *given = (score, cluster.episode_count);
            }
        }
    }
    Ok(found)
}
