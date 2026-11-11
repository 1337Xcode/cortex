//! What upkeep asks the history worker to blame ahead of a prompt: the files most likely to be
//! shown, the landmarks of the repository and the files sessions worked on in the last day, so
//! that their first lines' reasons are in the cache when a work order shows them.
//!
//! A file is warmed only when the mined history holds at least [`MIN_COMMITS`] commits of it
//! (fewer have little history to tell, and on a large repository most files have fewer), when the
//! index read its content and lines, and when no blame of its first lines is kept yet. At most
//! [`MAX_WARMED_A_PASS`] files are added to the queue in one pass of upkeep and
//! [`MAX_WARMED_A_DAY`] in a UTC day, a span the worker remembers not to blame passed over, and
//! each from its first line to its [`MAX_BLAME_LINES`]th at most. The worker runs none of them
//! while an index is under way. Files are taken by node, so the same store warms the
//! same files.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::InterestTier;
use cortex_git::blame::MAX_BLAME_LINES;
use cortex_learn::LANDMARK_FROM;
use cortex_store::{FileRow, NodeId, Queries, StoreError};

use super::jobs::Jobs;
use super::queue::BlameJob;

/// The fewest commits of the mined history a file must have to be warmed.
pub(crate) const MIN_COMMITS: u32 = 3;

/// The most files warming asks for in a UTC day: a blame of a file with a long history takes git
/// up to a second, so 200 a day is a few minutes of one core.
pub(crate) const MAX_WARMED_A_DAY: u64 = 200;

/// The most files warming asks for in one pass of upkeep: half of the worker's queue, so that the
/// blames a prompt asks for still find room.
pub(crate) const MAX_WARMED_A_PASS: u64 = 32;

/// How far back a session's work makes a file hot: a day, in milliseconds.
const HOT_WITHIN_MS: i64 = 24 * 60 * 60 * 1_000;

/// Asks `jobs` for blames of the files worth warming that `queries` reads at `now_ms`, in node
/// order, within the limits of a pass and a day, passing over the spans the worker remembers not
/// to blame: how many files it added to the queue, which are the ones the day's limit counts.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn warm(queries: Queries<'_>, jobs: &Jobs, now_ms: i64) -> Result<u64, StoreError> {
    let room = jobs
        .warming_room(now_ms, MAX_WARMED_A_DAY)
        .min(MAX_WARMED_A_PASS);
    if room == 0 {
        return Ok(0);
    }
    let mut nodes: BTreeSet<NodeId> = queries
        .file_interest_from(LANDMARK_FROM)?
        .into_iter()
        .filter(|row| row.tier == InterestTier::Landmark)
        .map(|row| row.node)
        .collect();
    nodes.extend(
        queries
            .positions_since(now_ms.saturating_sub(HOT_WITHIN_MS))?
            .into_iter()
            .map(|row| row.record.node),
    );
    let nodes: Vec<NodeId> = nodes.into_iter().collect();
    let rows = queries.files_of_nodes(&nodes)?;
    let ids: Vec<_> = rows.iter().map(|(_, row)| row.id).collect();
    let commits: BTreeMap<_, u32> = queries
        .file_commits_of(&ids)?
        .into_iter()
        .map(|row| (row.file, row.count))
        .collect();
    let mut queued = 0;
    for (_, row) in rows {
        if queued >= room {
            break;
        }
        let has_history = commits
            .get(&row.id)
            .is_some_and(|count| *count >= MIN_COMMITS);
        if !has_history {
            continue;
        }
        let Some(job) = first_lines(queries, &row)? else {
            continue;
        };
        if !jobs.is_remembered(&job) && jobs.ask_blame(job) {
            queued += 1;
        }
    }
    jobs.count_warmed(now_ms, queued);
    Ok(queued)
}

/// The blame of the first lines of `row`'s file, up to [`MAX_BLAME_LINES`]; none when the index
/// did not read its content or lines, or a blame of those lines is kept already.
fn first_lines(queries: Queries<'_>, row: &FileRow) -> Result<Option<BlameJob>, StoreError> {
    let (Some(file_hash), Some(lines)) = (row.file.content_hash, row.file.line_count) else {
        return Ok(None);
    };
    let last_line = u32::try_from(lines)
        .unwrap_or(u32::MAX)
        .min(MAX_BLAME_LINES);
    if last_line == 0 {
        return Ok(None);
    }
    let is_kept = queries
        .cached_blames((&row.file.path, &file_hash), 1, last_line)?
        .iter()
        .any(|kept| kept.first_line == 1 && kept.last_line >= last_line);
    Ok((!is_kept).then(|| BlameJob {
        path: row.file.path.clone(),
        file_hash,
        first_line: 1,
        last_line,
    }))
}
