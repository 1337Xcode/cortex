//! The files the map shows, sorted by path, each with the number that names it in the API.
//!
//! A file's id is its index in the list, so it holds only while the files do: the API names files
//! by id in every body, and a page that reads two bodies in turn may find them out of step after
//! a file was added, which the page handles by reading the snapshot again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use cortex_core::RepoPath;
use cortex_store::{FileId, NodeId, Queries, StoreError};

use crate::map::source::Source;

/// One file as the map lists it.
#[derive(Debug, Clone)]
pub(super) struct ListedFile {
    /// Where it is, relative to the repository's root.
    pub(super) path: RepoPath,
    /// Its row in the index.
    pub(super) file: FileId,
    /// Its node in the graph.
    pub(super) node: NodeId,
    /// How many lines it has; none counted gives 0.
    pub(super) lines: u64,
}

/// The files the store holds that the map lists, by path in byte order.
#[derive(Debug)]
pub(in crate::map) struct Listing {
    /// The files; a file's id is its index.
    files: Vec<ListedFile>,
    /// The id of each file, by its node: looked up, never walked, so its order never shows.
    by_node: HashMap<NodeId, usize>,
    /// The id of each file, by its row in the index, likewise.
    by_row: HashMap<FileId, usize>,
}

/// Whether the map lists a file at `path`: its text holds no control character, which the API's
/// paths never do.
fn is_listed(path: &RepoPath) -> bool {
    !path.as_str().chars().any(char::is_control)
}

impl Listing {
    /// The files the store holds, read through `queries` (in one read transaction, so that the
    /// two reads of the files agree).
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub(in crate::map) fn read(queries: Queries<'_>) -> Result<Self, StoreError> {
        let rows = queries.file_rows()?;
        let mut nodes = queries.file_nodes()?.into_iter().peekable();
        let mut files = Vec::with_capacity(rows.len());
        for row in rows {
            let path = row.file.path;
            while nodes.next_if(|(other, _)| *other < path).is_some() {}
            let Some((_, node)) = nodes.next_if(|(other, _)| *other == path) else {
                continue;
            };
            if is_listed(&path) {
                files.push(ListedFile {
                    path,
                    file: row.id,
                    node,
                    lines: row.file.line_count.unwrap_or(0),
                });
            }
        }
        let by_node = files
            .iter()
            .enumerate()
            .map(|(id, file)| (file.node, id))
            .collect();
        let by_row = files
            .iter()
            .enumerate()
            .map(|(id, file)| (file.file, id))
            .collect();
        Ok(Self {
            files,
            by_node,
            by_row,
        })
    }

    /// The files, by id.
    pub(super) fn files(&self) -> &[ListedFile] {
        &self.files
    }

    /// The file with id `id`, if there is one.
    pub(super) fn get(&self, id: usize) -> Option<&ListedFile> {
        self.files.get(id)
    }

    /// The id of the file whose node is `node`, if the map lists one.
    pub(in crate::map) fn id_of_node(&self, node: NodeId) -> Option<usize> {
        self.by_node.get(&node).copied()
    }

    /// The id of the file whose row in the index is `file`, if the map lists one.
    pub(super) fn id_of_row(&self, file: FileId) -> Option<usize> {
        self.by_row.get(&file).copied()
    }

    /// The id of the file at `path`, if the map lists one.
    pub(super) fn id_of_path(&self, path: &RepoPath) -> Option<usize> {
        self.files.binary_search_by(|file| file.path.cmp(path)).ok()
    }
}

/// The listing as it was last read, and the store's generation it was read at.
type Held = Option<(u64, Arc<Listing>)>;

/// The one listing the map keeps: built from the store, and built again only once the daemon notes
/// that the store changed since. Reading it is a read of every file of the index, which at
/// 100,000 files takes longer than the rest of a search or a file's panel, so the endpoints
/// share one copy.
///
/// The copy is held under a lock that also covers building it: a request that finds it out of
/// date builds the new one while the requests behind it wait for that build, and finds it done,
/// so a burst of requests after a change reads the files once.
#[derive(Debug, Default)]
pub(in crate::map) struct ListingCache {
    /// The listing and the generation it was built at.
    held: Mutex<Held>,
    /// How many listings were built, for the tests.
    #[cfg(test)]
    builds: std::sync::atomic::AtomicU64,
}

impl ListingCache {
    /// The listing of the store `source` reads, as of the store's latest noted change: the copy
    /// held when it was built at the generation `source` reports now, else a new one.
    ///
    /// # Errors
    ///
    /// The store cannot be opened or read; the copy held, if any, is kept.
    pub(in crate::map) fn get(&self, source: &Source) -> Result<Arc<Listing>, StoreError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        // The generation is read under the lock, so a request that waited for another's build
        // compares with what that build saw, not with what was current before the wait. It is
        // read before the files, so a change that lands while they are read leaves the copy at
        // the older generation and the next request builds again.
        let generation = source.generation();
        if let Some((built_at, listing)) = held.as_ref()
            && *built_at == generation
        {
            return Ok(Arc::clone(listing));
        }
        let reader = source.open()?;
        let listing = Arc::new(reader.consistently(Listing::read)?);
        #[cfg(test)]
        self.builds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *held = Some((generation, Arc::clone(&listing)));
        Ok(listing)
    }

    /// How many listings were built.
    #[cfg(test)]
    pub(in crate::map) fn builds(&self) -> u64 {
        self.builds.load(std::sync::atomic::Ordering::SeqCst)
    }
}
