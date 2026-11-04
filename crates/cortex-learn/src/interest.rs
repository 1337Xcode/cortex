//! What a task's work adds to each node's interest, and which tier each score puts a node in.
//!
//! Interest follows what the task did, an edit counting for most and a file a search found but
//! never opened for least. An edit or a read of some of a file's lines goes to the symbols those
//! lines touch, and a symbol's increment also brightens its file and the folder holding it, so
//! that the map lights up around the work. A span the task's prompt was given earns more when the
//! task went on to read or edit it. Nothing of this reaches an agent yet: interest is the
//! map's brightness and, later, part of what retrieval ranks by.
//!
//! A score is kept as of the task it was last touched at and decays by a factor for each task
//! since, so a node's interest now is its stored score brought forward to the task that asks.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{CommandKind, InterestTier, LineRange, RepoPath};
use cortex_store::{InterestRow, NodeId, Queries, StoreError, WriteTx};

use crate::decay::decayed;
use crate::given::{GivenFile, given_to};
use crate::places::Places;
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

/// What a span the task's prompt was given adds to each symbol it covers, or to its file when it
/// covers none, when the task went on to read or edit some of its lines: the work order pointed at
/// what the task needed.
const GIVEN_SPAN_USED: f64 = 0.3;

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
/// touch, the folders holding the files, and the spans the task's prompt was given.
///
/// # Errors
///
/// The store's errors.
pub fn interest_of(queries: Queries<'_>, state: &TaskState) -> Result<EarnedInterest, StoreError> {
    let mut earned = EarnedInterest::new();
    let mut places = Places::new(queries);
    let (first_edits, last_opens) = (first_edits(state), last_opens(state));
    let mut vouched = BTreeSet::new();
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
            // A check that passed vouches for every file edited before it, each once a task however
            // many times the task edited it and however many checks passed after.
            Step::Run {
                check: Some(check),
                exit_code: Some(0),
                ..
            } if matches!(check.kind, CommandKind::Test | CommandKind::Build) => {
                for (node, edited_at) in first_edits.values() {
                    if *edited_at < at && vouched.insert(*node) {
                        add(&mut earned, *node, PASSED_AFTER_EDIT);
                    }
                }
            }
            Step::Run { .. } => {}
        }
    }
    let given = given_to(queries, state.task_id())?;
    add_given(&mut earned, &mut places, state, &given)?;
    Ok(earned)
}

/// Adds to `earned` what the spans `given` showed earn: [`GIVEN_SPAN_USED`] for each span the task
/// `state` went on to read or edit, once however often it did, credited as a read of its lines is.
fn add_given(
    earned: &mut EarnedInterest,
    places: &mut Places<'_>,
    state: &TaskState,
    given: &[GivenFile],
) -> Result<(), StoreError> {
    for file in given {
        for span in &file.spans {
            if let Some(reached) = reaching(state, file.node, *span) {
                add_within(earned, places, (reached, Some(*span)), GIVEN_SPAN_USED)?;
            }
        }
    }
    Ok(())
}

/// The file of the first step of `state` that read or edited lines `span` of the file `node`, or
/// the whole of it.
fn reaching(state: &TaskState, node: NodeId, span: LineRange) -> Option<&FileNode> {
    state.steps().iter().find_map(|step| match step {
        Step::Read { file, range, .. } | Step::Edit { file, range, .. }
            if file.node == Some(node) && range.is_none_or(|lines| lines.overlaps(span)) =>
        {
            Some(file)
        }
        _ => None,
    })
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

/// The tier `row`'s node stands in at task `seq`, its score brought forward by `decay` for each task
/// since: a landmark as the last task to choose the landmarks left it, since they are chosen again
/// only when a task ends; else interesting while its score is above the fog's; else dark.
#[must_use]
pub fn tier_now(row: &InterestRow, (seq, decay): (u64, f64)) -> InterestTier {
    if row.tier == InterestTier::Landmark {
        return InterestTier::Landmark;
    }
    tier_of(decayed(row.score, row.last_seq, seq, decay))
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

    /// The tier a score falls in on its own: the threshold is a score to stand above, so a score
    /// at it is still fog.
    #[test]
    fn a_score_is_interesting_only_above_the_threshold() {
        assert_eq!(tier_of(0.21), InterestTier::Interesting);
        assert_eq!(tier_of(0.2), InterestTier::Dark);
        assert_eq!(tier_of(0.0), InterestTier::Dark);
    }

    proptest! {
        /// However large a repository, the landmarks it allows stay within their bounds.
        #[test]
        fn the_landmarks_allowed_stay_within_their_bounds(files_in_repo in 0..u64::MAX) {
            let count = landmark_count(files_in_repo);
            prop_assert!(count >= LANDMARKS_MIN);
            prop_assert!(count <= LANDMARKS_MAX);
        }
    }
}
