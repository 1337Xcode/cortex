//! One mining of a repository's history into what the store keeps of co-change. The whole window
//! is mined when nothing was mined yet, when the last full mining is a week old (so that recency
//! weights move with the history), or when the commit mined last is no longer on HEAD's
//! first-parent line (the history was rewritten); otherwise only the commits HEAD gained are, and
//! their counts are added to those stored, at weights counted from the new HEAD; when HEAD has not
//! moved, nothing is.
//!
//! [`mine`] reads the history; [`write`] stores what it found inside a transaction of the daemon's
//! single writer.

use std::collections::{HashMap, HashSet};

use cortex_core::{Config, EdgeType, RepoPath};
use cortex_store::{
    CountedFile, EdgeRecord, FileCommitsRow, FileId, GitStateKey, NodeId, PairRow, Queries,
    StoreError, WriteTx,
};

use crate::cochange::{self, CoChange, CoChangeEdge, PairCommits};
use crate::history::{GitError, HistorySource, Window};

/// How long a full mining stands: a week, after which the next mining is a full one again.
pub const FULL_EVERY_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Seconds in a day, for the window's oldest time.
const DAY_S: i64 = 86_400;

/// Where the last mining ended, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// HEAD's commit when it was mined, empty when HEAD's branch had no commit.
    pub last_commit: String,
    /// The sequence number of the newest commit mined.
    pub last_seq: u64,
    /// When the last full mining was written, in UTC milliseconds.
    pub full_at_ms: i64,
}

impl Mark {
    /// The mark the store holds; `None` before the first mining, or when a value cannot be read,
    /// which the next mining, a full one, writes again.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn read(queries: Queries<'_>) -> Result<Option<Self>, StoreError> {
        let Some(last_commit) = queries.git_state(GitStateKey::LastCommit)? else {
            return Ok(None);
        };
        let last_seq = queries
            .git_state(GitStateKey::LastSeq)?
            .and_then(|seq| seq.parse().ok());
        let full_at_ms = queries
            .git_state(GitStateKey::FullAt)?
            .and_then(|at| at.parse().ok());
        Ok(last_seq.zip(full_at_ms).map(|(last_seq, full_at_ms)| Self {
            last_commit,
            last_seq,
            full_at_ms,
        }))
    }
}

/// What a mining found.
#[derive(Debug, Clone, PartialEq)]
pub enum Mining {
    /// HEAD is where the last mining left it, within the week: nothing to write.
    Unchanged,
    /// The whole window, to replace what is stored.
    Full(Harvest),
    /// The commits HEAD gained, to add to what is stored.
    Added(Harvest),
}

/// What a mining counted and where it ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Harvest {
    /// The counts and edges.
    pub cochange: CoChange<FileId>,
    /// HEAD's commit, empty when HEAD's branch has no commit.
    pub head: String,
    /// The sequence number of the newest commit mined.
    pub last_seq: u64,
    /// Whether the history is cut short by a shallow clone.
    pub is_shallow: bool,
    /// Whether the repository is a partial clone.
    pub is_partial: bool,
}

/// Mines `source`'s history, the last mining having ended at `mark`, for the `files` that count,
/// with the window and weights `config` sets, at `now_ms` (UTC milliseconds).
///
/// # Errors
///
/// The history cannot be read.
pub fn mine(
    source: &dyn HistorySource,
    mark: Option<&Mark>,
    files: &[CountedFile],
    config: &Config,
    now_ms: i64,
) -> Result<Mining, GitError> {
    let head = source.head()?.unwrap_or_default();
    let fresh = mark.filter(|mark| now_ms.saturating_sub(mark.full_at_ms) < FULL_EVERY_MS);
    if fresh.is_some_and(|mark| mark.last_commit == head) {
        return Ok(Mining::Unchanged);
    }
    let window = window(config, now_ms, fresh.map(|mark| mark.last_commit.clone()));
    let mined = source.read(&window)?;
    let current: HashMap<RepoPath, FileId> = files
        .iter()
        .map(|file| (file.path.clone(), file.file))
        .collect();
    let half_life = u32::try_from(config.git_half_life_commits).unwrap_or(u32::MAX);
    let read = mined.commits.len() as u64;
    // Commits gained on top of what was mined: numbered on from the newest mined. Anything else,
    // including a read whose stop commit never came (the history was rewritten), is the whole
    // window, numbered from 1.
    let (first_seq, is_added) = match fresh {
        Some(mark) if mined.is_stop_reached => (mark.last_seq + 1, true),
        _ => (1, false),
    };
    let harvest = Harvest {
        cochange: cochange::compute(&mined.commits, &current, half_life, first_seq),
        head,
        last_seq: (first_seq + read).saturating_sub(1),
        is_shallow: mined.is_shallow,
        is_partial: mined.is_partial,
    };
    Ok(if is_added {
        Mining::Added(harvest)
    } else {
        Mining::Full(harvest)
    })
}

/// The window `config` sets at `now_ms`, stopping at `stop_at` when given.
fn window(config: &Config, now_ms: i64, stop_at: Option<String>) -> Window {
    let max_age_s = i64::try_from(config.git_max_age_days)
        .unwrap_or(i64::MAX)
        .saturating_mul(DAY_S);
    Window {
        max_commits: usize::try_from(config.git_max_commits).unwrap_or(usize::MAX),
        oldest_s: (now_ms / 1000).saturating_sub(max_age_s),
        max_files: usize::try_from(config.git_max_files_per_commit).unwrap_or(usize::MAX),
        stop_at,
    }
}

