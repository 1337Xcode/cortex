//! One mining of a repository's history into what the store keeps of co-change. The whole window
//! is mined when nothing was mined yet, when the last full mining is a week old (so that the
//! window moves with the history), or when the commit mined last is no longer on HEAD's
//! first-parent line (the history was rewritten); otherwise only the commits HEAD gained are, and
//! their counts are added to those stored, weighed from the same anchor as the full mining, so
//! that every share is the one a full mining would find. When the commits gained rename a file,
//! whose stored counts are under its old name, or lie too far past the anchor for their weights to
//! be kept, the whole window is mined again instead; when HEAD has not moved, nothing is.
//!
//! [`mine`] reads the history; [`write`] stores what it found with the daemon's single writer, in
//! transactions it lets the writer's other work in between.

use std::collections::{BTreeMap, HashMap, HashSet};

use cortex_core::{Config, EdgeType, RepoPath};
use cortex_store::{
    CountedFile, EdgeRecord, FileCommitsRow, FileId, GitStateKey, NodeId, Pacer, PairRow, Pause,
    Queries, StoreError, WriteTx, Writer,
};

use crate::cochange::{self, CoChange, CoChangeEdge, MAX_GAIN_HALF_LIVES, PairCommits, Weighting};
use crate::history::{GitError, HistorySource, MinedCommit, Window};

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
    /// The sequence number commits are weighed from: the newest of the last full mining.
    pub anchor_seq: u64,
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
        let anchor_seq = queries
            .git_state(GitStateKey::AnchorSeq)?
            .and_then(|seq| seq.parse().ok());
        Ok(match (last_seq, full_at_ms, anchor_seq) {
            (Some(last_seq), Some(full_at_ms), Some(anchor_seq)) => Some(Self {
                last_commit,
                last_seq,
                full_at_ms,
                anchor_seq,
            }),
            _ => None,
        })
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
    /// The sequence number its commits were weighed from.
    pub anchor_seq: u64,
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
    if !is_due(mark, &head, now_ms) {
        return Ok(Mining::Unchanged);
    }
    let fresh = fresh(mark, now_ms);
    // Both reads start at the HEAD resolved above, never at HEAD as it is when they run: a commit
    // made in between is the next mining's, not this one's twice.
    let from = (!head.is_empty()).then(|| head.clone());
    let mined = source.read(&window(
        config,
        now_ms,
        (fresh.map(|mark| mark.last_commit.clone()), from.clone()),
    ))?;
    let half_life_commits = u32::try_from(config.git_half_life_commits).unwrap_or(u32::MAX);
    // Commits gained on top of what was mined are numbered on from the newest mined and weighed
    // from the full mining's anchor, when they can be added. A read whose stop commit never came
    // (the history was rewritten) is already the whole window; gained commits that cannot be added
    // are read again with it.
    let added = fresh
        .filter(|mark| mined.is_stop_reached && can_add(mark, &mined.commits, half_life_commits));
    let (mined, first_seq, anchor_seq) = if let Some(mark) = added {
        (mined, mark.last_seq + 1, mark.anchor_seq)
    } else {
        let whole = if mined.is_stop_reached {
            source.read(&window(config, now_ms, (None, from)))?
        } else {
            mined
        };
        let newest = whole.commits.len() as u64;
        (whole, 1, newest)
    };
    let current: HashMap<RepoPath, FileId> = files
        .iter()
        .map(|file| (file.path.clone(), file.file))
        .collect();
    let weighting = Weighting {
        half_life_commits,
        anchor_seq,
    };
    let harvest = Harvest {
        cochange: cochange::compute(&mined.commits, &current, weighting, first_seq),
        head,
        last_seq: (first_seq + mined.commits.len() as u64).saturating_sub(1),
        anchor_seq,
        is_shallow: mined.is_shallow,
        is_partial: mined.is_partial,
    };
    Ok(if added.is_some() {
        Mining::Added(harvest)
    } else {
        Mining::Full(harvest)
    })
}

/// Whether `gained`, the commits HEAD gained since `mark`, newest first, can be added to the
/// stored counts: none of them renames a file (the stored counts of a renamed file are under its
/// old name), and the newest lies within [`MAX_GAIN_HALF_LIVES`] of the anchor.
fn can_add(mark: &Mark, gained: &[MinedCommit], half_life_commits: u32) -> bool {
    let renames = gained
        .iter()
        .filter_map(|commit| commit.changes.as_ref())
        .flatten()
        .any(|change| change.renamed_from.is_some());
    let newest_seq = mark.last_seq + gained.len() as u64;
    let weighting = Weighting {
        half_life_commits,
        anchor_seq: mark.anchor_seq,
    };
    !renames && weighting.half_lives_after(newest_seq) <= MAX_GAIN_HALF_LIVES
}

/// Whether a mining at `now_ms` would read anything, the last one having ended at `mark` and HEAD
/// being at `head` (empty when its branch has no commit): when nothing was mined yet, when the last
/// full mining is a week old, or when HEAD is not the commit mined last.
#[must_use]
pub fn is_due(mark: Option<&Mark>, head: &str, now_ms: i64) -> bool {
    fresh(mark, now_ms).is_none_or(|mark| mark.last_commit != head)
}

/// `mark` while its full mining is under a week old at `now_ms`.
fn fresh(mark: Option<&Mark>, now_ms: i64) -> Option<&Mark> {
    mark.filter(|mark| now_ms.saturating_sub(mark.full_at_ms) < FULL_EVERY_MS)
}

