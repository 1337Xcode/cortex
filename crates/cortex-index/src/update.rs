//! Indexing chosen paths now: the files the watcher saw change, those an agent read or edited, and
//! the crawler's next batch in huge mode. Each path is looked at as the walk would look at it: a
//! file found there is read and parsed unless it is stored as it is, a file gone or no longer
//! found is removed, and one that cannot be read now is left as stored. Then only what the change
//! concerns is tied again: the references in the files written and in the files around them, read
//! again for their imports, and the references elsewhere named like a definition that came or
//! went. The store ends as an index of the whole repository would leave it.
//!
//! A change that concerns more than files is indexed with the whole repository instead: a folder
//! (created, removed or renamed), a file of rules about which files count, the SCIP index, and a
//! store whose last index did not finish.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use cortex_core::{FileFlags, RepoPath, ScaleMode};
use cortex_store::{FileRow, StoreError, Writer};

use crate::discover::DiscoveredFile;
use crate::grammar::Grammars;
use crate::pipeline::{
    self, Context, IndexError, IndexOptions, IndexReport, Pass, index_repository,
};
use crate::scale::{self, RepoSize};
use crate::scip::{self, ScipOutcome};
use crate::seen::{self, PathRules, PathState, WalkRules};
use crate::{folders, incremental};

/// What the chosen paths hold, against the store.
#[derive(Default)]
struct Found {
    /// The files to read: every file found, since a path is given because it changed. One whose
    /// contents are as stored gets only its new time.
    candidates: Vec<DiscoveredFile>,
    /// The stored rows of the candidates stored before.
    known: BTreeMap<RepoPath, FileRow>,
    /// The stored files no longer found.
    gone: Vec<FileRow>,
}

/// Indexes the files at `paths` of the repository at `root` now, as the module documentation
/// says, and reports what it wrote. In huge mode too, every file found is parsed: these are the
/// files in use.
///
/// # Errors
///
/// As [`index_repository`]: [`IndexError::Discover`] when the repository's rules cannot be read,
/// [`IndexError::Store`], [`IndexError::Grammar`] and [`IndexError::WorkerPanicked`].
pub fn index_paths(
    root: &Path,
    grammars: &Arc<Grammars>,
    writer: &mut Writer,
    options: &IndexOptions<'_>,
    paths: &[RepoPath],
) -> Result<IndexReport, IndexError> {
    let paths: BTreeSet<&RepoPath> = paths.iter().collect();
    let mut rules = PathRules::new(WalkRules::of(root, &options.discover))?;
    let is_complete = incremental::is_complete(writer.queries())?;
    if !is_complete || concerns_everything((root, writer), &mut rules, &paths)? {
        return index_repository(root, grammars, writer, options);
    }
    incremental::mark_started(writer)?;
    let found = find(writer, &mut rules, &paths)?;
    let gone: BTreeSet<RepoPath> = found.gone.iter().map(|row| row.file.path.clone()).collect();
    let (around_gone, mut names) = incremental::around_removal(writer.queries(), &gone)?;
    pipeline::remove_files(writer, &gone.iter().cloned().collect::<Vec<_>>())?;
    let walked: BTreeMap<&RepoPath, FileFlags> = found
        .candidates
        .iter()
        .map(|file| (&file.path, file.flags))
        .collect();
    let candidates: Vec<&RepoPath> = walked.keys().copied().collect();
    let queries = writer.queries();
    let flags_of = |path: &RepoPath| {
        walked.get(path).copied().or_else(|| {
            let row = queries.file(path).ok()??;
            Some(row.file.flags)
        })
    };
    let max_file_bytes = options.discover.max_file_bytes;
    let project = pipeline::load_project(root, writer, (&candidates, &flags_of), max_file_bytes)?;
    let context = Context {
        root,
        grammars,
        project: &project,
        known: Some(&found.known),
        parsed_before: None,
        max_file_bytes,
        parse_timeout: options.parse_timeout,
        threads: options.threads,
        now_ms: options.now_ms,
        scip_written_ns: scip::written_ns(root),
    };
    let mut report = IndexReport {
        files: 0,
        parsed: 0,
        unchanged: 0,
        removed: pipeline::count(gone.len()),
        walk_problems: Vec::new(),
        problems: Vec::new(),
        left_out: Vec::new(),
        changed_symbols: Vec::new(),
        scip: ScipOutcome::NotRead,
        size: RepoSize {
            source_files: 0,
            lines: 0,
        },
        mode: ScaleMode::Tiny,
    };
    let written = pipeline::write_files(writer, &found.candidates, &context, &mut report)?;
    for file in &written {
        names.extend(file.names.clone());
    }
    let has_new_files = !gone.is_empty() || written.iter().any(|file| file.is_new);
    let canonical_root = root.canonicalize().ok();
    let queries = writer.queries();
    let file_of = |path: &RepoPath| {
        let row = queries.file(path).ok()??.file;
        if !seen::is_reached_directly(root, canonical_root.as_deref()?, path) {
            return None;
        }
        Some(DiscoveredFile {
            path: row.path,
            language: row.language,
            size_bytes: row.size_bytes,
            mtime_ns: row.mtime_ns,
            flags: incremental::unparsed(row.flags),
        })
    };
    let (around, is_cut) =
        pipeline::files_around(writer, &file_of, &written, (around_gone, &names))?;
    let read_again = pipeline::read_again(&around, &context)?;
    pipeline::link(
        writer,
        (written, read_again),
        Pass::Around(&names),
        &mut report,
    )?;
    if has_new_files {
        folders::write_folders(writer)?;
    }
    (report.mode, report.size) = scale::settle_mode(writer, options.mode)?;
    // Files around the change left out by the cap keep stale ties: the next index reads them all.
    if !is_cut {
        incremental::mark_complete(writer)?;
    }
    Ok(report)
}

/// Whether a change at `paths` of the repository at `root`, stored by `writer`, concerns more than
/// the files there: a file of rules the walk reads, as `rules` tells, the SCIP index, or a folder,
/// now or in the store.
fn concerns_everything(
    (root, writer): (&Path, &Writer),
    rules: &mut PathRules,
    paths: &BTreeSet<&RepoPath>,
) -> Result<bool, StoreError> {
    for path in paths {
        let is_special = rules.changes_rules(path) || path.as_str() == scip::INDEX_FILE;
        if is_special || path.to_native(root).is_dir() || writer.queries().has_files_under(path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What `paths` hold, found by `rules`, against the store `writer` keeps.
fn find(
    writer: &Writer,
    rules: &mut PathRules,
    paths: &BTreeSet<&RepoPath>,
) -> Result<Found, StoreError> {
    let queries = writer.queries();
    let mut found = Found::default();
    for path in paths {
        let stored = queries.file(path)?;
        match rules.state_of(path) {
            PathState::Absent => found.gone.extend(stored),
            // As the walk keeps a file it cannot inspect, so does an index of its path.
            PathState::Unreadable => {}
            PathState::File(file) => {
                // Its time and size are not trusted to say it is unchanged: a file rewritten at
                // the same size within one tick of a coarse clock keeps both.
                if let Some(row) = stored {
                    found.known.insert((*path).clone(), row);
                }
                found.candidates.push(file);
            }
        }
    }
    Ok(found)
}