/// Stores `mining` in `transaction` at `now_ms`: a full mining replaces the counts and every
/// co-change edge, commits gained add to the counts and rewrite the edges of the files they
/// changed. Files removed since the mining read them are left out.
///
/// # Errors
///
/// Store errors.
pub fn write(
    transaction: &mut WriteTx<'_>,
    mining: &Mining,
    now_ms: i64,
) -> Result<(), StoreError> {
    let (harvest, is_full) = match mining {
        Mining::Unchanged => return Ok(()),
        Mining::Full(harvest) => (harvest, true),
        Mining::Added(harvest) => (harvest, false),
    };
    let nodes: HashMap<FileId, NodeId> = transaction
        .queries()
        .cochange_files()?
        .into_iter()
        .map(|file| (file.file, file.node))
        .collect();
    let files: Vec<FileCommitsRow> = harvest
        .cochange
        .files
        .iter()
        .filter(|file| nodes.contains_key(&file.file))
        .map(|file| FileCommitsRow {
            file: file.file,
            count: file.count,
            weighted: file.weighted,
        })
        .collect();
    let pairs: Vec<PairRow> = harvest
        .cochange
        .pairs
        .iter()
        .filter(|pair| nodes.contains_key(&pair.file_a) && nodes.contains_key(&pair.file_b))
        .map(|pair| PairRow {
            file_a: pair.file_a,
            file_b: pair.file_b,
            count: pair.count,
            weighted: pair.weighted,
            last_commit_seq: pair.last_commit_seq,
        })
        .collect();
    if is_full {
        transaction.replace_cochange(&files, &pairs)?;
        transaction.remove_edges_of_type(EdgeType::CoChange)?;
        write_edges(transaction, &harvest.cochange.edges, &nodes, None)?;
        transaction.set_git_state(GitStateKey::FullAt, &now_ms.to_string())?;
    } else {
        transaction.add_cochange(&files, &pairs)?;
        let changed: Vec<FileId> = files.iter().map(|file| file.file).collect();
        let edges = edges_of(transaction.queries(), &changed)?;
        let changed: HashSet<FileId> = changed.into_iter().collect();
        write_edges(transaction, &edges, &nodes, Some(&changed))?;
    }
    transaction.set_git_state(GitStateKey::LastCommit, &harvest.head)?;
    transaction.set_git_state(GitStateKey::LastSeq, &harvest.last_seq.to_string())
}

/// The edges from each of `changed`, from the counts the store holds now: every pair of each
/// changed file and the commit counts of every file in those pairs.
fn edges_of(
    queries: Queries<'_>,
    changed: &[FileId],
) -> Result<Vec<CoChangeEdge<FileId>>, StoreError> {
    let pairs: Vec<PairCommits<FileId>> = queries
        .cochange_pairs_of(changed)?
        .into_iter()
        .map(|pair| PairCommits {
            file_a: pair.file_a,
            file_b: pair.file_b,
            count: pair.count,
            weighted: pair.weighted,
            last_commit_seq: pair.last_commit_seq,
        })
        .collect();
    let involved: Vec<FileId> = pairs
        .iter()
        .flat_map(|pair| [pair.file_a, pair.file_b])
        .chain(changed.iter().copied())
        .collect::<HashSet<FileId>>()
        .into_iter()
        .collect();
    let weights: HashMap<FileId, (u32, f64)> = queries
        .file_commits_of(&involved)?
        .into_iter()
        .map(|file| (file.file, (file.count, file.weighted)))
        .collect();
    Ok(cochange::edges(&pairs, &weights))
}

/// Writes `edges` as co-change edges between the files' nodes, replacing those of each file they
/// start at; with `only`, of those files alone, which also lose the edges they no longer have.
fn write_edges(
    transaction: &mut WriteTx<'_>,
    edges: &[CoChangeEdge<FileId>],
    nodes: &HashMap<FileId, NodeId>,
    only: Option<&HashSet<FileId>>,
) -> Result<(), StoreError> {
    let mut from_each: HashMap<FileId, Vec<EdgeRecord>> = HashMap::new();
    if let Some(only) = only {
        for file in only {
            from_each.insert(*file, Vec::new());
        }
    }
    for edge in edges {
        let is_kept = only.is_none_or(|only| only.contains(&edge.from));
        let (Some(_), Some(&dst)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
            continue;
        };
        if is_kept {
            from_each.entry(edge.from).or_default().push(EdgeRecord {
                dst,
                weight: edge.confidence,
                count: edge.co_changes,
                last_seq: 0,
                is_directed: true,
            });
        }
    }
    let mut sources: Vec<(FileId, Vec<EdgeRecord>)> = from_each.into_iter().collect();
    sources.sort_by_key(|(file, _)| *file);
    for (file, records) in sources {
        if let Some(&src) = nodes.get(&file) {
            transaction.replace_edges(src, EdgeType::CoChange, &records)?;
        }
    }
    Ok(())
}
