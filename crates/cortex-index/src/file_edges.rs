//! The graph's structural edges between files: from each file to each file its imports name.
//!
//! An import edge points one way, from the importer. It is as strong as a reference tied by its
//! import path, and its count is how many of the importer's imports name the target.

use std::collections::BTreeMap;

use cortex_core::{EdgeType, RepoPath, Resolution};
use cortex_store::{EdgeRecord, NodeId, StoreError, Writer};

/// How many importers have their edges written in one transaction: a commit costs little next to
/// this many files' edges, and the transaction stays short.
const IMPORTERS_PER_TRANSACTION: usize = 4096;

/// The files one file's imports name, each with how many of its imports name it.
pub(crate) type ImportCounts = BTreeMap<RepoPath, u32>;

/// Replaces the structural edges from each file of `importers` with edges to the files its
/// imports name. A file or target without a node gets no edge.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn write_import_edges(
    writer: &mut Writer,
    importers: &[(RepoPath, ImportCounts)],
) -> Result<(), StoreError> {
    let nodes: BTreeMap<RepoPath, NodeId> = writer.queries().file_nodes()?.into_iter().collect();
    for batch in importers.chunks(IMPORTERS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        for (importer, counts) in batch {
            let Some(src) = nodes.get(importer) else {
                continue;
            };
            let edges: Vec<EdgeRecord> = counts
                .iter()
                .filter_map(|(target, count)| {
                    Some(EdgeRecord {
                        dst: *nodes.get(target)?,
                        weight: Resolution::ImportPath.confidence(),
                        count: *count,
                        // No task observed it: the code itself says so.
                        last_seq: 0,
                        is_directed: true,
                    })
                })
                .collect();
            transaction.replace_edges(*src, EdgeType::Structural, &edges)?;
        }
        transaction.commit()?;
    }
    Ok(())
}
