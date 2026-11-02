//! What a task's work adds to each node's interest, and which tier each score puts a node in.
//!
//! Interest follows what the task did, an edit counting for most and a file a search found but
//! never opened for least. An edit or a read of some of a file's lines goes to the symbols those
//! lines touch, and a symbol's increment also brightens its file and the folder holding it, so
//! that the map lights up around the work. Nothing of this reaches an agent yet: interest is the
//! map's brightness and, later, part of what retrieval ranks by.
//!
//! A score is kept as of the task it was last touched at and decays by a factor for each task
//! since, so a node's interest now is its stored score brought forward to the task that asks.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{CommandKind, InterestTier, LineRange, RepoPath};
use cortex_store::{InterestRow, NodeId, Queries, StoreError, SymbolSpan, WriteTx};

use crate::task_state::{FileNode, Step, TaskState};

/// What an edit adds to each symbol its lines touch, or to the file when they touch none. The work
/// itself: nothing a task does says more about what matters to it.
const EDIT: f64 = 1.0;

/// What a test or build that passed after a file was edited adds to that file. Less than the edit:
/// it says the work holds up, not that the file is where the work is.
const PASSED_AFTER_EDIT: f64 = 0.5;

/// What a read of some of a file's lines adds to each symbol they cover, or to the file when they
/// cover none.
const READ_RANGE: f64 = 0.4;

/// What a read of a whole file adds to it. Less than a read of chosen lines, which says which part
/// of the file the reader was after.
const READ_FILE: f64 = 0.3;

/// What a file a search found adds to it when the task went on to read or edit it.
const HIT_OPENED: f64 = 0.2;

/// What a file a search found adds to it when the task never opened it. Almost nothing: it says
/// only that the file holds a word the task searched for.
const HIT_UNOPENED: f64 = 0.05;

/// What share of a symbol's increment the file holding it takes.
const FILE_SHARE: f64 = 0.5;

/// What share of a symbol's increment the folder holding its file takes.
const FOLDER_SHARE: f64 = 0.25;

/// The score a node must stand above to be more than fog.
const INTERESTING_ABOVE: f64 = 0.2;

/// The score a file must reach to be a landmark, however few files reach it.
const LANDMARK_FROM: f64 = 3.0;

/// The fewest files the landmarks may take, so that a small repository has landmarks at all.
const LANDMARKS_MIN: usize = 5;

/// The most files the landmarks may take, so that a large repository has few.
const LANDMARKS_MAX: usize = 50;

/// How many of a repository's files the landmarks may take, as one in this many.
const LANDMARK_IN: u64 = 100;

/// What a task's work adds to the interest of the nodes it touched, by node.
pub type EarnedInterest = BTreeMap<NodeId, f64>;

/// What the work `state` shows earns each node, read with `queries` for the symbols the lines
/// touch and the folders holding the files.
///
/// # Errors
///
/// The store's errors.
pub fn interest_of(queries: Queries<'_>, state: &TaskState) -> Result<EarnedInterest, StoreError> {
    let mut earned = EarnedInterest::new();
    let mut places = Places::new(queries);
    let (first_edits, last_opens) = (first_edits(state), last_opens(state));
    for (at, step) in state.steps().iter().enumerate() {
        match step {
            Step::Edit { file, range, .. } => {
                add_within(&mut earned, &mut places, (file, *range), EDIT)?;
            }
            Step::Read { file, range, .. } => {
                let score = if range.is_some() {
                    READ_RANGE
                } else {
                    READ_FILE
                };
                add_within(&mut earned, &mut places, (file, *range), score)?;
            }
            Step::Search { hits, .. } => {
                for hit in hits {
                    let opened = last_opens.get(&hit.path).is_some_and(|open| *open > at);
                    let score = if opened { HIT_OPENED } else { HIT_UNOPENED };
                    if let Some(node) = hit.node {
                        add(&mut earned, node, score);
                    }
                }
            }
            // A check that passed vouches for every file edited before it, each once however many
            // times the task edited it.
            Step::Run {
                check: Some(check),
                exit_code: Some(0),
                ..
            } if matches!(check.kind, CommandKind::Test | CommandKind::Build) => {
                for (node, edited_at) in first_edits.values() {
                    if *edited_at < at {
                        add(&mut earned, *node, PASSED_AFTER_EDIT);
                    }
                }
            }
            Step::Run { .. } => {}
        }
    }
    Ok(earned)
}

