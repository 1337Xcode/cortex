//! What tasks read next: for a file, the files tasks went on to read (or change) right after it,
//! before their first change, kept in the `transitions` table. The learner adds to them at each
//! task's end, the read path looks up the strongest from a file an agent reads, and upkeep removes
//! those that faded out.

use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::{Queries, json_array};
use crate::schema;
use crate::values;

/// One transition: from a file to the file tasks read next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionRow {
    /// The file read first.
    pub src: NodeId,
    /// The file read right after it.
    pub dst: NodeId,
    /// What the tasks that went this way added, as of `last_seq`.
    pub weight: f64,
    /// How many tasks went this way.
    pub count: u32,
    /// The task sequence number the weight was last updated at.
    pub last_seq: u64,
}

impl Queries<'_> {
    /// The transitions from each of `srcs`, by first file and then second.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn transitions_from(self, srcs: &[NodeId]) -> Result<Vec<TransitionRow>, StoreError> {
        let list = json_array(srcs.iter().map(|src| src.get().to_string()));
        self.transition_rows(schema::TRANSITIONS_FROM, (list,))
    }

    /// The transitions after the pair `after` (from the first when none), by first file and then
    /// second, at most `limit`: a page for upkeep.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn transitions_after(
        self,
        after: Option<(NodeId, NodeId)>,
        limit: u64,
    ) -> Result<Vec<TransitionRow>, StoreError> {
        let (src, dst) = after.map_or((-1, -1), |(src, dst)| (src.get(), dst.get()));
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.transition_rows(schema::TRANSITIONS_AFTER, (src, dst, limit))
    }

    /// The transitions `sql` reads with `params`.
    fn transition_rows(
        self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<TransitionRow>, StoreError> {
        let rows: Vec<(i64, i64, f64, i64, i64)> = self
            .connection()
            .prepare_cached(sql)
            .and_then(|mut statement| {
                statement
                    .query_map(params, |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    })?
                    .collect()
            })
            .map_err(sqlite("reading what tasks read next"))?;
        rows.into_iter()
            .map(|(src, dst, weight, count, last_seq)| {
                Ok(TransitionRow {
                    src: NodeId(src),
                    dst: NodeId(dst),
                    weight,
                    count: values::number(count, "transitions.count", src)?,
                    last_seq: values::number(last_seq, "transitions.last_seq", src)?,
                })
            })
            .collect()
    }
}

/// Sets each of `rows`, adding the pair when it is not there.
pub(crate) fn set(connection: &Connection, rows: &[TransitionRow]) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::TRANSITION_SET)
        .map_err(sqlite("writing what tasks read next"))?;
    for row in rows {
        let last_seq = i64::try_from(row.last_seq).map_err(|_| StoreError::TooLarge {
            column: "transitions.last_seq",
            value: row.last_seq,
        })?;
        statement
            .execute((
                row.src.get(),
                row.dst.get(),
                row.weight,
                row.count,
                last_seq,
            ))
            .map_err(sqlite("writing what tasks read next"))?;
    }
    Ok(())
}

/// Removes the transitions from each pair's first file to its second: how many.
pub(crate) fn remove(
    connection: &Connection,
    pairs: &[(NodeId, NodeId)],
) -> Result<u64, StoreError> {
    if pairs.is_empty() {
        return Ok(0);
    }
    let list = json_array(
        pairs
            .iter()
            .map(|(src, dst)| format!("[{},{}]", src.get(), dst.get())),
    );
    let removed = connection
        .execute(schema::TRANSITIONS_REMOVE, (list,))
        .map_err(sqlite("removing what tasks read next"))?;
    Ok(u64::try_from(removed).unwrap_or(0))
}
