//! What agents did to each file, added up: how many times they read it, when they last read or
//! edited it, and the tokens their responses took while they worked on it.
//!
//! Only what an agent's hooks reported counts: an edit the file watcher saw is the user's. Rows
//! hold node ids and numbers, nothing a file or a response said.

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::{schema, values};

/// What agents did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileReads {
    /// The file's node.
    pub node: NodeId,
    /// How many times agents read it.
    pub reads: u64,
    /// When an agent last read or edited it, in UTC milliseconds.
    pub last_ms: i64,
}

impl Queries<'_> {
    /// For every file node an agent read or edited, how often it was read and when it was last
    /// read or edited, by node. A node a file no longer has may be among them: events outlive
    /// the files they name.
    ///
    /// # Errors
    ///
    /// SQLite errors and counts that cannot be read back.
    pub fn file_reads(self) -> Result<Vec<FileReads>, StoreError> {
        let rows: Vec<(i64, i64, i64)> = self
            .connection()
            .prepare_cached(schema::FILE_READS)
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect()
            })
            .map_err(sqlite("counting reads"))?;
        rows.into_iter()
            .map(|(node, reads, last_ms)| {
                Ok(FileReads {
                    node: NodeId(node),
                    reads: values::number(reads, "events.kind", node)?,
                    last_ms,
                })
            })
            .collect()
    }

    /// The tokens each file's work took since `since_ms` (UTC milliseconds), by node: every
    /// response of a session counts in full for the file that session last read or edited at or
    /// before the response's time, as the usage rows count responses (one a forked session
    /// copied counts once). A response before the session's first read or edit counts for no
    /// file, and a file that took none is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors and counts that cannot be read back.
    pub fn file_tokens(self, since_ms: i64) -> Result<Vec<(NodeId, u64)>, StoreError> {
        let rows: Vec<(i64, i64)> = self
            .connection()
            .prepare_cached(schema::FILE_TOKENS)
            .and_then(|mut statement| {
                statement
                    .query_map([since_ms], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .map_err(sqlite("attributing tokens to files"))?;
        rows.into_iter()
            .map(|(node, tokens)| Ok((NodeId(node), values::number(tokens, "usage", node)?)))
            .collect()
    }
}
