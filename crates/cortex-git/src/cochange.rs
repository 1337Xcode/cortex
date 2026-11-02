//! Which files change together: from the commits a read gave, newest first, each file's commit
//! count and each pair's co-change count, both also weighted by how recent each commit is, and the
//! co-change edges they support. Pure: the caller reads the history and the files, and stores the
//! result.
//!
//! A commit's files are followed to their current paths through the renames of newer commits, and
//! only the files the caller names count (those that exist now, neither generated nor vendored). A
//! commit made `age` commits before the newest one read weighs `0.5^(age / half-life)`; a mass
//! commit counts towards the age of older ones but adds nothing, and its renames are not followed.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};

use cortex_core::RepoPath;

use crate::history::MinedCommit;

/// The fewest commits two files must change in together for an edge between them.
pub const MIN_COCHANGES: u32 = 2;

/// The least confidence an edge needs.
pub const MIN_CONFIDENCE: f64 = 0.1;

/// The most edges that start at one file, to the partners ranked highest by confidence times
/// `ln(1 + co-changes)`.
pub const MAX_PARTNERS: usize = 32;

/// A file's commit counts.
#[derive(Debug, Clone, PartialEq)]
pub struct FileCommits<F> {
    /// The file.
    pub file: F,
    /// How many commits read changed it.
    pub count: u32,
    /// The sum of those commits' weights.
    pub weighted: f64,
}

/// Two files' co-change counts.
#[derive(Debug, Clone, PartialEq)]
pub struct PairCommits<F> {
    /// The file that sorts first.
    pub file_a: F,
    /// The file that sorts second.
    pub file_b: F,
    /// How many commits read changed both.
    pub count: u32,
    /// The sum of those commits' weights.
    pub weighted: f64,
    /// The newest of those commits, as its sequence number: the oldest commit read is
    /// `first_seq`, each newer one the next number.
    pub last_commit_seq: u64,
}

/// A co-change edge: of `from`'s weighted commits, the share that also changed `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct CoChangeEdge<F> {
    /// The file the edge starts at.
    pub from: F,
    /// Its partner.
    pub to: F,
    /// The weighted co-changes over `from`'s weighted commits: at least [`MIN_CONFIDENCE`], at
    /// most 1.
    pub confidence: f64,
    /// How many commits changed both.
    pub co_changes: u32,
}

/// What a run of commits says about which files change together.
#[derive(Debug, Clone, PartialEq)]
pub struct CoChange<F> {
    /// Each file some commit changed, in file order.
    pub files: Vec<FileCommits<F>>,
    /// Each pair of files some commit changed together, in file order.
    pub pairs: Vec<PairCommits<F>>,
    /// The edges, by the file they start at, then by their partner.
    pub edges: Vec<CoChangeEdge<F>>,
}

/// What the commits say. `commits` are newest first, as a read gives them; `current` names each
/// file that counts by its current path; `half_life_commits` is the age at which a commit weighs
/// half as much as the newest; `first_seq` numbers the oldest commit given.
#[must_use]
pub fn compute<F: Copy + Ord + Hash, S: BuildHasher>(
    commits: &[MinedCommit],
    current: &HashMap<RepoPath, F, S>,
    half_life_commits: u32,
    first_seq: u64,
) -> CoChange<F> {
    let mut counter = Counter::new();
    let mut renamed_to: HashMap<RepoPath, RepoPath> = HashMap::new();
    let newest_seq = first_seq + commits.len().saturating_sub(1) as u64;
    for (age, commit) in commits.iter().enumerate() {
        let Some(changes) = &commit.changes else {
            continue;
        };
        let weight = weight(age, half_life_commits);
        let seq = newest_seq - age as u64;
        let mut files: Vec<F> = changes
            .iter()
            .filter_map(|change| current.get(now_named(&change.path, &renamed_to)))
            .copied()
            .collect();
        files.sort_unstable();
        files.dedup();
        counter.add(&files, weight, seq);
        // In older commits, a file this one renamed has its old path. Every new path is followed
        // to its current name before any of this commit's renames is noted, so that renames within
        // one commit (a to b, b to c) do not run into each other.
        let renames: Vec<(RepoPath, RepoPath)> = changes
            .iter()
            .filter_map(|change| {
                let from = change.renamed_from.clone()?;
                Some((from, now_named(&change.path, &renamed_to).clone()))
            })
            .collect();
        renamed_to.extend(renames);
    }
    counter.finish()
}

/// The name `path` has now, following the renames of newer commits.
fn now_named<'a>(path: &'a RepoPath, renamed_to: &'a HashMap<RepoPath, RepoPath>) -> &'a RepoPath {
    renamed_to.get(path).unwrap_or(path)
}

