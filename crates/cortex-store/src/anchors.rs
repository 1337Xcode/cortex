//! Where facts' anchors stand now. An anchor is a file or a symbol; what it stands on is read in
//! one place, the `anchor_hashes` view and [`Queries::anchor_states`]: the file's path, what the
//! index holds for it, the symbol's qualified name, and the hash an anchor on the node would keep
//! now (a file's first 16 content-hash bytes, a symbol's body hash). A reader that shows a fact
//! compares that hash with the one the fact kept, and the file on disk with what the index holds.

use std::collections::BTreeMap;

use cortex_core::{ContentHash, Identifier, RepoPath, ShortHash};

use crate::error::{StoreError, sqlite};
use crate::facts::FactAnchor;
use crate::ids::NodeId;
use crate::queries::{Queries, json_array};
use crate::{schema, values};

/// Where one anchor stands now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorState {
    /// The file or symbol.
    pub node: NodeId,
    /// The file, the symbol's for a symbol.
    pub path: RepoPath,
    /// What the index holds for that file; none when it does not read its contents.
    pub content: Option<ContentHash>,
    /// For a symbol, its qualified name.
    pub symbol: Option<Identifier>,
    /// The hash an anchor on the node would keep now; none for a file whose contents are not read.
    pub hash: Option<ShortHash>,
}

impl AnchorState {
    /// The anchor as a note names it: the file's path, and for a symbol `:` and its qualified name.
    #[must_use]
    pub fn name(&self) -> String {
        match &self.symbol {
            Some(symbol) => format!("{}:{}", self.path.as_str(), symbol.as_str()),
            None => self.path.as_str().to_owned(),
        }
    }
}

/// An anchor state's columns as read: node, path, content hash, qualified name, hash now.
type StateColumns = (
    i64,
    String,
    Option<Vec<u8>>,
    Option<String>,
    Option<Vec<u8>>,
);

impl Queries<'_> {
    /// Where each of `nodes` that is a file or a symbol in one stands now, in node order. A node
    /// that is gone, or neither, is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn anchor_states(self, nodes: &[NodeId]) -> Result<Vec<AnchorState>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let rows: Vec<StateColumns> = self
            .connection()
            .prepare_cached(schema::ANCHOR_STATES_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    })?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading where anchors stand"))?;
        rows.into_iter().map(state_of).collect()
    }

    /// Each of `nodes` that is a file or a symbol, named as a note names an anchor: the file's
    /// path, and for a symbol `:` and its qualified name. A node that is gone, or neither, is left
    /// out.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn anchor_names(self, nodes: &[NodeId]) -> Result<BTreeMap<NodeId, String>, StoreError> {
        Ok(self
            .anchor_states(nodes)?
            .into_iter()
            .map(|state| (state.node, state.name()))
            .collect())
    }

    /// The hash an anchor on each of `nodes` would keep now, in node order: the first 16 bytes of
    /// a file's content hash, a symbol's body hash. A node with none (a folder, a file whose
    /// contents are not read, a node that does not exist) is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a hash that is not what the store writes.
    pub fn anchor_hashes(self, nodes: &[NodeId]) -> Result<Vec<FactAnchor>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let rows: Vec<(i64, Vec<u8>)> = self
            .connection()
            .prepare_cached(schema::ANCHOR_HASHES_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading anchor hashes"))?;
        rows.into_iter()
            .map(|(node, hash)| anchor(node, hash, "anchor_hashes.hash", node))
            .collect()
    }
}

/// The anchor state `columns` hold.
fn state_of(columns: StateColumns) -> Result<AnchorState, StoreError> {
    let (node, path, content, symbol, hash) = columns;
    let corrupt = |column: &'static str| StoreError::Corrupt { column, row: node };
    let content = content
        .map(|bytes| {
            <[u8; 32]>::try_from(bytes)
                .map(ContentHash::from_bytes)
                .map_err(|_| corrupt("files.content_hash"))
        })
        .transpose()?;
    let hash = hash
        .map(|bytes| anchor(node, bytes, "anchor_hashes.hash", node).map(|anchor| anchor.hash))
        .transpose()?;
    Ok(AnchorState {
        node: NodeId(node),
        path: values::path(&path, "files.path", node)?,
        content,
        symbol: symbol
            .map(|name| values::identifier(&name, "symbols.qual_name", node))
            .transpose()?,
        hash,
    })
}

/// The anchor on `node` whose stored hash is `hash`, which must be 16 bytes; `column` of `row`
/// is corrupt otherwise.
pub(crate) fn anchor(
    node: i64,
    hash: Vec<u8>,
    column: &'static str,
    row: i64,
) -> Result<FactAnchor, StoreError> {
    let hash: [u8; 16] = hash
        .try_into()
        .map_err(|_| StoreError::Corrupt { column, row })?;
    Ok(FactAnchor {
        node: NodeId(node),
        hash: ShortHash::from_bytes(hash),
    })
}