/// The window `config` sets at `now_ms`, read from `from` and stopping at `stop_at` when given.
fn window(
    config: &Config,
    now_ms: i64,
    (stop_at, from): (Option<String>, Option<String>),
) -> Window {
    let max_age_s = i64::try_from(config.git_max_age_days)
        .unwrap_or(i64::MAX)
        .saturating_mul(DAY_S);
    Window {
        max_commits: usize::try_from(config.git_max_commits).unwrap_or(usize::MAX),
        oldest_s: (now_ms / 1000).saturating_sub(max_age_s),
        max_files: usize::try_from(config.git_max_files_per_commit).unwrap_or(usize::MAX),
        stop_at,
        from,
    }
}

/// Stores `mining` with `writer` at `now_ms` (UTC milliseconds), in transactions cut whenever
/// `pause` is due, letting the caller in between them as an index does. The mark goes in the first
/// transaction and comes back in the last, so that a write cut short (the daemon killed between
/// two, or an error) leaves none, and the next mining is a full one. A full mining replaces the
/// counts and every co-change edge; commits gained add to the counts and rewrite the edges of the
/// files they changed from the sums. Files removed since the mining read them are left out.
///
/// # Errors
///
/// Store errors. One after the first transaction leaves the store with no mark.
pub fn write(
    writer: &mut Writer,
    mining: &Mining,
    now_ms: i64,
    pause: Option<Pause<'_>>,
) -> Result<(), StoreError> {
    let (harvest, is_full) = match mining {
        Mining::Unchanged => return Ok(()),
        Mining::Full(harvest) => (harvest, true),
        Mining::Added(harvest) => (harvest, false),
    };
    let mut pacer = Pacer::new(pause);
    let nodes = begin(writer, is_full)?;
    pacer.let_in(writer);
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
    in_turns(writer, &mut pacer, &files, |transaction, file| {
        transaction.add_cochange(std::slice::from_ref(file), &[])
    })?;
    in_turns(writer, &mut pacer, &pairs, |transaction, pair| {
        transaction.add_cochange(&[], std::slice::from_ref(pair))
    })?;
    let sources = if is_full {
        edges_from(&harvest.cochange.edges, &nodes, None)
    } else {
        let changed: Vec<FileId> = files.iter().map(|file| file.file).collect();
        let edges = edges_of(writer.queries(), &changed)?;
        edges_from(&edges, &nodes, Some(&changed.into_iter().collect()))
    };
    in_turns(
        writer,
        &mut pacer,
        &sources,
        |transaction, (src, records)| transaction.replace_edges(*src, EdgeType::CoChange, records),
    )?;
    finish(writer, harvest, is_full, now_ms)
}

/// Opens a write: removes the mark, and for a full mining every count and co-change edge, in one
/// transaction, and returns the node of each file that counts, read in it.
fn begin(writer: &mut Writer, is_full: bool) -> Result<HashMap<FileId, NodeId>, StoreError> {
    let mut transaction = writer.transaction()?;
    transaction.remove_git_state(GitStateKey::LastCommit)?;
    if is_full {
        transaction.clear_cochange()?;
        transaction.remove_edges_of_type(EdgeType::CoChange)?;
    }
    let nodes = transaction
        .queries()
        .cochange_files()?
        .into_iter()
        .map(|file| (file.file, file.node))
        .collect();
    transaction.commit()?;
    Ok(nodes)
}

/// Closes a write: the mark, and for a full mining its time, in one transaction.
fn finish(
    writer: &mut Writer,
    harvest: &Harvest,
    is_full: bool,
    now_ms: i64,
) -> Result<(), StoreError> {
    let mut transaction = writer.transaction()?;
    if is_full {
        transaction.set_git_state(GitStateKey::FullAt, &now_ms.to_string())?;
        transaction.set_git_state(GitStateKey::AnchorSeq, &harvest.anchor_seq.to_string())?;
    }
    transaction.set_git_state(GitStateKey::LastSeq, &harvest.last_seq.to_string())?;
    transaction.set_git_state(GitStateKey::LastCommit, &harvest.head)?;
    transaction.commit()
}

/// Writes each of `items` with `write_one`, in transactions `pacer` cuts whenever the caller's turn
/// is due. The items are written in the same order however the cuts fall.
fn in_turns<T>(
    writer: &mut Writer,
    pacer: &mut Pacer<'_>,
    items: &[T],
    mut write_one: impl FnMut(&mut WriteTx<'_>, &T) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    let mut transaction = writer.transaction()?;
    for item in items {
        if pacer.is_due() {
            transaction.commit()?;
            pacer.offer(writer);
            transaction = writer.transaction()?;
        }
        write_one(&mut transaction, item)?;
    }
    transaction.commit()
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

/// `edges` as co-change edges between the files' nodes, grouped by the node they start at, in
/// order of file; with `only`, from those files alone, each listed even with no edge left, so
/// that it loses the edges it no longer has.
fn edges_from(
    edges: &[CoChangeEdge<FileId>],
    nodes: &HashMap<FileId, NodeId>,
    only: Option<&HashSet<FileId>>,
) -> Vec<(NodeId, Vec<EdgeRecord>)> {
    let mut from_each: BTreeMap<FileId, Vec<EdgeRecord>> = BTreeMap::new();
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
    from_each
        .into_iter()
        .filter_map(|(file, records)| nodes.get(&file).map(|&src| (src, records)))
        .collect()
}
