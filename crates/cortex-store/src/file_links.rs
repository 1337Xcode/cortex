//! How one file is linked to the others, as `cortex why` tells it: for each other file, the
//! strongest edge of each kind between it, or a symbol in it, and the file or a symbol in the file,
//! read both ways. Code structure, history and use together each say something different, so they
//! are kept apart.

use cortex_core::{EdgeType, RepoPath};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::{schema, values};

/// The kinds of link a file's links are read for: code structure, history, use together.
const LINK_KINDS: [EdgeType; 3] = [EdgeType::Structural, EdgeType::CoChange, EdgeType::CoAccess];

/// One other file a file is linked to, by one kind of link.
#[derive(Debug, Clone, PartialEq)]
pub struct FileLink {
    /// The other file.
    pub path: RepoPath,
    /// The kind of link.
    pub edge_type: EdgeType,
    /// The strongest such link's weight.
    pub weight: f64,
}

impl Queries<'_> {
    /// The other files the file at node `file`, or a symbol in it, is linked to by code structure,
    /// history or use together, either way, each with the strongest link of each kind: by kind,
    /// then strongest first, then by path. None for a node that is not a file.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn links_of_file(self, file: NodeId) -> Result<Vec<FileLink>, StoreError> {
        let [structural, history, used] = LINK_KINDS.map(EdgeType::number);
        let rows: Vec<(String, i64, f64)> = self
            .connection()
            .prepare_cached(schema::LINKS_OF_FILE)
            .and_then(|mut statement| {
                statement
                    .query_map((file.get(), structural, history, used), |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading a file's links"))?;
        rows.into_iter()
            .map(|(path, kind, weight)| {
                let corrupt = || StoreError::Corrupt {
                    column: "edges.type",
                    row: file.get(),
                };
                let edge_type = u8::try_from(kind)
                    .ok()
                    .and_then(EdgeType::from_number)
                    .ok_or_else(corrupt)?;
                Ok(FileLink {
                    path: values::path(&path, "nodes.path", file.get())?,
                    edge_type,
                    weight,
                })
            })
            .collect()
    }
}
