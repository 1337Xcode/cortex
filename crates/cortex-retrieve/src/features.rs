//! The gate's features, from one retrieval: how well the prompt's words match, how much of the
//! activation the top files hold, how many independent kinds of evidence stand behind them, how
//! like earlier work the prompt is, and whether the session is continuing its own work.
//!
//! The evidence counted is that behind the top three files; the activation's mass is the top five
//! files' share of all the activation kept. The session continues its own work when one of the top
//! three files was edited earlier in it and the prompt's words also found that file.

use std::collections::BTreeSet;

use crate::gate::Features;
use crate::push::Activation;
use crate::score::ScoredFile;
use crate::seeds::Seeds;
use crate::support::{Evidence, Support};

/// How many of the best files the evidence is counted over.
pub const GATED_FILES: usize = 3;

/// How many of the best files the activation's mass is taken over.
pub const MASSED_FILES: usize = 5;

/// What one retrieval found, as the gate weighs it.
#[derive(Debug, Clone, Copy)]
pub struct Found<'a, Id> {
    /// What each source seeded.
    pub seeds: &'a Seeds,
    /// Where activation went.
    pub activation: &'a Activation,
    /// The files it reached, the best first.
    pub scored: &'a [ScoredFile],
    /// The evidence behind each file.
    pub support: &'a Support<'a, Id>,
}

/// The features of what `found` holds, with `recent_precision`, the share of the repository's
/// latest delivered injections that were used, when there are any.
#[must_use]
pub fn features_of<Id: Ord + Copy>(
    found: &Found<'_, Id>,
    recent_precision: Option<f64>,
) -> Features {
    let top = found.scored.iter().take(GATED_FILES);
    let kinds: Vec<(u32, BTreeSet<Evidence>)> = top
        .map(|file| (file.file, found.support.of(file.file)))
        .collect();
    let agreeing = kinds
        .iter()
        .map(|(_, kinds)| kinds.len())
        .max()
        .unwrap_or(0);
    let supporting = kinds
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .collect::<BTreeSet<Evidence>>()
        .len();
    let is_continuing = kinds.iter().any(|(file, kinds)| {
        found.seeds.session.edited_files.contains(file) && kinds.contains(&Evidence::Lexical)
    });
    Features {
        top_lexical: found.seeds.lexical.top_lexical,
        top_mass: top_mass(found.activation, found.scored),
        agreeing_kinds: u8::try_from(agreeing).unwrap_or(u8::MAX),
        supporting_kinds: u8::try_from(supporting).unwrap_or(u8::MAX),
        similar_task: found.seeds.similar.best,
        recent_precision,
        is_continuing,
    }
}

/// The share of all the activation `activation` kept that the best [`MASSED_FILES`] of `scored`
/// hold, from 0 to 1; 0 when nothing kept any.
fn top_mass(activation: &Activation, scored: &[ScoredFile]) -> f64 {
    let total: f64 = activation.kept.iter().map(|(_, kept)| kept).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let top: f64 = scored
        .iter()
        .take(MASSED_FILES)
        .map(|file| file.activation)
        .sum();
    (top / total).clamp(0.0, 1.0)
}