/// Adds `earned` to the stored interest in `transaction`, each score first brought forward to task
/// `seq` by `decay` for each task since the node was last touched, and writes the tier each score
/// puts its node in, the landmarks chosen again from the files the store holds afterwards.
///
/// # Errors
///
/// The store's errors.
pub fn add_interest(
    transaction: &mut WriteTx<'_>,
    (seq, decay): (u64, f64),
    earned: &EarnedInterest,
) -> Result<(), StoreError> {
    let nodes: Vec<NodeId> = earned.keys().copied().collect();
    let held: BTreeMap<NodeId, InterestRow> = transaction
        .queries()
        .interest_of(&nodes)?
        .into_iter()
        .map(|row| (row.node, row))
        .collect();
    let added: Vec<InterestRow> = earned
        .iter()
        .map(|(node, add)| {
            let before = held
                .get(node)
                .map_or(0.0, |row| decayed(row.score, row.last_seq, seq, decay));
            InterestRow {
                node: *node,
                score: before + add,
                last_seq: seq,
                tier: tier_of(before + add),
            }
        })
        .collect();
    transaction.replace_interest(&added)?;
    retier_landmarks(transaction, (seq, decay))
}

/// Writes the tier of every file that could be a landmark, the landmarks chosen from the scores
/// the store holds now, each brought forward to task `seq`.
///
/// A file reaches the landmarks' threshold only from a stored score at least as high, decay never
/// raising one, so the files stored at or above it are every candidate there is. One that has
/// fallen out of the landmarks is among them and is written back as merely interesting.
fn retier_landmarks(
    transaction: &mut WriteTx<'_>,
    (seq, decay): (u64, f64),
) -> Result<(), StoreError> {
    let candidates = transaction.queries().file_interest_from(LANDMARK_FROM)?;
    let scores: BTreeMap<NodeId, f64> = candidates
        .iter()
        .map(|row| (row.node, decayed(row.score, row.last_seq, seq, decay)))
        .collect();
    let files_in_repo = transaction.queries().counts()?.files;
    let landmarks = landmarks(&scores, files_in_repo);
    let tiered: Vec<InterestRow> = candidates
        .into_iter()
        .map(|row| {
            let tier = if landmarks.contains(&row.node) {
                InterestTier::Landmark
            } else {
                tier_of(scores.get(&row.node).copied().unwrap_or(row.score))
            };
            InterestRow { tier, ..row }
        })
        .collect();
    transaction.replace_interest(&tiered)
}

/// Adds `score` to `node`.
fn add(earned: &mut EarnedInterest, node: NodeId, score: f64) {
    *earned.entry(node).or_default() += score;
}

/// Adds `score` to the symbols of `file` that the lines `range` touch, each also brightening the
/// file and the folder holding it; or to the file alone when the lines touch no symbol, there is
/// no range, or the store holds no symbols for the file.
fn add_within(
    earned: &mut EarnedInterest,
    places: &mut Places<'_>,
    (file, range): (&FileNode, Option<LineRange>),
    score: f64,
) -> Result<(), StoreError> {
    let covering = match range {
        Some(range) => places.covering(&file.path, range)?,
        None => Vec::new(),
    };
    if covering.is_empty() {
        if let Some(node) = file.node {
            add(earned, node, score);
        }
        return Ok(());
    }
    let folder = places.folder_of(&file.path)?;
    for node in covering {
        add(earned, node, score);
        if let Some(file) = file.node {
            add(earned, file, score * FILE_SHARE);
        }
        if let Some(folder) = folder {
            add(earned, folder, score * FOLDER_SHARE);
        }
    }
    Ok(())
}

