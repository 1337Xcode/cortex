//! The graph's folders: a node for each folder that directly holds a stored file, joined to each
//! such file by a containment edge. A folder that holds only folders gets no node.
//!
//! A containment edge carries full strength and points both ways. How much containment counts
//! when activation spreads is the containment type's weight (`retrieval.type_weights`, 0.1 by
//! default), applied where the graph is read, so the edge itself adds nothing to it.

use std::collections::BTreeMap;

use cortex_core::{EdgeType, RepoPath};
use cortex_store::{EdgeRecord, StoreError, Writer};

/// The strength stored on a containment edge: a folder holds its file entirely.
const CONTAINMENT_STRENGTH: f64 = 1.0;

/// Gives every folder that directly holds a stored file a node joined to each such file, and
/// removes the nodes of folders that hold none any more, in one transaction.
///
/// # Errors
///
/// Store errors; the transaction then writes nothing.
pub(crate) fn write_folders(writer: &mut Writer) -> Result<(), StoreError> {
    let mut folders: BTreeMap<RepoPath, Vec<EdgeRecord>> = BTreeMap::new();
    for (path, node) in writer.queries().file_nodes()? {
        // A stored file has a name, so it is in a folder, the root at least.
        let Some(folder) = path.parent() else {
            continue;
        };
        folders.entry(folder).or_default().push(EdgeRecord {
            dst: node,
            weight: CONTAINMENT_STRENGTH,
            // One observation supports it: the folder listing.
            count: 1,
            last_seq: 0,
            is_directed: false,
        });
    }
    let emptied: Vec<RepoPath> = writer
        .queries()
        .folder_nodes()?
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| !folders.contains_key(path))
        .collect();
    let mut transaction = writer.transaction()?;
    for path in &emptied {
        transaction.remove_folder_node(path)?;
    }
    // INVARIANT: deterministic. New folders get their nodes in path order.
    for (path, edges) in &folders {
        let node = transaction.folder_node(path)?;
        transaction.replace_edges(node, EdgeType::Containment, edges)?;
    }
    transaction.commit()
}
