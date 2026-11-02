//! Co-change statistics on small histories worked out by hand: the expected counts, weights and
//! confidences are in the comment above each test. With a half-life of 1 commit, the commits
//! weigh 1, 0.5, 0.25 and so on from the newest, sums a computer adds exactly.

use std::collections::HashMap;

use cortex_core::{PathError, RepoPath};
use cortex_git::cochange::{CoChange, CoChangeEdge, FileCommits, PairCommits, compute};
use cortex_git::{MinedCommit, PathChange};
use proptest::prelude::*;

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A commit changing `paths`, each `"new<old"` a rename from `old`; ids and times do not count.
fn commit(paths: &[&str]) -> TestResult<MinedCommit> {
    let changes = paths
        .iter()
        .map(|path| {
            Ok(match path.split_once('<') {
                Some((path, from)) => PathChange {
                    path: RepoPath::new(path)?,
                    renamed_from: Some(RepoPath::new(from)?),
                },
                None => PathChange {
                    path: RepoPath::new(path)?,
                    renamed_from: None,
                },
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    Ok(MinedCommit {
        id: String::new(),
        time_s: 0,
        changes: Some(changes),
    })
}

/// A mass commit, read without its files.
fn mass() -> MinedCommit {
    MinedCommit {
        id: String::new(),
        time_s: 0,
        changes: None,
    }
}

/// The files that count, by current path, numbered from 1 in the order given.
fn files(paths: &[&str]) -> TestResult<HashMap<RepoPath, u32>> {
    paths
        .iter()
        .zip(1..)
        .map(|(path, id)| Ok((RepoPath::new(path)?, id)))
        .collect()
}

/// Whether `actual` is `expected` up to a rounding error.
fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-12
}

/// Three commits change `a`, the two newest also `b`, half-life 1:
/// - `a`: 3 commits, weighted 1 + 0.5 + 0.25 = 1.75; `b`: 2, weighted 1 + 0.5 = 1.5.
/// - the pair: 2 co-changes, weighted 1.5, the newest at sequence 12 (the oldest is 10).
/// - `a` to `b`: 1.5 / 1.75 = 6/7; `b` to `a`: 1.5 / 1.5 = 1.
#[test]
fn counts_weights_and_confidence() -> TestResult {
    let commits = [commit(&["a", "b"])?, commit(&["a", "b"])?, commit(&["a"])?];
    let found = compute(&commits, &files(&["a", "b"])?, 1, 10);
    let expected = CoChange {
        files: vec![
            FileCommits {
                file: 1,
                count: 3,
                weighted: 1.75,
            },
            FileCommits {
                file: 2,
                count: 2,
                weighted: 1.5,
            },
        ],
        pairs: vec![PairCommits {
            file_a: 1,
            file_b: 2,
            count: 2,
            weighted: 1.5,
            last_commit_seq: 12,
        }],
        edges: vec![
            CoChangeEdge {
                from: 1,
                to: 2,
                confidence: 1.5 / 1.75,
                co_changes: 2,
            },
            CoChangeEdge {
                from: 2,
                to: 1,
                confidence: 1.0,
                co_changes: 2,
            },
        ],
    };
    assert_eq!(found, expected);
    assert!(
        found
            .edges
            .first()
            .is_some_and(|edge| near(edge.confidence, 6.0 / 7.0))
    );
    Ok(())
}

/// A file renamed counts its history under its current name. Newest first: `new<old` with `x`,
/// then `old` with `x`, then `old` alone; `old` is `new` now. Half-life 1:
/// - `new`: 3 commits, weighted 1.75; `x`: 2, weighted 1.5; the pair: 2, weighted 1.5.
#[test]
fn a_renamed_file_keeps_its_history() -> TestResult {
    let commits = [
        commit(&["new<old", "x"])?,
        commit(&["old", "x"])?,
        commit(&["old"])?,
    ];
    let found = compute(&commits, &files(&["new", "x"])?, 1, 0);
    assert_eq!(
        found.files,
        vec![
            FileCommits {
                file: 1,
                count: 3,
                weighted: 1.75,
            },
            FileCommits {
                file: 2,
                count: 2,
                weighted: 1.5,
            },
        ]
    );
    let pairs: Vec<(u32, u32, u32, f64)> = found
        .pairs
        .iter()
        .map(|pair| (pair.file_a, pair.file_b, pair.count, pair.weighted))
        .collect();
    assert_eq!(pairs, [(1, 2, 2, 1.5)]);
    Ok(())
}

/// Renames within one commit do not run into each other, and a path taken again later is a new
/// file. Newest first: `c` added anew; `a<b` and `b<c` in one commit (`b` became `a` and `c`
/// became `b`); `b` and `c` together, which are `a` and `b` now. Files now: `a` 1, `b` 2, `c` 3.
/// - 3: 1 commit; 1 and 2: 2 commits each; the pair (1, 2): 2 co-changes. Had the renames been
///   noted one by one in path order, the old `c` would follow `b<c` and then `a<b` to `a`, the
///   oldest commit would hold `a` alone, and the pair would have 1 co-change.
#[test]
fn renames_in_one_commit_and_reused_paths() -> TestResult {
    let commits = [
        commit(&["c"])?,
        commit(&["a<b", "b<c"])?,
        commit(&["b", "c"])?,
    ];
    let found = compute(&commits, &files(&["a", "b", "c"])?, 1, 0);
    let counts: Vec<(u32, u32)> = found
        .files
        .iter()
        .map(|file| (file.file, file.count))
        .collect();
    assert_eq!(counts, [(1, 2), (2, 2), (3, 1)]);
    let pairs: Vec<(u32, u32, u32)> = found
        .pairs
        .iter()
        .map(|pair| (pair.file_a, pair.file_b, pair.count))
        .collect();
    assert_eq!(pairs, [(1, 2, 2)]);
    Ok(())
}

/// A file counts once in a commit even when two of its changes lead to it: newest first, `y<x`,
/// then `x` and `y` together, both `y` now. So `y` has 2 commits and no pair, not 3 commits and a
/// pair with itself.
#[test]
fn a_file_counts_once_a_commit() -> TestResult {
    let commits = [commit(&["y<x"])?, commit(&["x", "y"])?];
    let found = compute(&commits, &files(&["y"])?, 1, 0);
    let counts: Vec<(u32, u32)> = found
        .files
        .iter()
        .map(|file| (file.file, file.count))
        .collect();
    assert_eq!(counts, [(1, 2)]);
    assert!(found.pairs.is_empty());
    Ok(())
}

/// Only the files given count: `gen`, generated, and `gone`, deleted, are not among them, so the
/// commits that changed them with `a` give `a` its commits and nothing else.
#[test]
fn files_not_given_do_not_count() -> TestResult {
    let commits = [commit(&["a", "gen"])?, commit(&["a", "gone"])?];
    let found = compute(&commits, &files(&["a"])?, 1, 0);
    assert_eq!(
        found.files,
        vec![FileCommits {
            file: 1,
            count: 2,
            weighted: 1.5,
        }]
    );
    assert!(found.pairs.is_empty());
    assert!(found.edges.is_empty());
    Ok(())
}

/// A mass commit adds nothing but counts towards the age of older ones: newest first, a mass
/// commit, then `a` with `b` twice, at ages 1 and 2, half-life 1: weights 0.5 and 0.25, so `a`,
/// `b` and the pair each weigh 0.75.
#[test]
fn a_mass_commit_ages_the_older_ones() -> TestResult {
    let commits = [mass(), commit(&["a", "b"])?, commit(&["a", "b"])?];
    let found = compute(&commits, &files(&["a", "b"])?, 1, 0);
    let weighted: Vec<f64> = found.files.iter().map(|file| file.weighted).collect();
    assert_eq!(weighted, [0.75, 0.75]);
    assert_eq!(found.pairs.first().map(|pair| pair.weighted), Some(0.75));
    Ok(())
}

/// An edge needs 2 co-changes and a confidence of 0.1. Half-life 1:
/// - `p` with `q` once: confidence 1 both ways, but 1 co-change, so no edge.
/// - `a` in 5 commits, newest first: alone, with `c`, with `c`, alone, with `b`. `a` weighs
///   1 + 0.5 + 0.25 + 0.125 + 0.0625 = 1.9375; `a` with `c` 0.5 + 0.25 = 0.75, so `a` to `c` is
///   0.75 / 1.9375 and `c` to `a` 1; `b`: 1 co-change only.
/// - `d` in 8 commits, with `e` in the two oldest (2^-6 + 2^-7): `d` weighs 1.9921875, so `d` to
///   `e` is about 0.012, under 0.1, while `e` to `d` is 1.
#[test]
fn an_edge_needs_two_co_changes_and_a_tenth() -> TestResult {
    let pq = compute(&[commit(&["p", "q"])?], &files(&["p", "q"])?, 1, 0);
    assert_eq!(pq.pairs.len(), 1);
    assert!(pq.edges.is_empty());

    let commits = [
        commit(&["a"])?,
        commit(&["a", "c"])?,
        commit(&["a", "c"])?,
        commit(&["a"])?,
        commit(&["a", "b"])?,
    ];
    let abc = compute(&commits, &files(&["a", "b", "c"])?, 1, 0);
    let edges: Vec<(u32, u32)> = abc.edges.iter().map(|edge| (edge.from, edge.to)).collect();
    assert_eq!(edges, [(1, 3), (3, 1)]);
    assert!(
        abc.edges
            .first()
            .is_some_and(|edge| near(edge.confidence, 0.75 / 1.9375))
    );

    let mut commits = vec![commit(&["d"])?; 6];
    commits.extend([commit(&["d", "e"])?, commit(&["d", "e"])?]);
    let de = compute(&commits, &files(&["d", "e"])?, 1, 0);
    let edges: Vec<(u32, u32, f64)> = de
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to, edge.confidence))
        .collect();
    assert_eq!(edges, [(2, 1, 1.0)]);
    Ok(())
}

/// A file keeps its 32 best partners, ranked by confidence times `ln(1 + co-changes)`; partners
/// ranked alike go by id. Newest first: `hub` with `p33`; then twice `hub` with all of `p1` to
/// `p33`. Half-life 1: `hub` weighs 1.75; `p33` co-changed 3 times, confidence 1, rank ln 4, about
/// 1.39; each other partner 2 times, weighted 0.75, confidence 3/7, rank 3/7 times ln 3, about
/// 0.47. So `p33` stays, then `p1` to `p31`, and `p32` is left out. `hub` is file 1 and `pN` file
/// N + 1.
#[test]
fn a_file_keeps_its_best_partners() -> TestResult {
    let partners: Vec<String> = (1..=33).map(|index| format!("p{index}")).collect();
    let mut every: Vec<&str> = vec!["hub"];
    every.extend(partners.iter().map(String::as_str));
    let commits = [commit(&["hub", "p33"])?, commit(&every)?, commit(&every)?];
    let found = compute(&commits, &files(&every)?, 1, 0);
    let from_hub: Vec<u32> = found
        .edges
        .iter()
        .filter(|edge| edge.from == 1)
        .map(|edge| edge.to)
        .collect();
    let mut expected: Vec<u32> = (2..=32).collect();
    expected.push(34);
    assert_eq!(from_hub, expected);
    Ok(())
}

/// Partners are ranked by confidence times `ln(1 + co-changes)`, not by confidence alone. Half-life
/// 4, so a commit at age `k` weighs `2^(-k/4)`: 1, 0.8409, 0.7071, 0.5946, 0.5, in all 3.6426.
/// Newest first, five commits of `hub` with 31 fillers `f1` to `f31`; `a` joins the two newest,
/// `b` the three oldest. Confidence: each filler 1; `a` 1.8409 / 3.6426 = 0.5054; `b` 1.8017 /
/// 3.6426 = 0.4946. Rank: fillers ln 6 = 1.79; `a` 0.5054 * ln 3 = 0.555; `b` 0.4946 * ln 4 =
/// 0.686. With 33 partners and room for 32, `a` is left out; by confidence alone `b` would be.
#[test]
fn partners_are_ranked_by_confidence_and_count() -> TestResult {
    let fillers: Vec<String> = (1..=31).map(|index| format!("f{index}")).collect();
    let with = |extra: &str| -> Vec<String> {
        let mut paths = vec!["hub".to_owned(), extra.to_owned()];
        paths.extend(fillers.iter().cloned());
        paths
    };
    let mut commits = Vec::new();
    for extra in ["a", "a", "b", "b", "b"] {
        let paths = with(extra);
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        commits.push(commit(&paths)?);
    }
    let mut every = vec!["hub", "a", "b"];
    every.extend(fillers.iter().map(String::as_str));
    let found = compute(&commits, &files(&every)?, 4, 0);
    let from_hub: Vec<u32> = found
        .edges
        .iter()
        .filter(|edge| edge.from == 1)
        .map(|edge| edge.to)
        .collect();
    let mut expected = vec![3];
    expected.extend(4..=34);
    assert_eq!(from_hub, expected);
    Ok(())
}

/// A history as the property draws it: per commit, none for a mass commit, or its changes, each
/// the number of a file `f0` to `f11` and maybe the number of the file it was renamed from.
type History = Vec<Option<Vec<(u8, Option<u8>)>>>;

/// A history of up to 40 commits, each a mass commit or up to 5 changes among files `f0` to
/// `f11`, each change possibly a rename from one of them.
fn histories() -> impl Strategy<Value = History> {
    prop::collection::vec(
        prop::option::of(prop::collection::vec(
            (0u8..12, prop::option::of(0u8..12)),
            0..6,
        )),
        0..40,
    )
}

/// The commits `history` describes, from file names `f0` to `f11`.
fn commits_of(history: &History) -> Result<Vec<MinedCommit>, PathError> {
    let name = |index: u8| RepoPath::new(&format!("f{index}"));
    history
        .iter()
        .map(|entry| {
            let changes = match entry {
                None => None,
                Some(changes) => Some(
                    changes
                        .iter()
                        .map(|&(path, from)| {
                            Ok(PathChange {
                                path: name(path)?,
                                renamed_from: from.map(name).transpose()?,
                            })
                        })
                        .collect::<Result<Vec<_>, PathError>>()?,
                ),
            };
            Ok(MinedCommit {
                id: String::new(),
                time_s: 0,
                changes,
            })
        })
        .collect()
}

proptest! {
    /// On any history, with files `f0` to `f7` counting: the same input gives the same output; a
    /// pair's counts never pass either file's; every edge has 2 co-changes and a confidence from
    /// 0.1 to 1; no file starts more than 32 edges; files, pairs and edges are in order.
    #[test]
    fn any_history_keeps_the_rules(history in histories(), half_life in 1u32..50) {
        let commits = commits_of(&history)?;
        let current: HashMap<RepoPath, u32> = (0u8..8)
            .map(|index| Ok((RepoPath::new(&format!("f{index}"))?, u32::from(index))))
            .collect::<Result<_, PathError>>()?;
        let found = compute(&commits, &current, half_life, 1);
        prop_assert_eq!(&found, &compute(&commits, &current, half_life, 1));
        let by_file: HashMap<u32, (u32, f64)> = found
            .files
            .iter()
            .map(|file| (file.file, (file.count, file.weighted)))
            .collect();
        for pair in &found.pairs {
            prop_assert!(pair.file_a < pair.file_b);
            for file in [pair.file_a, pair.file_b] {
                let (count, weighted) = by_file.get(&file).copied().unwrap_or_default();
                prop_assert!(pair.count <= count);
                prop_assert!(pair.weighted <= weighted + 1e-12);
            }
        }
        let mut starts: HashMap<u32, usize> = HashMap::new();
        for edge in &found.edges {
            prop_assert!((0.1..=1.0).contains(&edge.confidence));
            let co_changes = found
                .pairs
                .iter()
                .find(|pair| {
                    (pair.file_a, pair.file_b) == (edge.from.min(edge.to), edge.from.max(edge.to))
                })
                .map_or(0, |pair| pair.count);
            prop_assert!(co_changes >= 2);
            *starts.entry(edge.from).or_default() += 1;
        }
        prop_assert!(starts.values().all(|count| *count <= 32));
        let files_in_order = found
            .files
            .windows(2)
            .all(|two| two.first().map(|a| a.file) < two.last().map(|b| b.file));
        prop_assert!(files_in_order);
        let edges_in_order = found
            .edges
            .windows(2)
            .all(|two| two.first().map(|a| (a.from, a.to)) < two.last().map(|b| (b.from, b.to)));
        prop_assert!(edges_in_order);
    }
}
