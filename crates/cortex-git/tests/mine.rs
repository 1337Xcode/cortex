//! Mining into a real store: a repository made with git, its files indexed as the indexer would
//! store them, then minings as the daemon runs them: the first, commits added, HEAD unmoved, a
//! rewritten history, a week passed, a file removed before the write, a write cut into turns. With
//! a half-life of 1 commit, commits weigh 1, 0.5, 0.25 and so on from the newest; each test works
//! its numbers out in its comment.

mod common;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::Duration;

use common::{DAY_0_S, DAY_S, Repo, TestResult, lines};
use cortex_core::{Config, EdgeType, FileFlags, RepoId, RepoPath};
use cortex_git::GixHistory;
use cortex_git::mine::{FULL_EVERY_MS, Mark, Mining, mine, write};
use cortex_store::{
    FileCommitsRow, FileId, FileIndex, FileRecord, GitStateKey, NodeId, PairRow, Pause, Writer,
    WriterOptions,
};

/// The memory map the store's tests use, 256 MiB.
const MMAP_BYTES: u64 = 256 * 1024 * 1024;

/// A store for the repository at `root`, in its `.cortex` folder, opened at `now_ms`.
fn store(root: &Path, now_ms: i64) -> TestResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: MMAP_BYTES,
        now_ms,
    })?)
}

/// Stores the files at `paths` as the indexer would, with no language, and returns their ids and
/// nodes in that order.
fn index(writer: &mut Writer, paths: &[&str]) -> TestResult<Vec<(FileId, NodeId)>> {
    let mut transaction = writer.transaction()?;
    for path in paths {
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 10,
                line_count: None,
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
    }
    let counted = transaction.queries().cochange_files()?;
    let ids = paths
        .iter()
        .map(|path| {
            counted
                .iter()
                .find(|file| file.path.as_str() == *path)
                .map(|file| (file.file, file.node))
                .ok_or_else(|| format!("{path} is not stored").into())
        })
        .collect::<TestResult<Vec<_>>>()?;
    transaction.commit()?;
    Ok(ids)
}

/// Mines the repository at `root` into `writer` at `now_ms`, half-life 1, as the daemon does, with
/// `pause` between the write's transactions, and returns what was mined.
fn mine_with(
    root: &Path,
    writer: &mut Writer,
    now_ms: i64,
    pause: Option<Pause<'_>>,
) -> TestResult<Mining> {
    let config = Config {
        git_half_life_commits: 1,
        ..Config::default()
    };
    let mark = Mark::read(writer.queries())?;
    let files = writer.queries().cochange_files()?;
    let mining = mine(
        &GixHistory::open(root)?,
        mark.as_ref(),
        &files,
        &config,
        now_ms,
    )?;
    write(writer, &mining, now_ms, pause)?;
    Ok(mining)
}

/// Mines the repository at `root` into `writer` at `now_ms`, as [`mine_with`] does without a
/// pause.
fn mine_into(root: &Path, writer: &mut Writer, now_ms: i64) -> TestResult<Mining> {
    mine_with(root, writer, now_ms, None)
}

/// What the store holds of co-change: each file's counts, each pair's, and each co-change edge
/// as (from, to, weight, co-changes), for `files`.
type Stored = (
    Vec<FileCommitsRow>,
    Vec<PairRow>,
    Vec<(NodeId, NodeId, f64, u32)>,
);

/// What the store holds of co-change for `files`.
fn stored(writer: &Writer, files: &[(FileId, NodeId)]) -> TestResult<Stored> {
    let queries = writer.queries();
    let ids: Vec<FileId> = files.iter().map(|(file, _)| *file).collect();
    let mut edges = Vec::new();
    for (_, node) in files {
        for row in queries.edges_from(*node)? {
            if row.edge_type == EdgeType::CoChange {
                edges.push((row.src, row.edge.dst, row.edge.weight, row.edge.count));
            }
        }
    }
    Ok((
        queries.file_commits_of(&ids)?,
        queries.cochange_pairs_of(&ids)?,
        edges,
    ))
}

/// The time `days` after day 0, in milliseconds.
fn at(days: i64) -> i64 {
    (DAY_0_S + days * DAY_S) * 1000
}