/// The `score` a node had at task `last_seq`, brought forward to task `seq` by `decay` for each
/// task since. A score written at a task yet to come is left as it is.
fn decayed(score: f64, last_seq: u64, seq: u64, decay: f64) -> f64 {
    let tasks = i32::try_from(seq.saturating_sub(last_seq)).unwrap_or(i32::MAX);
    score * decay.powi(tasks)
}

/// The tier a score falls in on its own, before the landmarks are chosen.
fn tier_of(score: f64) -> InterestTier {
    if score > INTERESTING_ABOVE {
        InterestTier::Interesting
    } else {
        InterestTier::Dark
    }
}

/// The landmarks among the files `scores` gives the scores of: the highest scoring of those that
/// reach [`LANDMARK_FROM`], as many as a repository of `files_in_repo` files allows. Files of
/// equal score are taken in node order, so the same scores always give the same landmarks.
fn landmarks(scores: &BTreeMap<NodeId, f64>, files_in_repo: u64) -> BTreeSet<NodeId> {
    let mut ranked: Vec<(NodeId, f64)> = scores
        .iter()
        .filter(|(_, score)| **score >= LANDMARK_FROM)
        .map(|(node, score)| (*node, *score))
        .collect();
    ranked.sort_by(|(left_node, left), (right_node, right)| {
        right
            .total_cmp(left)
            .then(left_node.get().cmp(&right_node.get()))
    });
    ranked
        .into_iter()
        .take(landmark_count(files_in_repo))
        .map(|(node, _)| node)
        .collect()
}

/// How many files a repository of `files_in_repo` files may have as landmarks: one in
/// [`LANDMARK_IN`], rounded up, and then no fewer than [`LANDMARKS_MIN`] nor more than
/// [`LANDMARKS_MAX`].
fn landmark_count(files_in_repo: u64) -> usize {
    let share = files_in_repo.div_ceil(LANDMARK_IN);
    usize::try_from(share)
        .unwrap_or(usize::MAX)
        .clamp(LANDMARKS_MIN, LANDMARKS_MAX)
}

/// Where the increments of the files a task touched go, each file asked about once.
struct Places<'a> {
    /// The store.
    queries: Queries<'a>,
    /// Each file's symbols, by the file's path; empty for a file with none and for one the store
    /// does not hold.
    symbols: BTreeMap<RepoPath, Vec<SymbolSpan>>,
    /// The node of the folder holding each file, by the file's path.
    folders: BTreeMap<RepoPath, Option<NodeId>>,
}

impl<'a> Places<'a> {
    /// Nothing asked yet.
    fn new(queries: Queries<'a>) -> Self {
        Self {
            queries,
            symbols: BTreeMap::new(),
            folders: BTreeMap::new(),
        }
    }

    /// The nodes of the symbols of the file at `path` that the lines `range` touch, in the order
    /// they appear in the file.
    fn covering(&mut self, path: &RepoPath, range: LineRange) -> Result<Vec<NodeId>, StoreError> {
        if !self.symbols.contains_key(path) {
            let file = self.queries.file(path)?.map(|row| row.id);
            let spans = match file {
                Some(file) => self.queries.symbol_spans_in_file(file)?,
                None => Vec::new(),
            };
            self.symbols.insert(path.clone(), spans);
        }
        let spans = self.symbols.get(path).map_or(&[][..], Vec::as_slice);
        Ok(spans
            .iter()
            .filter(|span| {
                span.start_line <= range.end_line() && range.start_line() <= span.end_line
            })
            .map(|span| span.node)
            .collect())
    }

    /// The node of the folder holding the file at `path`, when that folder has one.
    fn folder_of(&mut self, path: &RepoPath) -> Result<Option<NodeId>, StoreError> {
        if let Some(folder) = self.folders.get(path) {
            return Ok(*folder);
        }
        let folder = match path.parent() {
            Some(folder) => self.queries.folder_node_at(&folder)?,
            None => None,
        };
        self.folders.insert(path.clone(), folder);
        Ok(folder)
    }
}

