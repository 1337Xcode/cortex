//! The hint a read gets before it runs: one line, added beside the file it reads, never changing
//! the read. A range the session read since its context was last compacted, from a file that has
//! not changed since, is said to be read already; otherwise, in a long file, the parts the task's work order pointed at are
//! named by their lines. Every line number is checked against the file as it is on disk.

use std::path::Path;

use cortex_core::{EventKind, Identifier, LineRange, RepoPath, ShortHash, Ulid, estimate_tokens};
use cortex_store::{NodeId, Queries, StoreError};

use crate::disk::CheckedFile;
use crate::plain::is_plain;

/// The most tokens a read hint takes, as estimated.
pub const MAX_HINT_TOKENS: usize = 120;

/// How many lines a file has at most for its relevant parts to go unnamed: a shorter file is read
/// whole for little more than a hint would save.
pub const HINTED_ABOVE_LINES: u32 = 600;

/// The most parts of a file a hint names, the strongest first.
pub const MAX_HINTED_PARTS: usize = 3;

/// The most of a session's reads of one file and its compactions looked at, the latest first.
const MAX_READS_LOOKED_AT: usize = 64;

/// What a read hint is worked out for.
#[derive(Debug, Clone, Copy)]
pub struct HintQuery<'a> {
    /// The repository's root, where the file is read from.
    pub root: &'a Path,
    /// The file about to be read.
    pub path: &'a RepoPath,
    /// The lines about to be read; none for the whole file.
    pub asked: Option<LineRange>,
    /// The session reading it.
    pub session: &'a Identifier,
    /// The session's open task, if any.
    pub task: Option<Ulid>,
    /// `index.max_file_bytes`: the most bytes the file may hold to be read here.
    pub max_file_bytes: u64,
}

/// The hint the read `asked` describes gets, as the store `queries` reads holds what came before
/// it; none when the store does not hold the file as it is on disk, or there is nothing to say.
///
/// # Errors
///
/// The store's errors.
pub fn read_hint(
    queries: Queries<'_>,
    asked: &HintQuery<'_>,
) -> Result<Option<String>, StoreError> {
    let Some(row) = queries.file(asked.path)? else {
        return Ok(None);
    };
    let Some(stored) = row.file.content_hash else {
        return Ok(None);
    };
    if !is_plain(asked.path.as_str()) {
        return Ok(None);
    }
    // INVARIANT: true-at-render (the lines a hint names are the file's on disk: it is the file the
    // index read, or no hint is given)
    let Ok(file) = CheckedFile::read(asked.root, asked.path, &stored, asked.max_file_bytes) else {
        return Ok(None);
    };
    let Ok(whole) = LineRange::new(1, file.line_count()) else {
        return Ok(None);
    };
    let Some(node) = queries.file_node(row.id)? else {
        return Ok(None);
    };
    let wanted = asked.asked.unwrap_or(whole);
    if let Some(earlier) = read_before(queries, asked.session, (node, stored.short()), wanted)? {
        let (start, end) = (
            earlier.start_line(),
            earlier.end_line().min(whole.end_line()),
        );
        let text = format!(
            "[cortex] You read lines {start}-{end} of this file earlier in this session; \
             unchanged since.\n"
        );
        return Ok(within_cap(text));
    }
    let Some(task) = asked.task.filter(|_| whole.end_line() > HINTED_ABOVE_LINES) else {
        return Ok(None);
    };
    let parts = parts_given(queries, (task, node, row.id), whole)?;
    Ok(relevant_parts(asked.path, &parts))
}

/// The range of one of `session`'s reads of the file whose node and short hash are `node` and
/// `hash` that holds all of `wanted`: a read that succeeded, of the file as it is now, since the
/// session's latest compaction, which took what came before it out of the agent's context.
fn read_before(
    queries: Queries<'_>,
    session: &Identifier,
    (node, hash): (NodeId, ShortHash),
    wanted: LineRange,
) -> Result<Option<LineRange>, StoreError> {
    let reads = queries.reads_in_session(session, node, MAX_READS_LOOKED_AT)?;
    let mut since_compacted = reads
        .iter()
        .take_while(|row| row.record.kind == EventKind::Read);
    Ok(since_compacted.find_map(|row| {
        let event = &row.record;
        let is_same = event.meta.is_success != Some(false) && event.meta.content == Some(hash);
        // A read of the whole file is kept with no range: it holds every line.
        let holds = event.range.is_none_or(|range| {
            range.start_line() <= wanted.start_line() && wanted.end_line() <= range.end_line()
        });
        (is_same && holds).then(|| event.range.unwrap_or(wanted))
    }))
}

/// The parts of the file of `file` (its node `node`) that task `task`'s work orders given showed or
/// listed, the strongest first, each with the name of the symbol it spans when there is one: at
/// most [`MAX_HINTED_PARTS`], each once, each inside `whole`.
fn parts_given(
    queries: Queries<'_>,
    (task, node, file): (Ulid, NodeId, cortex_store::FileId),
    whole: LineRange,
) -> Result<Vec<(LineRange, Option<String>)>, StoreError> {
    let names: Vec<(u32, u32, String)> = queries
        .symbol_spans_in_file(file)?
        .into_iter()
        .map(|span| {
            (
                span.start_line,
                span.end_line,
                span.name.as_str().to_owned(),
            )
        })
        .collect();
    let mut parts: Vec<(LineRange, Option<String>)> = Vec::new();
    for injection in queries.injections_of_task(task)? {
        if !injection.record.was_delivered() {
            continue;
        }
        let spans = injection
            .record
            .files
            .iter()
            .filter(|given| given.node == node)
            .flat_map(|given| given.spans.iter().copied());
        for (start, end) in spans {
            let Ok(range) = LineRange::new(start, end) else {
                continue;
            };
            if range.end_line() > whole.end_line() || parts.iter().any(|(kept, _)| *kept == range) {
                continue;
            }
            let name = names
                .iter()
                .find(|(first, last, _)| (*first, *last) == (start, end))
                .map(|(_, _, name)| name.clone())
                .filter(|name| is_plain(name));
            parts.push((range, name));
        }
    }
    parts.truncate(MAX_HINTED_PARTS);
    Ok(parts)
}

/// The hint naming `parts` of the file at `path`, as many of them as fit in [`MAX_HINT_TOKENS`];
/// none without parts.
fn relevant_parts(path: &RepoPath, parts: &[(LineRange, Option<String>)]) -> Option<String> {
    (1..=parts.len()).rev().find_map(|count| {
        let named: Vec<String> = parts
            .iter()
            .take(count)
            .map(|(range, name)| {
                let lines = format!("{}-{}", range.start_line(), range.end_line());
                match name {
                    Some(name) => format!("{lines} ({name})"),
                    None => lines,
                }
            })
            .collect();
        let listed = match named.as_slice() {
            [one] => format!("part of {path} is lines {one}"),
            [first @ .., last] => {
                format!("parts of {path} are lines {} and {last}", first.join(", "))
            }
            [] => return None,
        };
        within_cap(format!("[cortex] For this task the relevant {listed}.\n"))
    })
}

/// `text` when it is within [`MAX_HINT_TOKENS`].
fn within_cap(text: String) -> Option<String> {
    // INVARIANT: budgeted (a hint past its cap is not given)
    (estimate_tokens(&text) <= MAX_HINT_TOKENS).then_some(text)
}
