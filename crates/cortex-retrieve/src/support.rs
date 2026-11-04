//! Which independent kinds of evidence point at a file: the prompt's words, earlier work like the
//! prompt's, and the links that join the file to the other files the seeds point at.
//!
//! A file is supported lexically when the prompt's words found it or one of its symbols, and by
//! similar work when a cluster of earlier tasks like the prompt changed it. It is supported by a
//! kind of link when a link of that kind joins it, or one of its symbols, to another file the
//! seeds point at, or to one of that file's symbols. Not every link counts, and each is held to
//! its floor on its own, never as a sum with others between the same two nodes. A reference counts
//! only when it is tied at 0.8 or more, the confidence every statement shown to an agent needs. A
//! co-access link counts only when its pair was worked on together, its weight after fading at
//! least 0.05: half of what one successful task that edited both gives, which a pair that was only
//! looked at never reaches. A co-change link counts as the miner wrote it, either way round.
//! Exploration's seeds are not evidence: they are where nothing points yet.

use std::collections::BTreeSet;

use cortex_core::EdgeType;
use cortex_learn::MAX_LINK_WEIGHT;

use crate::graph::{Graph, narrowed};
use crate::seeds::Seeds;

/// The least confidence a reference needs to count as support.
pub const STRUCTURAL_FLOOR: f64 = 0.8;

/// The least faded weight a co-access link needs to count as support.
pub const COACCESS_FLOOR: f64 = 0.05;

/// One independent kind of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Evidence {
    /// The prompt's words found it.
    Lexical,
    /// A reference or an import joins it to another file the seeds point at.
    Structural,
    /// It changes in the same commits as another file the seeds point at.
    CoChange,
    /// Agents worked on it together with another file the seeds point at.
    CoAccess,
    /// Earlier tasks like the prompt changed it.
    Similar,
}

/// The evidence behind the files of one retrieval: the files the seeds point at, and how strong a
/// link must be to count.
#[derive(Debug, Clone)]
pub struct Support<'a, Id> {
    /// The graph.
    graph: &'a Graph<Id>,
    /// What each source found.
    seeds: &'a Seeds,
    /// The files the seeds of the sources other than exploration point at, by node.
    seed_files: BTreeSet<u32>,
    /// The files a seed file's co-change links run to, by node.
    changed_with_seeds: BTreeSet<u32>,
    /// The least weight one structural link of the graph carries to count, as the graph keeps it.
    structural_floor: f64,
    /// The least weight one co-access link of the graph carries to count, as the graph keeps it.
    coaccess_floor: f64,
}

impl<'a, Id: Ord + Copy> Support<'a, Id> {
    /// The evidence `seeds` give over `graph`, its links held to floors in the weights the graph
    /// was weighed by.
    #[must_use]
    pub fn new(graph: &'a Graph<Id>, seeds: &'a Seeds) -> Self {
        let seed_files: BTreeSet<u32> = seeds
            .lexical
            .seeds
            .keys()
            .chain(seeds.session.seeds.keys())
            .chain(seeds.similar.seeds.keys())
            .filter_map(|node| graph.file_of(*node))
            .collect();
        let changed_with_seeds = seed_files
            .iter()
            .flat_map(|file| graph.links(*file, EdgeType::CoChange))
            .filter(|(_, weight)| *weight > 0.0)
            .map(|(target, _)| target)
            .collect();
        let type_weights = graph.type_weights();
        // A floor is narrowed as the graph narrows a link's weight, so a link at the floor exactly
        // is not lost to rounding.
        let floor = |weight: f64| f64::from(narrowed(weight));
        Self {
            graph,
            seeds,
            seed_files,
            changed_with_seeds,
            structural_floor: floor(STRUCTURAL_FLOOR * type_weights.structural),
            coaccess_floor: floor(COACCESS_FLOOR / MAX_LINK_WEIGHT * type_weights.coaccess),
        }
    }

    /// The kinds of evidence behind file `file`.
    #[must_use]
    pub fn of(&self, file: u32) -> BTreeSet<Evidence> {
        let mut found = BTreeSet::new();
        if self.seeds.lexical.files.contains_key(&file) {
            found.insert(Evidence::Lexical);
        }
        if self.seeds.similar.seeds.contains_key(&file) {
            found.insert(Evidence::Similar);
        }
        let kinds = [
            (
                EdgeType::Structural,
                Evidence::Structural,
                self.structural_floor,
            ),
            (EdgeType::CoChange, Evidence::CoChange, f64::MIN_POSITIVE),
            (EdgeType::CoAccess, Evidence::CoAccess, self.coaccess_floor),
        ];
        for (kind, evidence, floor) in kinds {
            if self.is_joined(file, (kind, floor)) {
                found.insert(evidence);
            }
        }
        // A co-change link runs one way, as its confidence does; either way says the two change
        // together.
        if self.changed_with_seeds.contains(&file) {
            found.insert(Evidence::CoChange);
        }
        found
    }

    /// Whether one link of `kind`, weighing at least `floor` on its own, runs from file `file` or
    /// one of its symbols to a node of a seed file other than `file`.
    fn is_joined(&self, file: u32, (kind, floor): (EdgeType, f64)) -> bool {
        std::iter::once(file)
            .chain(self.graph.symbols_of(file).iter().copied())
            .any(|node| {
                self.graph.peak_links(node, kind).any(|(target, peak)| {
                    peak >= floor
                        && self
                            .graph
                            .file_of(target)
                            .is_some_and(|other| other != file && self.seed_files.contains(&other))
                })
            })
    }
}
