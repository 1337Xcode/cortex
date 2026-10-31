//! Graph nodes looked up by what they stand for: every one read at once when most will be needed,
//! as when every file was written, else each read when first needed and kept.

use std::collections::BTreeMap;

use cortex_store::{NodeId, StoreError};

/// How one node is read: the node of a key, if it has one.
type ReadOne<'q, K> = Box<dyn Fn(&K) -> Result<Option<NodeId>, StoreError> + 'q>;

/// The nodes of some keys, read at once or as needed.
pub(crate) struct NodeLookup<'q, K> {
    /// The nodes read so far; a key without a node maps to `None`.
    known: BTreeMap<K, Option<NodeId>>,
    /// How a node not read yet is read; `None` when every node was read at once.
    read_one: Option<ReadOne<'q, K>>,
}

impl<'q, K: Ord + Clone> NodeLookup<'q, K> {
    /// Every node there is, read at once: a key not among them has none.
    pub(crate) fn every(nodes: impl IntoIterator<Item = (K, NodeId)>) -> Self {
        Self {
            known: nodes
                .into_iter()
                .map(|(key, node)| (key, Some(node)))
                .collect(),
            read_one: None,
        }
    }

    /// Nodes read by `read_one` when first needed.
    pub(crate) fn as_needed(
        read_one: impl Fn(&K) -> Result<Option<NodeId>, StoreError> + 'q,
    ) -> Self {
        Self {
            known: BTreeMap::new(),
            read_one: Some(Box::new(read_one)),
        }
    }

    /// The node of `key`, if it has one.
    ///
    /// # Errors
    ///
    /// Store errors while reading it.
    pub(crate) fn get(&mut self, key: &K) -> Result<Option<NodeId>, StoreError> {
        if let Some(node) = self.known.get(key) {
            return Ok(*node);
        }
        let Some(read_one) = &self.read_one else {
            return Ok(None);
        };
        let node = read_one(key)?;
        self.known.insert(key.clone(), node);
        Ok(node)
    }

    /// Every node read so far.
    pub(crate) fn read(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.known.values().flatten().copied()
    }
}
