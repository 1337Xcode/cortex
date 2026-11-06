//! What the store says about an edited file, read for the warning rules: its co-change partners,
//! the files agents worked on with it, the calls to its symbols whose signature changed, and its
//! tests. A call site is told only from a file whose bytes on disk are the ones the index read, so
//! its line is the line the reference is on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cortex_core::{EdgeType, EventKind, RefKind, RepoPath, Ulid};
use cortex_learn::{MAX_TASK_ROWS, decayed};
use cortex_store::{FileId, NodeId, Queries, StoreError, SymbolId};

use super::{CallSite, Drift, EditFacts, Linked, NamedFile, Partner, TaskSoFar, TestFile};
use crate::disk::CheckedFile;

/// The most files whose calls to one changed symbol are checked on disk; the calls in files past
/// it are counted as not checked, so that a symbol called from everywhere costs an edit no more
/// than this many reads.
pub const MAX_CHECKED_CALLER_FILES: usize = 50;

/// What an edit's facts are read for.
#[derive(Debug, Clone, Copy)]
pub struct EditQuery<'a> {
    /// The repository's root, where call sites are read from.
    pub root: &'a Path,
    /// The edited file.
    pub edited: &'a RepoPath,
    /// The edited file's symbols whose signature the edit changed.
    pub drifted: &'a [SymbolId],
    /// `learning.link_decay`: what a co-access link keeps of its weight for each task.
    pub link_decay: f64,
    /// `index.max_file_bytes`: the most bytes a file read for its call sites may hold.
    pub max_file_bytes: u64,
}

/// The facts of the edit `asked` describes, as the store `queries` reads holds them; none when the
/// store holds no such file.
///
/// # Errors
///
/// The store's errors.
pub fn edit_facts(
    queries: Queries<'_>,
    asked: &EditQuery<'_>,
) -> Result<Option<EditFacts>, StoreError> {
    let Some(row) = queries.file(asked.edited)? else {
        return Ok(None);
    };
    let Some(node) = queries.file_node(row.id)? else {
        return Ok(None);
    };
    let commits = queries
        .file_commits_of(&[row.id])?
        .first()
        .map_or(0, |counted| counted.count);
    let (partners, linked) = companions(queries, node, asked.link_decay)?;
    let mut drifts = Vec::new();
    for symbol in asked.drifted {
        if let Some(drift) = drift(queries, asked, row.id, *symbol)? {
            drifts.push(drift);
        }
    }
    let mut tests = Vec::new();
    for (test, confidence) in queries.test_files_into(node)? {
        if let Some((_, path)) = queries.node_file(test)? {
            tests.push(TestFile {
                file: NamedFile { node: test, path },
                confidence,
            });
        }
    }
    Ok(Some(EditFacts {
        edited: NamedFile {
            node,
            path: asked.edited.clone(),
        },
        commits,
        partners,
        linked,
        drifts,
        tests,
    }))
}

/// The files the edges from `node` join it to by co-change and by co-access, each link faded by
/// `link_decay` for each task since it last changed.
fn companions(
    queries: Queries<'_>,
    node: NodeId,
    link_decay: f64,
) -> Result<(Vec<Partner>, Vec<Linked>), StoreError> {
    let edges: Vec<_> = queries
        .edges_from(node)?
        .into_iter()
        .filter(|row| matches!(row.edge_type, EdgeType::CoChange | EdgeType::CoAccess))
        .collect();
    let targets: Vec<NodeId> = edges.iter().map(|row| row.edge.dst).collect();
    // Only file nodes come back: a co-access link between symbols names no file.
    let files = queries.files_of_nodes(&targets)?;
    let ids: Vec<FileId> = files.iter().map(|(_, file)| file.id).collect();
    let commits_of: BTreeMap<FileId, u32> = queries
        .file_commits_of(&ids)?
        .into_iter()
        .map(|counted| (counted.file, counted.count))
        .collect();
    let paths: BTreeMap<NodeId, (RepoPath, u32)> = files
        .into_iter()
        .map(|(target, file)| {
            let commits = commits_of.get(&file.id).copied().unwrap_or(0);
            (target, (file.file.path, commits))
        })
        .collect();
    let latest_seq = queries.latest_task_seq()?;
    let (mut partners, mut linked) = (Vec::new(), Vec::new());
    for row in edges {
        let Some((path, partner_commits)) = paths.get(&row.edge.dst) else {
            continue;
        };
        let file = NamedFile {
            node: row.edge.dst,
            path: path.clone(),
        };
        if row.edge_type == EdgeType::CoChange {
            // A partner changed at least as often as it changed with the edited file, whatever its
            // own count says.
            let partner_commits = (*partner_commits).max(row.edge.count).max(1);
            partners.push(Partner {
                file,
                co_changes: row.edge.count,
                confidence: row.edge.weight,
                reverse_confidence: f64::from(row.edge.count) / f64::from(partner_commits),
            });
        } else {
            let last_seq = u64::try_from(row.edge.last_seq).unwrap_or(0);
            linked.push(Linked {
                file,
                weight: decayed(row.edge.weight, last_seq, latest_seq, link_decay),
                tasks: row.edge.count,
            });
        }
    }
    Ok((partners, linked))
}

