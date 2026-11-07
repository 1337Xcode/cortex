//! The recap a session gets after its context is compacted: what its open task was doing, so that
//! the agent goes on without finding it again. It names the files the task edited, the latest
//! first; the files it was warned of and has not read or edited since; and the task's prompt, when
//! the user's settings kept its start. Lines past the cap go from the end, the prompt first. It
//! names no line of a file: an edit's lines are not kept as the file was after it, so they could
//! not be checked against the file as it is now.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use cortex_core::{EventKind, RepoPath, WarningKind, estimate_tokens};
use cortex_store::{EventRow, NodeId, Queries, StoreError, TaskRow, WarningRecord};

use crate::open_warnings::{plain_path, so_far};
use cortex_core::{is_plain, is_taken_out};

/// The most tokens a recap takes, as estimated.
pub const MAX_RECAP_TOKENS: usize = 500;

/// The most edited files a recap names, the latest edited first.
pub const MAX_RECAPPED_FILES: usize = 12;

/// The most open warnings a recap names, the latest first.
pub const MAX_RECAPPED_WARNINGS: usize = 6;

/// The first line of every recap.
const HEADER: &str = "[cortex] Before the compaction, the task in progress:\n";

/// The recap a session gets after a compaction of its open task `task`, as the store `queries`
/// reads holds it; none when the task has nothing to recall.
///
/// # Errors
///
/// The store's errors.
pub fn recap(queries: Queries<'_>, task: &TaskRow) -> Result<Option<String>, StoreError> {
    let so_far = so_far(queries, task.record.task_id)?;
    let mut lines: Vec<String> = Vec::new();
    lines.extend(edited_line(&edited_files(&so_far.events)));
    lines.extend(warned_lines(queries, &so_far.open)?);
    let excerpt = task.record.prompt_excerpt.as_ref();
    if let Some(prompt) = excerpt.and_then(|excerpt| one_line(excerpt.as_str())) {
        lines.push(format!("- began with the prompt \"{prompt}\""));
    }
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(within_cap(&lines))
}

/// The recap's text for as many of `lines` as fit in [`MAX_RECAP_TOKENS`], the first kept longest;
/// none when not even the first fits.
fn within_cap(lines: &[String]) -> Option<String> {
    (1..=lines.len()).rev().find_map(|count| {
        let mut text = String::from(HEADER);
        for line in lines.iter().take(count) {
            let _ = writeln!(text, "{line}");
        }
        // INVARIANT: budgeted (a recap past its cap drops its last lines, or is not given)
        (estimate_tokens(&text) <= MAX_RECAP_TOKENS).then_some(text)
    })
}

/// The files `events`, a task's latest first, edited successfully, the latest edited first, each
/// once; a path that is not plain is not named.
fn edited_files(events: &[EventRow]) -> Vec<&RepoPath> {
    let mut seen: BTreeSet<&RepoPath> = BTreeSet::new();
    events
        .iter()
        .map(|row| &row.record)
        .filter(|event| event.kind == EventKind::Edit && event.meta.is_success != Some(false))
        .filter_map(|event| event.path.as_ref())
        .filter(|path| seen.insert(*path) && is_plain(path.as_str()))
        .collect()
}

/// The line naming `files`, at most [`MAX_RECAPPED_FILES`] of them and as many as fit in the cap
/// beside the header, with how many more there are; none without files.
fn edited_line(files: &[&RepoPath]) -> Option<String> {
    (1..=files.len().min(MAX_RECAPPED_FILES))
        .rev()
        .map(|count| {
            let named: Vec<&str> = files.iter().take(count).map(|path| path.as_str()).collect();
            let more = files.len() - count;
            let mut line = format!("- edited {}", named.join(", "));
            if more > 0 {
                let _ = write!(line, " and {more} more");
            }
            line
        })
        .find(|line| estimate_tokens(&format!("{HEADER}{line}\n")) <= MAX_RECAP_TOKENS)
}

/// A line for each file `open`, the task's open warnings, the latest first, named: the latest
/// warning's, each file once, at most [`MAX_RECAPPED_WARNINGS`]; one whose file or reason cannot be
/// named is left out.
fn warned_lines(queries: Queries<'_>, open: &[WarningRecord]) -> Result<Vec<String>, StoreError> {
    let mut named: BTreeSet<NodeId> = BTreeSet::new();
    let mut lines = Vec::new();
    for warning in open.iter().filter(|warning| named.insert(warning.target)) {
        let Some(target) = plain_path(queries, warning.target)? else {
            continue;
        };
        let why = match warning.kind {
            WarningKind::CallSite => Some("which calls a signature this task changed".to_owned()),
            WarningKind::CoChange => plain_path(queries, warning.source)?
                .map(|source| format!("which usually changes with {source}")),
            WarningKind::Test => {
                plain_path(queries, warning.source)?.map(|source| format!("the tests of {source}"))
            }
        };
        let Some(why) = why else {
            continue;
        };
        lines.push(format!(
            "- was warned of {target}, {why}; not read or edited since"
        ));
        if lines.len() == MAX_RECAPPED_WARNINGS {
            break;
        }
    }
    Ok(lines)
}

/// `text` on one line: each run of line breaks and other spaces one space, what [`is_taken_out`]
/// names taken out, trimmed; none when it holds an agent's reminder tag or nothing is left.
fn one_line(text: &str) -> Option<String> {
    let kept: String = text
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .filter(|character| !is_taken_out(*character))
        .collect();
    let line = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty() && is_plain(&line)).then_some(line)
}
