//! The graph's structural edges between files: from each file to each file its imports name, and
//! from a test file whose imports name no source file to the file it tests by its base name.
//!
//! An import edge points one way, from the importer. It is as strong as a reference tied by its
//! import path, and its count is how many of the importer's imports name the target. An edge from
//! a test by its base name is as strong as a reference tied by its name alone.

use std::collections::BTreeMap;

use cortex_core::{EdgeType, RepoPath, Resolution};
use cortex_store::{EdgeRecord, NodeId, StoreError, Writer};

/// How many importers have their edges written in one transaction: a commit costs little next to
/// this many files' edges, and the transaction stays short.
const IMPORTERS_PER_TRANSACTION: usize = 4096;

/// The files one file's imports name, each with how many of its imports name it.
pub(crate) type ImportCounts = BTreeMap<RepoPath, u32>;

/// Replaces the structural edges from each file of `importers` with edges to the files its
/// imports name, and, for a test among `test_targets`, to the file it tests by its base name. A
/// file or target without a node gets no edge.
///
/// # Errors
///
/// Store errors; the transactions committed before stay written.
pub(crate) fn write_import_edges(
    writer: &mut Writer,
    importers: &[(RepoPath, ImportCounts)],
    test_targets: &BTreeMap<RepoPath, RepoPath>,
) -> Result<(), StoreError> {
    let nodes: BTreeMap<RepoPath, NodeId> = writer.queries().file_nodes()?.into_iter().collect();
    for batch in importers.chunks(IMPORTERS_PER_TRANSACTION) {
        let mut transaction = writer.transaction()?;
        for (importer, counts) in batch {
            let Some(src) = nodes.get(importer) else {
                continue;
            };
            let imported = counts
                .iter()
                .map(|(target, count)| (target, Resolution::ImportPath, *count));
            let tested = test_targets
                .get(importer)
                .map(|target| (target, Resolution::NameOnly, 1));
            let edges: Vec<EdgeRecord> = imported
                .chain(tested)
                .filter_map(|(target, tied, count)| {
                    Some(EdgeRecord {
                        dst: *nodes.get(target)?,
                        weight: tied.confidence(),
                        count,
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
