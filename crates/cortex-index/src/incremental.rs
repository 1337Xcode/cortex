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

use cortex_core::{EdgeType, FileFlags, Identifier, RepoPath};
use cortex_store::{
    FileId, FileRow, MetaKey, NodeId, Queries, RefId, StoreError, SymbolId, Writer,
};

use crate::discover::DiscoveredFile;
use crate::name_changes::NameChanges;
use crate::resolve::search;

/// How many files away from a change the files read again for their bindings may be: a search for
/// a name passes through this many files less one and looks into the last.
const MAX_FILES_AWAY: usize = search::MAX_FILES_PASSED_THROUGH + 1;

/// The most files read again for their bindings around a change. Past this many the rest are left
/// as they are, and names passed on through them may resolve less until every file is read again.
pub(crate) const MAX_FILES_AROUND: usize = 10_000;

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

/// Files found around a change, and whether [`MAX_FILES_AROUND`] cut the search short: then
/// some files that should be read again are not, and the index must not count as complete.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Neighbours {
    /// The files found.
    pub(crate) files: BTreeSet<RepoPath>,
    /// Whether the cap left files out.
    pub(crate) is_cut: bool,
}

impl Neighbours {
    /// These files and `other`'s, at most [`MAX_FILES_AROUND`], in path order; cut when either
    /// was or when together they are more.
    pub(crate) fn join(self, other: Neighbours) -> Neighbours {
        let mut files = self.files;
        files.extend(other.files);
        let is_cut = self.is_cut || other.is_cut || files.len() > MAX_FILES_AROUND;
        Neighbours {
            files: files.into_iter().take(MAX_FILES_AROUND).collect(),
            is_cut,
        }
    }
}

/// The stored files whose imports name one of `files`, directly or through importers that pass
/// names on, up to four files away; at most [`MAX_FILES_AROUND`]. These are the files whose
/// references through imports may find something else when `files` pass on something else or are
/// removed.
///
/// # Errors
///
/// Store errors.
pub(crate) fn importers(
    queries: Queries<'_>,
    files: &BTreeSet<RepoPath>,
) -> Result<Neighbours, StoreError> {
    FileGraph::new(queries).walk(files, |node| {
        queries.sources_into(node, EdgeType::Structural)
    })
}

/// What removing the stored files at `paths` concerns, read before they are removed: their
/// importers, as [`importers`] finds them, the files holding the SCIP index's ties into them, as
/// [`scip_users`] finds them, and the names of their definitions, which other files' references
/// may find something else for.
///
/// # Errors
///
/// Store errors.
pub(crate) fn around_removal(
    queries: Queries<'_>,
    paths: &BTreeSet<RepoPath>,
) -> Result<(Neighbours, NameChanges), StoreError> {
    let mut names = NameChanges::default();
    let mut ids = Vec::new();
    for path in paths {
        let Some(row) = queries.file(path)? else {
            continue;
        };
        ids.push(row.id);
        for symbol in queries.symbols_in_file(row.id)? {
            names.went(&symbol.name, row.file.language);
        }
    }
    let tied = Neighbours {
        files: scip_users(queries, &ids)?,
        is_cut: false,
    };
    Ok((importers(queries, paths)?.join(tied), names))
}

/// The stored files holding a reference that goes through an import named in `names` or as a file
/// passes one of them on (see [`passed_on_as`]): references an index may tie otherwise when the
/// definitions of those names came or went, and only with the file's imports read again.
///
/// # Errors
///
/// Store errors.
pub(crate) fn users_through_imports(
    queries: Queries<'_>,
    names: &NameChanges,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    let mut files = BTreeSet::new();
    for name in passed_on_as(queries, names.names())? {
        files.extend(queries.files_with_refs_through_imports(&name)?);
    }
    paths_of(queries, files)
}

