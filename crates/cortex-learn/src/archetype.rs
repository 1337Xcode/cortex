//! Clusters of tasks that changed much the same files, so that a later task can be told what
//! tasks like it changed.
//!
//! A task belongs with another when the files they changed overlap enough: half of what they
//! changed between them is what both changed. Each task joins the cluster it is most like, if it
//! is like any enough, and otherwise starts one. A cluster is compared by the files its oldest
//! task changed, so it does not drift as it grows. Only tasks whose work stood are clustered: a
//! task that succeeded, or one nobody can vouch for either way; not one that failed, was put back
//! or was left before it ended, whose files say nothing about where such work belongs.
//!
//! A cluster is found by words: those of the files and symbols its tasks changed, as the search
//! index holds them. A word ranks by how many of the cluster's tasks gave it, weighed by how many
//! of all the tasks clustered gave it, so that words every task gives (`py`, `src`) fall behind
//! the words that tell this cluster from the others. That is all the text there is to go on, since
//! a task's record keeps no prompt, and they are the words a prompt is looked up by.
//!
//! A pass reads the newest records it is asked for, at most [`MAX_CLUSTERED_TASKS`], and compares
//! each task only with the clusters whose oldest task shares a file with it, as no other can reach
//! the bar. Each comparison costs the size of the two sets of files, so a pass costs at most the
//! tasks read times the clusters one task shares a file with times the files of a task.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Identifier, TaskOutcome};
use cortex_store::{ArchetypeRecord, EpisodeRecord, NodeId, Queries, StoreError};

/// How alike two tasks' changed files must be for the tasks to belong to one cluster: half of
/// what they changed between them.
const JOINS_A_CLUSTER: f64 = 0.5;

/// Most task records one pass reads, the newest: months of one developer's work, and few enough
/// that a pass where every task shares one file with every cluster still ends in well under a
/// second.
pub const MAX_CLUSTERED_TASKS: usize = 2_000;

/// Most clusters kept: those covering the most tasks, and of those covering as many, the ones
/// worked in most recently. A repository has a few shapes of task, and what is left over is one
/// task each.
const MAX_CLUSTERS: usize = 256;

/// Most files a cluster keeps, those most of its tasks changed first.
const MAX_CLUSTER_FILES: usize = 50;

/// Most words a cluster is found by, the best first, so that a reader can take as many of the
/// first as it needs.
const MAX_CLUSTER_TERMS: usize = 50;

/// The clusters the newest `limit` task records fall into, at most [`MAX_CLUSTERED_TASKS`], as of
/// `updated_at_ms`, read with `queries`.
///
/// # Errors
///
/// The store's errors.
pub fn archetypes_of(
    queries: Queries<'_>,
    (limit, updated_at_ms): (usize, i64),
) -> Result<Vec<ArchetypeRecord>, StoreError> {
    // INVARIANT: budgeted (a pass reads at most MAX_CLUSTERED_TASKS records)
    let records = queries.episodes(limit.min(MAX_CLUSTERED_TASKS))?;
    let words = words_of(queries, &records)?;
    let mut clusters: Vec<Cluster> = Vec::new();
    // The clusters whose oldest task changed each file, oldest first.
    let mut by_file: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
    for (at, record) in records.iter().enumerate() {
        let changed = changed_files(record);
        if changed.is_empty() || !stood(record.outcome) {
            continue;
        }
        let terms = terms_of(record, &words);
        if let Some(cluster) =
            most_like(&clusters, &by_file, &changed).and_then(|found| clusters.get_mut(found))
        {
            cluster.join(&changed, &terms, at);
            continue;
        }
        for node in &changed {
            by_file.entry(*node).or_default().push(clusters.len());
        }
        clusters.push(Cluster::of(&changed, &terms, at));
    }
    // How many of the tasks clustered gave each word: each task is in one cluster.
    let mut given_in_all: BTreeMap<Identifier, u32> = BTreeMap::new();
    for (term, (_, count)) in clusters.iter().flat_map(|cluster| &cluster.given_by) {
        let total = given_in_all.entry(term.clone()).or_default();
        *total = total.saturating_add(*count);
    }
    // INVARIANT: deterministic (ties on tasks go to the newest work, which no two clusters share)
    clusters.sort_by_key(|cluster| (Reverse(cluster.tasks), Reverse(cluster.newest)));
    clusters.truncate(MAX_CLUSTERS);
    Ok(clusters
        .into_iter()
        .map(|cluster| cluster.record(&given_in_all, updated_at_ms))
        .collect())
}

