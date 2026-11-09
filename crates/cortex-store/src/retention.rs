//! What upkeep stops keeping, deleted in bounded batches so that one batch never keeps a hook
//! waiting long: events older than their retention, episodes past their cap (those whose outcome
//! is unknown first, the oldest first), positions that can no longer hold, and facts retired long
//! ago with their anchors and the record of where they were given.

use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::schema;

/// Deletes those of the `limit` first events recorded that are older than `before_ms`: how many.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn delete_events(
    connection: &Connection,
    (before_ms, limit): (i64, u64),
) -> Result<u64, StoreError> {
    run(
        connection,
        schema::EVENTS_DELETE_BEFORE,
        (before_ms, as_sql(limit)),
        "deleting old events",
    )
}

/// Deletes at most `limit` episodes past the `keep` most worth keeping: those whose outcome is
/// unknown go first, then the oldest: how many.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn cap_episodes(
    connection: &Connection,
    (keep, limit): (u64, u64),
) -> Result<u64, StoreError> {
    run(
        connection,
        schema::EPISODES_OVER_CAP,
        (as_sql(keep), as_sql(limit)),
        "capping episodes",
    )
}

/// Deletes the positions last taken before `before_ms`: how many.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn delete_positions(connection: &Connection, before_ms: i64) -> Result<u64, StoreError> {
    run(
        connection,
        schema::POSITIONS_DELETE_BEFORE,
        [before_ms],
        "deleting old positions",
    )
}

/// Deletes at most `limit` facts retired before `before_ms`, with their anchors and the record of
/// the tasks they were given in: how many facts.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn delete_retired_facts(
    connection: &Connection,
    (before_ms, limit): (i64, u64),
) -> Result<u64, StoreError> {
    let ids: Vec<i64> = connection
        .prepare_cached(schema::FACTS_RETIRED_BEFORE)
        .and_then(|mut statement| {
            statement
                .query_map((before_ms, as_sql(limit)), |row| row.get(0))?
                .collect()
        })
        .map_err(sqlite("finding facts retired long ago"))?;
    if ids.is_empty() {
        return Ok(0);
    }
    let listed = serde_json::to_string(&ids).map_err(|_| StoreError::Corrupt {
        column: "facts.fact_id",
        row: 0,
    })?;
    for sql in [schema::FACT_ANCHORS_DELETE, schema::FACTS_GIVEN_DELETE] {
        run(
            connection,
            sql,
            [&listed],
            "deleting facts retired long ago",
        )?;
    }
    run(
        connection,
        schema::FACTS_DELETE,
        [&listed],
        "deleting facts retired long ago",
    )
}

/// `count` as SQLite takes a number, a count past what it holds read as the most it does.
fn as_sql(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Refreshes the query planner's statistics where they are stale.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn optimize(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(schema::OPTIMIZE)
        .map_err(sqlite("optimizing the database"))
}

/// Gives back to the file system at most `pages` free pages of the database (none from a
/// database that does not vacuum incrementally): how many it gave back.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn vacuum(connection: &Connection, pages: u32) -> Result<u64, StoreError> {
    let free = |connection: &Connection| -> Result<i64, StoreError> {
        connection
            .query_row(schema::FREE_PAGES, [], |row| row.get(0))
            .map_err(sqlite("reading the free pages"))
    };
    let before = free(connection)?;
    // The pragma takes its count inline: no parameter can stand for it. It gives back one page
    // each time it is stepped, so it is stepped until it is done.
    connection
        .prepare(&format!("PRAGMA incremental_vacuum({pages})"))
        .and_then(|mut statement| {
            let mut rows = statement.query([])?;
            while rows.next()?.is_some() {}
            Ok(())
        })
        .map_err(sqlite("giving back free pages"))?;
    let after = free(connection)?;
    Ok(u64::try_from(before.saturating_sub(after)).unwrap_or(0))
}

/// Runs the deletion `sql` with `params`: how many rows it removed.
fn run(
    connection: &Connection,
    sql: &'static str,
    params: impl rusqlite::Params,
    operation: &'static str,
) -> Result<u64, StoreError> {
    let removed = connection
        .prepare_cached(sql)
        .and_then(|mut statement| statement.execute(params))
        .map_err(sqlite(operation))?;
    Ok(u64::try_from(removed).unwrap_or(u64::MAX))
}
