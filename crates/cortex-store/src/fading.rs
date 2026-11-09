//! The rows that fade with the tasks since they were last changed, read a page at a time for
//! upkeep to weigh, and removed once they have faded to nothing: links of use (co-access), and
//! nodes' interest. What counts as faded is the learning crate's rule; the store only pages and
//! removes.

use cortex_core::EdgeType;
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::{schema, values};

/// One link of use, as upkeep weighs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadingLink {
    /// Where it starts.
    pub src: NodeId,
    /// Where it ends.
    pub dst: NodeId,
    /// Its weight as of `last_seq`.
    pub weight: f64,
    /// The task it was last changed at.
    pub last_seq: u64,
}

impl Queries<'_> {
    /// At most `limit` links of use after `after` in the order of their ends, from the first when
    /// `after` is none.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn coaccess_links_after(
        self,
        after: Option<(NodeId, NodeId)>,
        limit: u64,
    ) -> Result<Vec<FadingLink>, StoreError> {
        let (src, dst) = after.map_or((-1, -1), |(src, dst)| (src.get(), dst.get()));
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<(i64, i64, f64, i64)> = self
            .connection()
            .prepare_cached(schema::COACCESS_AFTER)
            .and_then(|mut statement| {
                statement
                    .query_map((EdgeType::CoAccess.number(), src, dst, limit), |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect()
            })
            .map_err(sqlite("reading links of use"))?;
        rows.into_iter()
            .map(|(src, dst, weight, last_seq)| {
                Ok(FadingLink {
                    src: NodeId(src),
                    dst: NodeId(dst),
                    weight,
                    last_seq: values::number(last_seq, "edges.last_seq", src)?,
                })
            })
            .collect()
    }
}

/// Removes the links of use from each pair's first node to its second: how many.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn remove_links(
    connection: &Connection,
    links: &[(NodeId, NodeId)],
) -> Result<u64, StoreError> {
    let pairs: Vec<[i64; 2]> = links
        .iter()
        .map(|(src, dst)| [src.get(), dst.get()])
        .collect();
    let listed = serde_json::to_string(&pairs).map_err(|_| StoreError::Corrupt {
        column: "edges.src",
        row: 0,
    })?;
    let removed = connection
        .prepare_cached(schema::COACCESS_REMOVE)
        .and_then(|mut statement| statement.execute((EdgeType::CoAccess.number(), listed)))
        .map_err(sqlite("removing faded links"))?;
    Ok(u64::try_from(removed).unwrap_or(u64::MAX))
}

/// Removes the interest of `nodes`, which then have none: how many.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn remove_interest(
    connection: &Connection,
    nodes: &[NodeId],
) -> Result<u64, StoreError> {
    let ids: Vec<i64> = nodes.iter().map(|node| node.get()).collect();
    let listed = serde_json::to_string(&ids).map_err(|_| StoreError::Corrupt {
        column: "interest.node_id",
        row: 0,
    })?;
    let removed = connection
        .prepare_cached(schema::INTEREST_REMOVE)
        .and_then(|mut statement| statement.execute([listed]))
        .map_err(sqlite("removing faded interest"))?;
    Ok(u64::try_from(removed).unwrap_or(u64::MAX))
}