/// Commits `paths` on `day`, each with text of that day's own.
fn change(repo: &Repo, day: i64, paths: &[&str]) -> TestResult<String> {
    for path in paths {
        repo.write(path, &lines(&format!("{path} on day {day}"), 3))?;
    }
    repo.commit(day)
}

/// The first mining reads the whole window. Newest first: day 3 `a`; day 2 `a` and `b`; day 1
/// `a` and `b`. `a`: 3 commits, 1 + 0.5 + 0.25 = 1.75; `b`: 2, 0.5 + 0.25 = 0.75; the pair: 2,
/// 0.75, newest at sequence 2 (day 2; the oldest is 1). Edges: `a` to `b` 0.75 / 1.75 = 3/7, `b`
/// to `a` 1. The mark: HEAD, sequence 3, full at the time of the mining.
#[test]
fn the_first_mining_reads_the_whole_window() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    let head = change(&repo, 3, &["a.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt"])?;
    let [(a, a_node), (b, b_node)] = files[..] else {
        return Err("two files".into());
    };

    let mining = mine_into(&repo.root(), &mut writer, at(10))?;
    assert!(matches!(mining, Mining::Full(_)), "{mining:?}");
    let (counts, pairs, edges) = stored(&writer, &files)?;
    assert_eq!(
        counts,
        [
            FileCommitsRow {
                file: a,
                count: 3,
                weighted: 1.75,
            },
            FileCommitsRow {
                file: b,
                count: 2,
                weighted: 0.75,
            },
        ]
    );
    assert_eq!(
        pairs,
        [PairRow {
            file_a: a,
            file_b: b,
            count: 2,
            weighted: 0.75,
            last_commit_seq: 2,
        }]
    );
    assert_eq!(
        edges,
        [(a_node, b_node, 0.75 / 1.75, 2), (b_node, a_node, 1.0, 2)]
    );
    let queries = writer.queries();
    assert_eq!(queries.git_state(GitStateKey::LastCommit)?, Some(head));
    assert_eq!(
        queries.git_state(GitStateKey::LastSeq)?.as_deref(),
        Some("3")
    );
    assert_eq!(
        queries.git_state(GitStateKey::FullAt)?,
        Some(at(10).to_string())
    );
    Ok(())
}

/// Commits HEAD gained are added, weighed from the full mining's anchor; then, HEAD unmoved,
/// nothing is mined. After the first mining of the test above (anchor: sequence 3), day 4 changes
/// `a` and `b` as sequence 4, one half-life past the anchor, weight 2: `a` 4 commits, 1.75 + 2 =
/// 3.75; `b` 3, 0.75 + 2 = 2.75; the pair 3, 2.75, newest at sequence 4. Edges: `a` to `b` 2.75 /
/// 3.75 = 11/15, `b` to `a` 1, the shares a full mining of the four commits finds (weights 1, 0.5,
/// 0.25 and 0.125 from the newest: 1.375 / 1.875 = 11/15), which a mining into a new store is
/// checked against. The full mining's time and anchor stay.
#[test]
fn commits_gained_are_added() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    change(&repo, 3, &["a.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt"])?;
    let [(a, a_node), (b, b_node)] = files[..] else {
        return Err("two files".into());
    };
    mine_into(&repo.root(), &mut writer, at(10))?;
    let head = change(&repo, 4, &["a.txt", "b.txt"])?;

    let mining = mine_into(&repo.root(), &mut writer, at(11))?;
    assert!(matches!(mining, Mining::Added(_)), "{mining:?}");
    let (counts, pairs, edges) = stored(&writer, &files)?;
    assert_eq!(
        counts,
        [
            FileCommitsRow {
                file: a,
                count: 4,
                weighted: 3.75,
            },
            FileCommitsRow {
                file: b,
                count: 3,
                weighted: 2.75,
            },
        ]
    );
    assert_eq!(
        pairs,
        [PairRow {
            file_a: a,
            file_b: b,
            count: 3,
            weighted: 2.75,
            last_commit_seq: 4,
        }]
    );
    assert_eq!(
        edges,
        [(a_node, b_node, 2.75 / 3.75, 3), (b_node, a_node, 1.0, 3)]
    );
    let mut fresh = store(&repo.root().join("fresh"), at(11))?;
    let fresh_files = index(&mut fresh, &["a.txt", "b.txt"])?;
    assert!(matches!(
        mine_into(&repo.root(), &mut fresh, at(11))?,
        Mining::Full(_)
    ));
    let (_, _, full_edges) = stored(&fresh, &fresh_files)?;
    let shares = |found: &[(NodeId, NodeId, f64, u32)]| -> Vec<(f64, u32)> {
        found.iter().map(|edge| (edge.2, edge.3)).collect()
    };
    let (gained, whole) = (shares(&edges), shares(&full_edges));
    assert_eq!(gained.len(), whole.len());
    for ((share, count), (full_share, full_count)) in gained.iter().zip(&whole) {
        assert!(
            (share - full_share).abs() < 1e-12,
            "{share} against {full_share}"
        );
        assert_eq!(count, full_count);
    }
    let queries = writer.queries();
    assert_eq!(queries.git_state(GitStateKey::LastCommit)?, Some(head));
    assert_eq!(
        queries.git_state(GitStateKey::LastSeq)?.as_deref(),
        Some("4")
    );
    assert_eq!(
        queries.git_state(GitStateKey::AnchorSeq)?.as_deref(),
        Some("3")
    );
    assert_eq!(
        queries.git_state(GitStateKey::FullAt)?,
        Some(at(10).to_string())
    );

    assert_eq!(
        mine_into(&repo.root(), &mut writer, at(12))?,
        Mining::Unchanged
    );
    assert_eq!(stored(&writer, &files)?, (counts, pairs, edges));
    Ok(())
}

