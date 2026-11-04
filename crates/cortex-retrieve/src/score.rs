//! What each file scores: the activation that reached the file and its symbols, each part weighed
//! by how much agents have worked there, with the files no work order points at left out.
//!
//! A node's score is its activation times its prior: 1.5 for a landmark, 1.25 for an interesting
//! node, 1 for the rest, each tier as of the latest task. A file's score is its own node's plus its
//! symbols'. A generated or vendored file, which the index holds at file level only, is never
//! where an agent should start, and is left out.

use std::collections::BTreeMap;

use cortex_core::{InterestTier, NodeKind};

use crate::graph::Graph;
use crate::push::Activation;

/// What a landmark's activation is multiplied by.
pub const LANDMARK_PRIOR: f64 = 1.5;

/// What an interesting node's activation is multiplied by.
pub const INTERESTING_PRIOR: f64 = 1.25;

/// One file's score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredFile {
    /// The file, by node.
    pub file: u32,
    /// Its node's score plus its symbols'.
    pub score: f64,
    /// Its node's activation plus its symbols', before any prior.
    pub activation: f64,
    /// Its symbols that scored anything, by node, the best first and then by node.
    pub symbols: Vec<(u32, f64)>,
}

/// The files `activation` reached in `graph`, each with its score, the best first and then by
/// node, those a work order may not point at left out.
#[must_use]
pub fn scored_files<Id: Ord + Copy>(graph: &Graph<Id>, activation: &Activation) -> Vec<ScoredFile> {
    let mut files: BTreeMap<u32, ScoredFile> = BTreeMap::new();
    for (node, kept) in &activation.kept {
        let Some(file) = graph.file_of(*node) else {
            continue;
        };
        if !graph
            .standing(file)
            .is_some_and(|standing| standing.is_listed)
        {
            continue;
        }
        let score = kept * prior(graph, *node);
        let held = files.entry(file).or_insert_with(|| ScoredFile {
            file,
            score: 0.0,
            activation: 0.0,
            symbols: Vec::new(),
        });
        held.score += score;
        held.activation += kept;
        if graph.kind(*node) == Some(NodeKind::Symbol) && score > 0.0 {
            held.symbols.push((*node, score));
        }
    }
    let mut ranked: Vec<ScoredFile> = files.into_values().collect();
    for file in &mut ranked {
        file.symbols
            .sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then(left.file.cmp(&right.file))
    });
    ranked
}

/// `scored` with the symbols of its first `staged` files scored from `activation` instead: each
/// symbol's activation times its prior, those above zero, the best first and then by node. The
/// files keep their own scores.
#[must_use]
pub fn with_symbols<Id: Ord + Copy>(
    graph: &Graph<Id>,
    mut scored: Vec<ScoredFile>,
    activation: &Activation,
    staged: usize,
) -> Vec<ScoredFile> {
    for file in scored.iter_mut().take(staged) {
        let mut symbols: Vec<(u32, f64)> = graph
            .symbols_of(file.file)
            .iter()
            .filter_map(|symbol| {
                let score = activation.of(*symbol) * prior(graph, *symbol);
                (score > 0.0).then_some((*symbol, score))
            })
            .collect();
        symbols.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
        file.symbols = symbols;
    }
    scored
}

/// What node `node`'s activation is multiplied by, from its tier as of the latest task.
fn prior<Id: Ord + Copy>(graph: &Graph<Id>, node: u32) -> f64 {
    match graph.standing(node).map(|standing| standing.tier) {
        Some(InterestTier::Landmark) => LANDMARK_PRIOR,
        Some(InterestTier::Interesting) => INTERESTING_PRIOR,
        Some(InterestTier::Dark) | None => 1.0,
    }
}