/// The drift of `symbol`, one of the edited file's (`edited`): its name, and the calls to it (not
/// its imports) from files whose bytes on disk are the ones the index read, by path and line, the edited file's own
/// among them for the rule to leave out; the calls in files past [`MAX_CHECKED_CALLER_FILES`]
/// counted as not checked. None when the store no longer holds the symbol.
fn drift(
    queries: Queries<'_>,
    asked: &EditQuery<'_>,
    edited: FileId,
    symbol: SymbolId,
) -> Result<Option<Drift>, StoreError> {
    let Some(name) = queries
        .symbols_in_file(edited)?
        .into_iter()
        .find(|row| row.id == symbol)
        .map(|row| row.name)
    else {
        return Ok(None);
    };
    let Some(symbol_node) = queries.symbol_node(symbol)? else {
        return Ok(None);
    };
    let mut by_file: BTreeMap<FileId, Vec<(u32, f64)>> = BTreeMap::new();
    // Only calls: an import of the symbol does not break when its signature changes.
    for reference in queries.refs_tied_to(symbol)? {
        if reference.kind == RefKind::Call {
            by_file
                .entry(reference.file)
                .or_default()
                .push((reference.line, reference.confidence));
        }
    }
    let mut call_sites = Vec::new();
    let mut unchecked = 0_u32;
    for (checked, (file, calls)) in by_file.into_iter().enumerate() {
        if checked >= MAX_CHECKED_CALLER_FILES {
            unchecked = unchecked.saturating_add(u32::try_from(calls.len()).unwrap_or(u32::MAX));
            continue;
        }
        let Some(caller) = checked_caller(queries, asked, file)? else {
            continue;
        };
        for (line, confidence) in calls {
            call_sites.push(CallSite {
                file: caller.clone(),
                line,
                confidence,
            });
        }
    }
    call_sites
        .sort_by(|left, right| (&left.file.path, left.line).cmp(&(&right.file.path, right.line)));
    Ok(Some(Drift {
        symbol: symbol_node,
        name: name.as_str().to_owned(),
        call_sites,
        unchecked,
    }))
}

/// The file `file` with its node and path, when its bytes on disk are the ones the index read;
/// none for a file that changed since, is gone, or was never read.
fn checked_caller(
    queries: Queries<'_>,
    asked: &EditQuery<'_>,
    file: FileId,
) -> Result<Option<NamedFile>, StoreError> {
    let Some(path) = queries.file_path(file)? else {
        return Ok(None);
    };
    let Some(row) = queries.file(&path)? else {
        return Ok(None);
    };
    let Some(stored) = row.file.content_hash else {
        return Ok(None);
    };
    // INVARIANT: true-at-render (a call site's line is told only from a file whose bytes are the
    // ones the index read)
    if CheckedFile::read(asked.root, &path, &stored, asked.max_file_bytes).is_err() {
        return Ok(None);
    }
    Ok(queries
        .file_node(file)?
        .map(|node| NamedFile { node, path }))
}

/// What task `task_id` did so far, as the store holds it and the edit being answered adds: the
/// files it read (a read that failed touched nothing) or edited, its first [`MAX_TASK_ROWS`] rows
/// read; `edited_now`, the file the edit being answered changed, which the store has not recorded
/// yet; and each pair the store holds it was warned of. Pairs told and not yet recorded are the
/// caller's to add.
///
/// # Errors
///
/// The store's errors.
pub fn task_so_far(
    queries: Queries<'_>,
    task_id: Ulid,
    edited_now: NodeId,
) -> Result<TaskSoFar, StoreError> {
    let mut touched = BTreeSet::from([edited_now]);
    let mut edited = BTreeSet::from([edited_now]);
    for row in queries.events_of_task(task_id, MAX_TASK_ROWS)? {
        let event = &row.record;
        let Some(node) = event.node_id else {
            continue;
        };
        match event.kind {
            EventKind::Edit => {
                touched.insert(node);
                edited.insert(node);
            }
            EventKind::Read if event.meta.is_success != Some(false) => {
                touched.insert(node);
            }
            _ => {}
        }
    }
    let warned = queries
        .warnings_of_task(task_id)?
        .iter()
        .map(|row| (row.record.kind, row.record.source, row.record.target))
        .collect();
    Ok(TaskSoFar {
        touched,
        edited,
        warned,
    })
}