/// A rewritten history is mined again whole, and what it no longer holds goes. Days 1 and 2
/// change `a` and `b`, days 3 and 4 `a` and `c`; after the first mining HEAD goes back to day 2
/// and day 5 changes `b`, so the commit mined last, day 4, is no longer on the line. Newest
/// first: day 5 `b`; day 2 `a` and `b`; day 1 `a` and `b`. `a`: 2 commits, 0.5 + 0.25 = 0.75;
/// `b`: 3, 1 + 0.5 + 0.25 = 1.75; `c` none, so no counts and no edges; the pair (`a`, `b`): 2,
/// 0.75. Edges: `a` to `b` 0.75 / 0.75 = 1, `b` to `a` 0.75 / 1.75 = 3/7. Sequence 3, full at the
/// time of this mining.
#[test]
fn a_rewritten_history_is_mined_again_whole() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    let day_2 = change(&repo, 2, &["a.txt", "b.txt"])?;
    change(&repo, 3, &["a.txt", "c.txt"])?;
    change(&repo, 4, &["a.txt", "c.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt", "c.txt"])?;
    let [(a, a_node), (b, b_node), (_, c_node)] = files[..] else {
        return Err("three files".into());
    };
    mine_into(&repo.root(), &mut writer, at(10))?;
    assert!(!writer.queries().edges_from(c_node)?.is_empty());
    repo.git(&["reset", "-q", "--hard", &day_2])?;
    change(&repo, 5, &["b.txt"])?;

    let mining = mine_into(&repo.root(), &mut writer, at(11))?;
    assert!(matches!(mining, Mining::Full(_)), "{mining:?}");
    let (counts, pairs, edges) = stored(&writer, &files)?;
    assert_eq!(
        counts,
        [
            FileCommitsRow {
                file: a,
                count: 2,
                weighted: 0.75,
            },
            FileCommitsRow {
                file: b,
                count: 3,
                weighted: 1.75,
            },
        ]
    );
    assert_eq!(
        pairs
            .iter()
            .map(|pair| (pair.file_a, pair.file_b, pair.count, pair.weighted))
            .collect::<Vec<_>>(),
        [(a, b, 2, 0.75)]
    );
    assert_eq!(
        edges,
        [(a_node, b_node, 1.0, 2), (b_node, a_node, 0.75 / 1.75, 2)]
    );
    let queries = writer.queries();
    assert_eq!(
        queries.git_state(GitStateKey::LastSeq)?.as_deref(),
        Some("3")
    );
    assert_eq!(
        queries.git_state(GitStateKey::FullAt)?,
        Some(at(11).to_string())
    );
    Ok(())
}

