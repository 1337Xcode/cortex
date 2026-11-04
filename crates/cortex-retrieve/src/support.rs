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

use std::collections::{BTreeMap, BTreeSet};

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
    /// The files the seeds of the prompt's words, the session and earlier work point at, by node:
    /// neither the dark neighbours exploration adds nor the files of the uncommitted diff, which
    /// the work tree holds whatever the prompt is about, so that neither stands as evidence.
    seed_files: BTreeSet<u32>,
    /// For each file a seed file's co-change links run to, by node: those seed files, each with
    /// the link's weight.
    changed_with_seeds: BTreeMap<u32, Vec<(u32, f64)>>,
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
        let mut changed_with_seeds: BTreeMap<u32, Vec<(u32, f64)>> = BTreeMap::new();
        for file in &seed_files {
            for (target, peak) in graph.peak_links(*file, EdgeType::CoChange) {
                if peak > 0.0 {
                    changed_with_seeds
                        .entry(target)
                        .or_default()
                        .push((*file, peak));
                }
            }
        }
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
            (EdgeType::Structural, Evidence::Structural),
            (EdgeType::CoChange, Evidence::CoChange),
            (EdgeType::CoAccess, Evidence::CoAccess),
        ];
        for (kind, evidence) in kinds {
            if !self.partners(file, kind).is_empty() {
                found.insert(evidence);
            }
        }
        found
    }

    /// The other seed files one link of `kind`, weighing at least its floor on its own, joins file
    /// `file` or one of its symbols to, or one of their symbols, each with the heaviest such link:
    /// the heaviest first, then by file. A co-change link counts whichever way it runs.
    #[must_use]
    pub fn partners(&self, file: u32, kind: EdgeType) -> Vec<(u32, f64)> {
        let floor = match kind {
            EdgeType::Structural => self.structural_floor,
            EdgeType::CoAccess => self.coaccess_floor,
            // The miner keeps a co-change link only at its own least confidence, so any weight
            // above none counts.
            EdgeType::CoChange => f64::MIN_POSITIVE,
            // A symbol's place in its file and a fact's anchor say nothing of two files' work.
            EdgeType::Containment | EdgeType::About => return Vec::new(),
        };
        let mut found: BTreeMap<u32, f64> = BTreeMap::new();
        let mut add = |other: u32, peak: f64| {
            if other != file && peak >= floor && self.seed_files.contains(&other) {
                let held = found.entry(other).or_insert(0.0);
                *held = held.max(peak);
            }
        };
        for node in std::iter::once(file).chain(self.graph.symbols_of(file).iter().copied()) {
            for (target, peak) in self.graph.peak_links(node, kind) {
                if let Some(other) = self.graph.file_of(target) {
                    add(other, peak);
                }
            }
        }
        // A co-change link runs one way, as its confidence does; either way says the two change
        // together.
        if kind == EdgeType::CoChange {
            for (other, peak) in self.changed_with_seeds.get(&file).into_iter().flatten() {
                add(*other, *peak);
            }
        }
        let mut partners: Vec<(u32, f64)> = found.into_iter().collect();
        partners.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
        partners
    }
}