/// The step that first edited each file the task edited, with the file's node, by path. A file
/// without a node is left out: an increment has nowhere to go.
fn first_edits(state: &TaskState) -> BTreeMap<RepoPath, (NodeId, usize)> {
    let mut first = BTreeMap::new();
    for (at, step) in state.steps().iter().enumerate() {
        if let Step::Edit { file, .. } = step
            && let Some(node) = file.node
        {
            first.entry(file.path.clone()).or_insert((node, at));
        }
    }
    first
}

/// The last step at which the task read or edited each file it read or edited, by path: a file a
/// search found before that step is one the task went on to open.
fn last_opens(state: &TaskState) -> BTreeMap<RepoPath, usize> {
    let mut last = BTreeMap::new();
    for (at, step) in state.steps().iter().enumerate() {
        if let Step::Read { file, .. } | Step::Edit { file, .. } = step {
            last.insert(file.path.clone(), at);
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// How many landmarks a repository's size allows, worked out by hand: one file in a hundred,
    /// rounded up, then held between 5 and 50. An empty repository and a repository of 500 files
    /// alike get the floor of 5; 501 files get 6; 1,000 files get 10; 4,901 files get 50, and so
    /// does every repository above it.
    #[test]
    fn a_repository_s_size_decides_how_many_landmarks_it_can_have() {
        let cases = [
            (0, 5),
            (1, 5),
            (100, 5),
            (500, 5),
            (501, 6),
            (1_000, 10),
            (4_901, 50),
            (5_000, 50),
            (100_000, 50),
            (u64::MAX, 50),
        ];
        for (files, expected) in cases {
            assert_eq!(landmark_count(files), expected, "{files} files");
        }
    }

    /// A score decays by the factor for each task since it was written, worked out by hand: 1.0
    /// written at task 1 and read at task 11 has decayed for 10 tasks, 0.98 to the tenth, which is
    /// 0.8170728068875469. A score read at the task it was written at has not decayed at all, one
    /// read at an earlier task is left as it is, and one left for a million tasks is gone.
    #[test]
    fn a_score_decays_by_the_factor_for_each_task_since() {
        assert!((decayed(1.0, 1, 11, 0.98) - 0.817_072_806_887_546_9).abs() < 1e-12);
        assert!((decayed(2.0, 11, 11, 0.98) - 2.0).abs() < 1e-12);
        assert!((decayed(2.0, 11, 4, 0.98) - 2.0).abs() < 1e-12);
        assert!(decayed(1.0, 1, 1_000_000, 0.98).abs() < 1e-12);
    }

    /// The tier a score falls in on its own: the threshold is a score to stand above, so a score
    /// at it is still fog.
    #[test]
    fn a_score_is_interesting_only_above_the_threshold() {
        assert_eq!(tier_of(0.21), InterestTier::Interesting);
        assert_eq!(tier_of(0.2), InterestTier::Dark);
        assert_eq!(tier_of(0.0), InterestTier::Dark);
    }

    proptest! {
        /// Decay never raises a score, never takes it below zero, and the longer a score has gone
        /// untouched the less of it is left.
        #[test]
        fn decay_only_ever_takes_from_a_score(
            score in 0.0..1e6_f64,
            last_seq in 0..1_000_000_u64,
            tasks in 0..1_000_000_u64,
            more in 0..1_000_000_u64,
            decay in 0.001..=1.0_f64,
        ) {
            let seq = last_seq.saturating_add(tasks);
            let left = decayed(score, last_seq, seq, decay);
            prop_assert!(left >= 0.0);
            prop_assert!(left <= score);
            let later = decayed(score, last_seq, seq.saturating_add(more), decay);
            prop_assert!(later <= left);
        }

        /// However large a repository, the landmarks it allows stay within their bounds.
        #[test]
        fn the_landmarks_allowed_stay_within_their_bounds(files_in_repo in 0..u64::MAX) {
            let count = landmark_count(files_in_repo);
            prop_assert!(count >= LANDMARKS_MIN);
            prop_assert!(count <= LANDMARKS_MAX);
        }
    }
}