/// Whether a task that ended with `outcome` did work that stood: it succeeded, or nothing says
/// otherwise.
fn stood(outcome: TaskOutcome) -> bool {
    matches!(outcome, TaskOutcome::Success | TaskOutcome::Unknown)
}

/// The files one task changed.
fn changed_files(record: &EpisodeRecord) -> BTreeSet<NodeId> {
    record.change.iter().map(|file| file.node).collect()
}

/// The files and symbols one task changed, in the order its record lists them: each file, then the
/// symbols it changed in that file.
fn changed_nodes(record: &EpisodeRecord) -> impl Iterator<Item = NodeId> + '_ {
    record.change.iter().flat_map(|file| {
        std::iter::once(file.node).chain(file.symbols.iter().map(|symbol| symbol.node))
    })
}

/// The words each file and symbol the records changed is found by in the search index, each once,
/// read together.
fn words_of(
    queries: Queries<'_>,
    records: &[EpisodeRecord],
) -> Result<BTreeMap<NodeId, Vec<Identifier>>, StoreError> {
    let nodes: BTreeSet<NodeId> = records.iter().flat_map(changed_nodes).collect();
    let listed: Vec<NodeId> = nodes.into_iter().collect();
    Ok(queries
        .search_tokens_of(&listed)?
        .into_iter()
        .map(|(node, tokens)| {
            let mut words: Vec<Identifier> = Vec::new();
            // A token too long to be an identifier names nothing a prompt would hold.
            for word in tokens
                .split(' ')
                .filter_map(|token| Identifier::new(token).ok())
            {
                if !words.contains(&word) {
                    words.push(word);
                }
            }
            (node, words)
        })
        .collect())
}

/// The words one task's changes are found by, each once, in the order its record lists the files
/// and symbols they belong to.
fn terms_of(record: &EpisodeRecord, words: &BTreeMap<NodeId, Vec<Identifier>>) -> Vec<Identifier> {
    let mut terms: Vec<Identifier> = Vec::new();
    for word in changed_nodes(record).flat_map(|node| words.get(&node).into_iter().flatten()) {
        if !terms.contains(word) {
            terms.push(word.clone());
        }
    }
    terms
}

/// The cluster a task that changed `changed` is most like, if it is like any enough: of the
/// clusters whose oldest task shares a file with it, the one it overlaps most, the older on a tie.
fn most_like(
    clusters: &[Cluster],
    by_file: &BTreeMap<NodeId, Vec<usize>>,
    changed: &BTreeSet<NodeId>,
) -> Option<usize> {
    let sharing: BTreeSet<usize> = changed
        .iter()
        .filter_map(|node| by_file.get(node))
        .flatten()
        .copied()
        .collect();
    let mut best: Option<(usize, f64)> = None;
    for at in sharing {
        let Some(cluster) = clusters.get(at) else {
            continue;
        };
        let share = overlap(&cluster.seed, changed);
        if share >= JOINS_A_CLUSTER && best.is_none_or(|(_, top)| share.total_cmp(&top).is_gt()) {
            best = Some((at, share));
        }
    }
    best.map(|(at, _)| at)
}

/// What share of the files two tasks changed between them they both changed, from 0 for tasks with
/// nothing in common to 1 for tasks that changed the same files.
fn overlap(left: &BTreeSet<NodeId>, right: &BTreeSet<NodeId>) -> f64 {
    let both = left.intersection(right).count();
    let either = left.union(right).count();
    if either == 0 {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a task changes far fewer files than a count loses precision at"
    )]
    let share = both as f64 / either as f64;
    share
}