/// `names` and every name a file passes one of them on as (`from c2 import x as y` passes `x` on as
/// `y`), and the names those are passed on as, as many files deep as a search for a name follows:
/// an import of `y` may find a definition named `x`.
///
/// # Errors
///
/// Store errors.
fn passed_on_as(
    queries: Queries<'_>,
    names: BTreeSet<&Identifier>,
) -> Result<BTreeSet<Identifier>, StoreError> {
    let mut found: BTreeSet<Identifier> = names.into_iter().cloned().collect();
    let mut last: Vec<Identifier> = found.iter().cloned().collect();
    for _ in 0..search::MAX_FILES_PASSED_THROUGH {
        let mut next = Vec::new();
        for name in &last {
            for alias in queries.names_passed_on_as(name)? {
                if found.insert(alias.clone()) {
                    next.push(alias);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        last = next;
    }
    Ok(found)
}

/// The stored files holding a reference the SCIP index tied to a definition in one of `files`,
/// read before those are written or removed. Once they change, what the index says of them is out
/// of date, and a full index ties such a reference otherwise: through its own file's imports,
/// which must be read again for it.
///
/// # Errors
///
/// Store errors.
pub(crate) fn scip_users(
    queries: Queries<'_>,
    files: &[FileId],
) -> Result<BTreeSet<RepoPath>, StoreError> {
    paths_of(queries, queries.files_with_scip_ties_into(files)?)
}

/// The stored files holding a reference the SCIP index tied when last read and does not tie in
/// `ties`, what it ties now: a full index ties such a reference through its own file's imports,
/// which must be read again for it.
///
/// # Errors
///
/// Store errors.
pub(crate) fn lost_scip_users(
    queries: Queries<'_>,
    ties: &BTreeMap<RefId, SymbolId>,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    let lost = queries
        .scip_ties()?
        .into_iter()
        .filter(|(reference, _)| !ties.contains_key(reference))
        .map(|(_, file)| file);
    paths_of(queries, lost.collect::<BTreeSet<_>>())
}

/// The paths of the stored files `files`.
///
/// # Errors
///
/// Store errors.
fn paths_of(
    queries: Queries<'_>,
    files: impl IntoIterator<Item = FileId>,
) -> Result<BTreeSet<RepoPath>, StoreError> {
    let mut paths = BTreeSet::new();
    for file in files {
        paths.extend(queries.file_path(file)?);
    }
    Ok(paths)
}

/// The stored files and their nodes, both ways, each read when first needed.
struct FileGraph<'q> {
    /// Where files and nodes are read from.
    queries: Queries<'q>,
    /// Each node read so far, with the row and path of the file it stands for, if any.
    files: BTreeMap<NodeId, Option<(FileId, RepoPath)>>,
}

impl<'q> FileGraph<'q> {
    /// The files and nodes `queries` reads.
    fn new(queries: Queries<'q>) -> Self {
        Self {
            queries,
            files: BTreeMap::new(),
        }
    }

    /// The row and path of the file `node` stands for, if it stands for one.
    fn file_of(&mut self, node: NodeId) -> Result<Option<(FileId, RepoPath)>, StoreError> {
        if let Some(file) = self.files.get(&node) {
            return Ok(file.clone());
        }
        let file = self.queries.node_file(node)?;
        self.files.insert(node, file.clone());
        Ok(file)
    }

    /// The files `next` leads to from `start`, step by step up to [`MAX_FILES_AWAY`] steps, each a
    /// file node `next` gives for `start` or for one already found that passes names on; without
    /// `start` itself, and at most [`MAX_FILES_AROUND`].
    fn walk(
        mut self,
        start: &BTreeSet<RepoPath>,
        next: impl Fn(NodeId) -> Result<Vec<NodeId>, StoreError>,
    ) -> Result<Neighbours, StoreError> {
        let mut found: BTreeSet<RepoPath> = BTreeSet::new();
        let mut frontier: Vec<NodeId> = Vec::new();
        for path in start {
            frontier.extend(self.queries.file_node_at(path)?);
        }
        for _ in 0..MAX_FILES_AWAY {
            let mut further = Vec::new();
            for node in frontier {
                for linked in next(node)? {
                    let Some((file, path)) = self.file_of(linked)? else {
                        continue;
                    };
                    if start.contains(&path) || found.contains(&path) {
                        continue;
                    }
                    if found.len() == MAX_FILES_AROUND {
                        return Ok(Neighbours {
                            files: found,
                            is_cut: true,
                        });
                    }
                    found.insert(path);
                    // Only a file that passes names on can pass on what changed further.
                    if !self.queries.passes_on(file)?.is_empty() {
                        further.push(linked);
                    }
                }
            }
            frontier = further;
        }
        Ok(Neighbours {
            files: found,
            is_cut: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Joined neighbours keep the first files in path order up to the cap and say they were cut
    /// when the two together are more than the cap, or when either was cut already.
    #[test]
    fn joined_neighbours_are_cut_at_the_cap() -> Result<(), cortex_core::PathError> {
        let files = |range: std::ops::Range<usize>| {
            range
                .map(|number| RepoPath::new(&format!("f{number:05}.py")))
                .collect::<Result<BTreeSet<_>, _>>()
        };
        let small = Neighbours {
            files: files(0..2)?,
            is_cut: false,
        }
        .join(Neighbours {
            files: files(1..3)?,
            is_cut: false,
        });
        assert_eq!((small.files.len(), small.is_cut), (3, false));
        let big = Neighbours {
            files: files(0..6_000)?,
            is_cut: false,
        }
        .join(Neighbours {
            files: files(6_000..10_001)?,
            is_cut: false,
        });
        assert_eq!((big.files.len(), big.is_cut), (MAX_FILES_AROUND, true));
        assert_eq!(big.files.last().map(RepoPath::as_str), Some("f09999.py"));
        let inherited = Neighbours {
            files: files(0..1)?,
            is_cut: true,
        }
        .join(Neighbours::default());
        assert!(inherited.is_cut);
        Ok(())
    }
}
