//! What can go wrong while retrieving: the store failing to read, or a graph too large to number.

use cortex_store::StoreError;

use crate::graph::GraphError;

/// Why retrieval could not go on.
#[derive(Debug, thiserror::Error)]
pub enum RetrieveError {
    /// The store could not be read.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The graph holds more than 32-bit numbers count.
    #[error(transparent)]
    Graph(#[from] GraphError),
}