/// Commits gained rewrite the edges of the files they changed alone: a partner they did not
/// change keeps its edges, those to files the new commits never touched included. Days 1 and 2
/// change `a`, `c` and `d`; day 3 changes `a` and `b`. After the first mining (of days 1 and 2)
/// and the second (of day 3), `c` still has its edge to `d`.
#[test]
fn the_edges_of_files_not_changed_stay() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "c.txt", "d.txt"])?;
    change(&repo, 2, &["a.txt", "c.txt", "d.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt", "c.txt", "d.txt"])?;
    let [_, _, (_, c_node), (_, d_node)] = files[..] else {
        return Err("four files".into());
    };
    mine_into(&repo.root(), &mut writer, at(10))?;
    let before = writer.queries().edges_from(c_node)?;
    change(&repo, 3, &["a.txt", "b.txt"])?;

    let mining = mine_into(&repo.root(), &mut writer, at(11))?;
    assert!(matches!(mining, Mining::Added(_)), "{mining:?}");
    let after = writer.queries().edges_from(c_node)?;
    assert_eq!(after, before);
    assert!(after.iter().any(|row| row.edge.dst == d_node));
    Ok(())
}

/// A week after a full mining, the next one is whole again, though HEAD has not moved; the day
/// before, it was not.
#[test]
fn a_week_later_the_mining_is_whole_again() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    index(&mut writer, &["a.txt", "b.txt"])?;
    mine_into(&repo.root(), &mut writer, at(10))?;

    let almost = at(10) + FULL_EVERY_MS - 1;
    assert_eq!(
        mine_into(&repo.root(), &mut writer, almost)?,
        Mining::Unchanged
    );
    let week = at(10) + FULL_EVERY_MS;
    let mining = mine_into(&repo.root(), &mut writer, week)?;
    assert!(matches!(mining, Mining::Full(_)), "{mining:?}");
    assert_eq!(
        writer.queries().git_state(GitStateKey::FullAt)?,
        Some(week.to_string())
    );
    Ok(())
}

/// A file removed from the store between the mining and the write is left out of what is written,
/// and the rest is written.
#[test]
fn a_file_removed_before_the_write_is_left_out() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt"])?;
    let [(a, _), _] = files[..] else {
        return Err("two files".into());
    };
    let counted = writer.queries().cochange_files()?;
    let mining = mine(
        &GixHistory::open(&repo.root())?,
        None,
        &counted,
        &Config::default(),
        at(10),
    )?;
    let mut transaction = writer.transaction()?;
    transaction.remove_file(&RepoPath::new("b.txt")?)?;
    transaction.commit()?;
    write(&mut writer, &mining, at(10), None)?;

    let (counts, pairs, edges) = stored(&writer, &files[..1])?;
    assert_eq!(
        counts
            .iter()
            .map(|row| (row.file, row.count))
            .collect::<Vec<_>>(),
        [(a, 2)]
    );
    assert!(pairs.is_empty());
    assert!(edges.is_empty());
    Ok(())
}

