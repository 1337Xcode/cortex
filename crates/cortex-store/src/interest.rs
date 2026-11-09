//! What Cortex keeps of each node's interest: the score the node had at the task it was last
//! touched at, and the tier that score put it in.
//!
//! A score is kept as of a task rather than as of now, so that the decay every task brings is
//! applied when the score is read instead of to every row at every task. What that means for a
//! reader: a row's score is the score at `last_seq`, and the task that reads it brings it forward.
//! Rows hold node ids, numbers and a tier's number, nothing of a file's contents.

use cortex_core::InterestTier;
use rusqlite::{Connection, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::{Queries, json_array};
use crate::schema;

/// A node's interest, as of the task it was last updated at.
#[derive(Debug, Clone, PartialEq)]
pub struct InterestRow {
    /// The node.
    pub node: NodeId,
    /// Its score at `last_seq`, before the decay of the tasks since.
    pub score: f64,
    /// The number of the task the score is as of.
    pub last_seq: u64,
    /// The tier the score put it in when it was written.
    pub tier: InterestTier,
}

/// Writes `rows`, replacing the interest of each node that has some.
pub(crate) fn replace(connection: &Connection, rows: &[InterestRow]) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::INTEREST_REPLACE)
        .map_err(sqlite("writing interest"))?;
    for row in rows {
        let last_seq = i64::try_from(row.last_seq).map_err(|_| StoreError::TooLarge {
            column: "interest.last_seq",
            value: row.last_seq,
        })?;
        statement
            .execute((row.node.get(), row.score, last_seq, row.tier.number()))
            .map_err(sqlite("writing interest"))?;
    }
    Ok(())
}

impl Queries<'_> {
    /// The interest of each of `nodes` that has any, by node.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn interest_of(self, nodes: &[NodeId]) -> Result<Vec<InterestRow>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        self.interest(schema::INTEREST_OF, (listed,))
    }

    /// Every file node whose stored score is at least `from_score`, by node. Decay never raises a
    /// score, so no file stored below a threshold can stand above it now: these are every file
    /// that could still reach it.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn file_interest_from(self, from_score: f64) -> Result<Vec<InterestRow>, StoreError> {
        self.interest(schema::FILE_INTEREST_FROM, (from_score,))
    }

    /// The interest rows `sql` reads with `params`.
    /// At most `limit` nodes' interest after node `after`, by node, from the first when `after` is
    /// none.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn interest_after(
        self,
        after: Option<NodeId>,
        limit: u64,
    ) -> Result<Vec<InterestRow>, StoreError> {
        let after = after.map_or(-1, NodeId::get);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.interest(schema::INTEREST_AFTER, (after, limit))
    }

    /// The rows `sql` selects with `params`.
    fn interest(
        self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<InterestRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(sql)
            .map_err(sqlite("reading interest"))?;
        let rows = statement
            .query_map(params, read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading interest"))?;
        rows.into_iter().map(checked).collect()
    }
}

/// One row as stored.
fn read(row: &Row<'_>) -> rusqlite::Result<(i64, f64, i64, u8)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

/// The row these stored values make, or the error that one of them cannot be read back.
pub(crate) fn checked(
    (node, score, last_seq, tier): (i64, f64, i64, u8),
) -> Result<InterestRow, StoreError> {
    Ok(InterestRow {
        node: NodeId(node),
        score,
        last_seq: u64::try_from(last_seq).map_err(|_| StoreError::Corrupt {
            column: "interest.last_seq",
            row: node,
        })?,
        tier: InterestTier::from_number(tier).ok_or(StoreError::Corrupt {
            column: "interest.tier",
            row: node,
        })?,
    })
}