/// A commit's weight at `age` commits before the newest: `0.5^(age / half_life_commits)`, which is
/// `exp(-age * ln 2 / half_life_commits)`.
fn weight(age: usize, half_life_commits: u32) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "ages and half-lives are far below 2^52"
    )]
    let halvings = age as f64 / f64::from(half_life_commits.max(1));
    0.5_f64.powf(halvings)
}

/// Running totals of commit and co-change counts.
struct Counter<F> {
    /// Per file: commits and weighted commits.
    files: HashMap<F, (u32, f64)>,
    /// Per pair, the first file sorting first: co-changes, weighted co-changes, newest sequence.
    pairs: HashMap<(F, F), (u32, f64, u64)>,
}

impl<F: Copy + Ord + Hash> Counter<F> {
    /// No counts yet.
    fn new() -> Self {
        Self {
            files: HashMap::new(),
            pairs: HashMap::new(),
        }
    }

    /// Counts one commit of `weight` and sequence number `seq` that changed `files`, sorted and
    /// without repeats.
    fn add(&mut self, files: &[F], weight: f64, seq: u64) {
        for (index, &file) in files.iter().enumerate() {
            let total = self.files.entry(file).or_insert((0, 0.0));
            total.0 += 1;
            total.1 += weight;
            for &partner in files.iter().skip(index + 1) {
                let pair = self.pairs.entry((file, partner)).or_insert((0, 0.0, seq));
                pair.0 += 1;
                pair.1 += weight;
                pair.2 = pair.2.max(seq);
            }
        }
    }

    /// The counts in file order, and the edges they support.
    fn finish(self) -> CoChange<F> {
        let mut files: Vec<FileCommits<F>> = self
            .files
            .iter()
            .map(|(&file, &(count, weighted))| FileCommits {
                file,
                count,
                weighted,
            })
            .collect();
        files.sort_unstable_by_key(|file| file.file);
        let mut pairs: Vec<PairCommits<F>> = self
            .pairs
            .iter()
            .map(
                |(&(file_a, file_b), &(count, weighted, last_commit_seq))| PairCommits {
                    file_a,
                    file_b,
                    count,
                    weighted,
                    last_commit_seq,
                },
            )
            .collect();
        pairs.sort_unstable_by_key(|pair| (pair.file_a, pair.file_b));
        let edges = edges(&pairs, &self.files);
        CoChange {
            files,
            pairs,
            edges,
        }
    }
}

/// The edges `pairs` support, each direction on its own: at least [`MIN_COCHANGES`] co-changes,
/// a confidence of at least [`MIN_CONFIDENCE`], and at most [`MAX_PARTNERS`] from one file, by
/// the file they start at, then by partner. `files` gives each file's commits and weighted
/// commits; a file it does not give starts no edge. An edge from a file is right only when
/// `pairs` holds every pair of that file.
#[must_use]
pub fn edges<F: Copy + Ord + Hash, S: BuildHasher>(
    pairs: &[PairCommits<F>],
    files: &HashMap<F, (u32, f64), S>,
) -> Vec<CoChangeEdge<F>> {
    let mut from_each: BTreeMap<F, Vec<(f64, u32, F)>> = BTreeMap::new();
    for pair in pairs.iter().filter(|pair| pair.count >= MIN_COCHANGES) {
        for (from, to) in [(pair.file_a, pair.file_b), (pair.file_b, pair.file_a)] {
            let Some(&(_, weighted)) = files.get(&from) else {
                continue;
            };
            // A file's weighted commits hold every weighted co-change of its pairs; rounding
            // aside, the share is at most 1.
            let confidence = (pair.weighted / weighted).min(1.0);
            if confidence >= MIN_CONFIDENCE {
                from_each
                    .entry(from)
                    .or_default()
                    .push((confidence, pair.count, to));
            }
        }
    }
    let mut edges = Vec::new();
    for (from, mut partners) in from_each {
        // The highest ranked first; of two ranked alike, the partner that sorts first.
        partners.sort_by(|a, b| rank(b).total_cmp(&rank(a)).then(a.2.cmp(&b.2)));
        partners.truncate(MAX_PARTNERS);
        partners.sort_by_key(|partner| partner.2);
        edges.extend(
            partners
                .into_iter()
                .map(|(confidence, co_changes, to)| CoChangeEdge {
                    from,
                    to,
                    confidence,
                    co_changes,
                }),
        );
    }
    edges
}

/// A partner's rank: its confidence times `ln(1 + co-changes)`.
fn rank<F>(partner: &(f64, u32, F)) -> f64 {
    partner.0 * f64::from(partner.1).ln_1p()
}