/// How well a word a cluster's tasks gave `in_cluster` times, of the `in_all` times the tasks
/// clustered gave it, tells the cluster apart: how often its tasks gave it, times the share of
/// those who gave it that are its own. Two of two tasks giving a word no other task gives score 2;
/// two of two giving a word all of eight tasks give score a half.
fn telling(in_cluster: u32, in_all: u32) -> f64 {
    f64::from(in_cluster) * f64::from(in_cluster) / f64::from(in_all.max(in_cluster))
}

/// A cluster as it is built: the files of its oldest task, how many tasks have joined, how many
/// of them changed each file and gave each word, and its newest task.
struct Cluster {
    /// The files the cluster's oldest task changed, which the others are compared against.
    seed: BTreeSet<NodeId>,
    /// How many tasks it covers.
    tasks: u32,
    /// How many of them changed each file.
    changed_by: BTreeMap<NodeId, u32>,
    /// For each word its tasks gave, when it was first met (a count of the words met before it)
    /// and how many of them gave it.
    given_by: BTreeMap<Identifier, (usize, u32)>,
    /// Where its newest task stands among the records read, oldest first.
    newest: usize,
}

impl Cluster {
    /// A cluster of the one task that changed `files`, found by `terms`, the `at`-th record read.
    fn of(files: &BTreeSet<NodeId>, terms: &[Identifier], at: usize) -> Self {
        let mut cluster = Self {
            seed: files.clone(),
            tasks: 0,
            changed_by: BTreeMap::new(),
            given_by: BTreeMap::new(),
            newest: at,
        };
        cluster.join(files, terms, at);
        cluster
    }

    /// Takes in a task that changed `files`, found by `terms`, the `at`-th record read.
    fn join(&mut self, files: &BTreeSet<NodeId>, terms: &[Identifier], at: usize) {
        self.tasks = self.tasks.saturating_add(1);
        self.newest = at;
        for node in files {
            let count = self.changed_by.entry(*node).or_default();
            *count = count.saturating_add(1);
        }
        for term in terms {
            let met = self.given_by.len();
            let (_, count) = self.given_by.entry(term.clone()).or_insert((met, 0));
            *count = count.saturating_add(1);
        }
    }

    /// The cluster as it is stored: the files most of its tasks changed, ties going to the smaller
    /// node; the words that best tell it apart, given how many of all the tasks clustered gave
    /// each (`given_in_all`), ties going to the word more of its tasks gave and then to the word
    /// met first; and how many tasks it covers.
    fn record(
        self,
        given_in_all: &BTreeMap<Identifier, u32>,
        updated_at_ms: i64,
    ) -> ArchetypeRecord {
        let mut files: Vec<(NodeId, u32)> = self.changed_by.into_iter().collect();
        // INVARIANT: deterministic (no two files share a node, so the order is total)
        files.sort_by_key(|(node, count)| (Reverse(*count), *node));
        files.truncate(MAX_CLUSTER_FILES);
        let mut terms: Vec<(Identifier, f64, u32, usize)> = self
            .given_by
            .into_iter()
            .map(|(term, (met, count))| {
                // Every task of a cluster is among those clustered, so `given_in_all` counts each
                // of its words at least as often as it does.
                let in_all = given_in_all.get(&term).copied().unwrap_or(count);
                let score = telling(count, in_all);
                (term, score, count, met)
            })
            .collect();
        // INVARIANT: deterministic (no two words were met at once, so the order is total)
        terms.sort_by(
            |(_, left, left_count, left_met), (_, right, right_count, right_met)| {
                right
                    .total_cmp(left)
                    .then(right_count.cmp(left_count))
                    .then(left_met.cmp(right_met))
            },
        );
        terms.truncate(MAX_CLUSTER_TERMS);
        ArchetypeRecord {
            seed_terms: terms.into_iter().map(|(term, ..)| term).collect(),
            files: files.into_iter().map(|(node, _)| node).collect(),
            episode_count: self.tasks,
            updated_at_ms,
        }
    }
}
