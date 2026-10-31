//! Indexing only what changed since the last index. A file the walk finds as it is stored, with
//! the same modification time, size, language and flags, is not read again; one whose contents hash
//! as stored has only its time updated. Every other file is read, parsed and written.
//!
//! What a file's imports bind lives only in memory, so the files around a change are read again
//! for it, without being written: those whose imports name a changed or removed file, up to four
//! files away, then those that the changed files and these reach through their imports, up to four
//! files away. Their references are tied again, and names passed on through them resolve as in a
//! full index. A Cortex of another version reads every file again, its rules perhaps changed.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{EdgeType, FileFlags, RepoPath};
use cortex_store::{FileRow, MetaKey, NodeId, Queries, StoreError, Writer};

use crate::discover::DiscoveredFile;

/// How many files away from a change the files read again for their bindings may be: as far as a
/// search for a name passes through files.
const MAX_FILES_AWAY: usize = 4;

/// The most files read again for their bindings around a change. Past this many the rest are left
/// as they are, and names passed on through them may resolve less until every file is read again.
const MAX_FILES_AROUND: usize = 10_000;

/// The version of this Cortex, which the store records when an index completes.
const CORTEX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether this Cortex completed the store's last index, so that its rows can be kept as they are.
///
/// # Errors
///
/// Store errors.
pub(crate) fn is_complete(queries: Queries<'_>) -> Result<bool, StoreError> {
    Ok(queries.meta(MetaKey::CortexVersion)?.as_deref() == Some(CORTEX_VERSION))
}

/// Marks the store's index as under way before anything is written: an index or a parse that
/// stops partway, the process killed or a write failed, leaves it so, and the next index reads
/// every file again instead of keeping rows whose ties were left half done.
///
/// # Errors
///
/// Store errors.
pub(crate) fn mark_started(writer: &mut Writer) -> Result<(), StoreError> {
    let mut transaction = writer.transaction()?;
    transaction.clear_cortex_version()?;
    transaction.commit()
}

/// Marks the store's index as completed by this Cortex.
///
/// # Errors
///
/// Store errors.
pub(crate) fn mark_complete(writer: &mut Writer) -> Result<(), StoreError> {
    let mut transaction = writer.transaction()?;
    transaction.set_cortex_version()?;
    transaction.commit()
}

/// The stored files an index may keep: every stored file's row, by path, when this Cortex
/// completed the last index, and none when another did, none did or the last one stopped partway.
///
/// # Errors
///
/// Store errors.
pub(crate) fn known_files(queries: Queries<'_>) -> Result<BTreeMap<RepoPath, FileRow>, StoreError> {
    if !is_complete(queries)? {
        return Ok(BTreeMap::new());
    }
    Ok(queries
        .file_rows()?
        .into_iter()
        .map(|row| (row.file.path.clone(), row))
        .collect())
}

/// The stored files parsed before, whatever Cortex parsed them: from `known` when it holds every
/// stored file, else read from the store.
///
/// # Errors
///
/// Store errors.
pub(crate) fn parsed_files(
    queries: Queries<'_>,
    known: &BTreeMap<RepoPath, FileRow>,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    let is_parsed = |row: &FileRow| row.file.flags.contains(FileFlags::PARSED);
    if !known.is_empty() {
        return Ok(known
            .values()
            .filter(|row| is_parsed(row))
            .map(|row| row.file.path.clone())
            .collect());
    }
    Ok(queries
        .file_rows()?
        .into_iter()
        .filter(is_parsed)
        .map(|row| row.file.path)
        .collect())
}

/// Whether `file`, as the walk found it, is as `known` holds it: the same modification time,
/// size, language and flags, but for whether it was parsed.
pub(crate) fn is_unchanged(file: &DiscoveredFile, known: &FileRow) -> bool {
    let stored = &known.file;
    file.mtime_ns == stored.mtime_ns
        && file.size_bytes == stored.size_bytes
        && file.language == stored.language
        && file.flags == unparsed(stored.flags)
}

/// `flags` without the mark that the file was parsed, as the walk finds a file's flags.
pub(crate) fn unparsed(flags: FileFlags) -> FileFlags {
    FileFlags::from_bits(flags.bits() & !FileFlags::PARSED.bits())
}

/// The stored files whose imports name one of `files`, directly or through files that do, up to
/// four files away; at most [`MAX_FILES_AROUND`].
///
/// # Errors
///
/// Store errors.
pub(crate) fn importers(
    queries: Queries<'_>,
    files: &BTreeSet<RepoPath>,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    FileGraph::new(queries).walk(files, |node| {
        queries.sources_into(node, EdgeType::Structural)
    })
}

/// The stored files that `files` reach through their imports, directly or through the files they
/// reach, up to four files away; at most [`MAX_FILES_AROUND`].
///
/// # Errors
///
/// Store errors.
pub(crate) fn reached(
    queries: Queries<'_>,
    files: &BTreeSet<RepoPath>,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    FileGraph::new(queries).walk(files, |node| {
        let rows = queries.edges_from(node)?;
        Ok(rows
            .into_iter()
            .filter(|row| row.edge_type == EdgeType::Structural)
            .map(|row| row.edge.dst)
            .collect())
    })
}

/// The stored files and their nodes, both ways, each read when first needed.
struct FileGraph<'q> {
    /// Where files and nodes are read from.
    queries: Queries<'q>,
    /// Each node read so far, with the path of the file it stands for, if any.
    paths: BTreeMap<NodeId, Option<RepoPath>>,
}

impl<'q> FileGraph<'q> {
    /// The files and nodes `queries` reads.
    fn new(queries: Queries<'q>) -> Self {
        Self {
            queries,
            paths: BTreeMap::new(),
        }
    }

    /// The path of the file `node` stands for, if it stands for one.
    fn path_of(&mut self, node: NodeId) -> Result<Option<RepoPath>, StoreError> {
        if let Some(path) = self.paths.get(&node) {
            return Ok(path.clone());
        }
        let path = self.queries.node_file(node)?;
        self.paths.insert(node, path.clone());
        Ok(path)
    }

    /// The files `next` leads to from `start`, step by step up to [`MAX_FILES_AWAY`] steps, each a
    /// file node `next` gives for one already found; without `start` itself, and at most
    /// [`MAX_FILES_AROUND`].
    fn walk(
        mut self,
        start: &BTreeSet<RepoPath>,
        next: impl Fn(NodeId) -> Result<Vec<NodeId>, StoreError>,
    ) -> Result<BTreeSet<RepoPath>, StoreError> {
        let mut found: BTreeSet<RepoPath> = BTreeSet::new();
        let mut frontier: Vec<NodeId> = Vec::new();
        for path in start {
            frontier.extend(self.queries.file_node_at(path)?);
        }
        for _ in 0..MAX_FILES_AWAY {
            let mut further = Vec::new();
            for node in frontier {
                for linked in next(node)? {
                    let Some(path) = self.path_of(linked)? else {
                        continue;
                    };
                    if start.contains(&path) || found.contains(&path) {
                        continue;
                    }
                    if found.len() == MAX_FILES_AROUND {
                        return Ok(found);
                    }
                    found.insert(path);
                    further.push(linked);
                }
            }
            frontier = further;
        }
        Ok(found)
    }
}