/// A write cut into turns, its pause due before every row (a period of zero), stores what the first
/// mining of the first test stores in one turn: `a` 3 commits weighing 1.75, `b` 2 weighing 0.75,
/// the pair 2 weighing 0.75 at sequence 2, edges 3/7 and 1. The caller is let in more than once.
#[test]
fn a_write_in_turns_stores_what_one_turn_does() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    change(&repo, 3, &["a.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    let files = index(&mut writer, &["a.txt", "b.txt"])?;
    let [(a, a_node), (b, b_node)] = files[..] else {
        return Err("two files".into());
    };
    let turns = Cell::new(0);
    let count = |_: &mut Writer| turns.set(turns.get() + 1);
    mine_with(
        &repo.root(),
        &mut writer,
        at(10),
        Some(Pause::new(&count, Duration::ZERO)),
    )?;
    assert!(turns.get() > 1, "{} turns", turns.get());
    let (counts, pairs, edges) = stored(&writer, &files)?;
    assert_eq!(
        counts,
        [
            FileCommitsRow {
                file: a,
                count: 3,
                weighted: 1.75,
            },
            FileCommitsRow {
                file: b,
                count: 2,
                weighted: 0.75,
            },
        ]
    );
    assert_eq!(
        pairs,
        [PairRow {
            file_a: a,
            file_b: b,
            count: 2,
            weighted: 0.75,
            last_commit_seq: 2,
        }]
    );
    assert_eq!(
        edges,
        [(a_node, b_node, 0.75 / 1.75, 2), (b_node, a_node, 1.0, 2)]
    );
    Ok(())
}

/// While the commits HEAD gained are written, every turn the caller gets finds no mark, so that a
/// write cut short leaves the next mining to read the whole window again rather than add the same
/// commits twice; once written, the mark is the new HEAD.
#[test]
fn a_write_under_way_holds_no_mark() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    index(&mut writer, &["a.txt", "b.txt"])?;
    mine_into(&repo.root(), &mut writer, at(10))?;
    let head = change(&repo, 3, &["a.txt", "b.txt"])?;
    let seen = RefCell::new(Vec::new());
    let look = |writer: &mut Writer| {
        seen.borrow_mut()
            .push(writer.queries().git_state(GitStateKey::LastCommit).ok());
    };
    let mining = mine_with(
        &repo.root(),
        &mut writer,
        at(11),
        Some(Pause::new(&look, Duration::ZERO)),
    )?;
    assert!(matches!(mining, Mining::Added(_)), "{mining:?}");
    let seen = seen.into_inner();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|mark| *mark == Some(None)), "{seen:?}");
    assert_eq!(
        writer.queries().git_state(GitStateKey::LastCommit)?,
        Some(head)
    );
    Ok(())
}

/// Commits gained that rename a file are mined again whole: the stored counts of the renamed file
/// are under its old name, which a full mining follows to the new one. Days 1 and 2 change `a` and
/// `b`; after the first mining day 3 renames `b` to `c`, so the whole window is read again, `c`
/// takes `b`'s two co-changes with `a`, and the anchor moves to the new newest commit.
#[test]
fn commits_gained_that_rename_are_mined_whole() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    change(&repo, 2, &["a.txt", "b.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    index(&mut writer, &["a.txt", "b.txt"])?;
    assert!(matches!(
        mine_into(&repo.root(), &mut writer, at(10))?,
        Mining::Full(_)
    ));
    repo.move_file("b.txt", "c.txt", None)?;
    repo.commit(3)?;
    let files = index(&mut writer, &["a.txt", "c.txt"])?;
    let mining = mine_into(&repo.root(), &mut writer, at(11))?;
    assert!(matches!(mining, Mining::Full(_)), "{mining:?}");
    let (_, pairs, _) = stored(&writer, &files)?;
    assert_eq!(pairs.iter().map(|pair| pair.count).collect::<Vec<_>>(), [2]);
    assert_eq!(
        writer
            .queries()
            .git_state(GitStateKey::AnchorSeq)?
            .as_deref(),
        Some("3")
    );
    Ok(())
}

/// Commits gained more than 64 half-lives past the anchor would weigh more than 2^64 times it, so
/// the window is mined again whole from a new anchor. At a half-life of 1 commit: 64 commits gained
/// are added, the 65th past them is not.
#[test]
fn commits_gained_too_far_past_the_anchor_are_mined_whole() -> TestResult {
    let repo = Repo::new()?;
    change(&repo, 1, &["a.txt", "b.txt"])?;
    let mut writer = store(&repo.root(), at(10))?;
    index(&mut writer, &["a.txt", "b.txt"])?;
    mine_into(&repo.root(), &mut writer, at(10))?;
    for commit in 0..64 {
        repo.write("a.txt", &format!("a, commit {commit}\n"))?;
        repo.commit(2)?;
    }
    let added = mine_into(&repo.root(), &mut writer, at(10))?;
    assert!(matches!(added, Mining::Added(_)), "{added:?}");
    repo.write("a.txt", "a, once more\n")?;
    repo.commit(2)?;
    let whole = mine_into(&repo.root(), &mut writer, at(10))?;
    assert!(matches!(whole, Mining::Full(_)), "{whole:?}");
    assert_eq!(
        writer
            .queries()
            .git_state(GitStateKey::AnchorSeq)?
            .as_deref(),
        Some("66")
    );
    Ok(())
}
