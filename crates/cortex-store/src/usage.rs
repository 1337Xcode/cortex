//! What Cortex keeps of the tokens agents spent: one row per response a session's transcript
//! reports, numbered by its order in the transcript, with the model, the time, the counts and how
//! the response was served. Rows hold numbers, a session's id and short names the provider
//! reports, nothing a transcript says otherwise.

use cortex_core::Identifier;
use rusqlite::{Connection, Row};

use crate::error::{StoreError, sqlite};
use crate::queries::Queries;
use crate::schema;

/// The tokens one response of a session took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    /// The session whose transcript reported it.
    pub session_id: Identifier,
    /// Its place among the transcript's responses, from 1.
    pub turn: u32,
    /// When it was written, in UTC milliseconds.
    pub at_ms: i64,
    /// The model that answered, when the transcript names it with a name.
    pub model: Option<Identifier>,
    /// Input read at the full price.
    pub input: u64,
    /// Input written to the cache.
    pub cache_write: u64,
    /// Of `cache_write`, what was written to the one-hour cache, when the transcript says.
    pub cache_write_1h: Option<u64>,
    /// Input read from the cache.
    pub cache_read: u64,
    /// Output, reasoning included.
    pub output: u64,
    /// Of `output`, the tokens spent reasoning, when the transcript reports them.
    pub reasoning: Option<u64>,
    /// The service tier it was served at, when the transcript names it.
    pub service_tier: Option<Identifier>,
    /// The speed it was served at, when the transcript names it.
    pub speed: Option<Identifier>,
    /// Where it ran, when the transcript names it.
    pub inference_geo: Option<Identifier>,
}

/// Writes `rows`, each replacing the row of its session and turn if there is one: a transcript
/// read again gives its responses the same turns.
pub(crate) fn replace(connection: &Connection, rows: &[UsageRow]) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::USAGE_REPLACE)
        .map_err(sqlite("writing usage"))?;
    let count = |column, value: u64| {
        i64::try_from(value).map_err(|_| StoreError::TooLarge { column, value })
    };
    let name = |name: &Option<Identifier>| name.as_ref().map(Identifier::as_str).map(str::to_owned);
    for row in rows {
        let maybe =
            |column, value: Option<u64>| value.map(|value| count(column, value)).transpose();
        statement
            .execute((
                row.session_id.as_str(),
                row.turn,
                row.at_ms,
                name(&row.model),
                count("usage.input", row.input)?,
                count("usage.cache_write", row.cache_write)?,
                maybe("usage.cache_write_1h", row.cache_write_1h)?,
                count("usage.cache_read", row.cache_read)?,
                count("usage.output", row.output)?,
                maybe("usage.reasoning", row.reasoning)?,
                name(&row.service_tier),
                name(&row.speed),
                name(&row.inference_geo),
            ))
            .map_err(sqlite("writing usage"))?;
    }
    Ok(())
}

impl Queries<'_> {
    /// The usage of `session_id`, by turn.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn usage_of_session(self, session_id: &Identifier) -> Result<Vec<UsageRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::USAGE_OF_SESSION)
            .map_err(sqlite("reading usage"))?;
        let rows = statement
            .query_map([session_id.as_str()], read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading usage"))?;
        rows.into_iter().map(RawUsage::check).collect()
    }
}

/// A usage row as SQLite gives it, before its values are checked.
struct RawUsage {
    /// `rowid`, for reporting a damaged row.
    rowid: i64,
    /// `session_id`.
    session_id: String,
    /// `turn`.
    turn: i64,
    /// `ts`.
    at_ms: i64,
    /// `input`, `cache_write`, `cache_write_1h`, `cache_read`, `output` and `reasoning`.
    counts: (i64, i64, Option<i64>, i64, i64, Option<i64>),
    /// `model`, `service_tier`, `speed` and `inference_geo`.
    names: [Option<String>; 4],
}

/// One row of [`schema::USAGE_OF_SESSION`].
fn read(row: &Row<'_>) -> rusqlite::Result<RawUsage> {
    Ok(RawUsage {
        rowid: row.get(0)?,
        session_id: row.get(1)?,
        turn: row.get(2)?,
        at_ms: row.get(3)?,
        counts: (
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
        ),
        names: [row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?],
    })
}

impl RawUsage {
    /// The row, or the error that one of its values cannot be read back: a count below zero, a
    /// turn out of range, or a session id or name that is no identifier.
    fn check(self) -> Result<UsageRow, StoreError> {
        let row = self.rowid;
        let corrupt = |column| StoreError::Corrupt { column, row };
        let count = |column, value: i64| u64::try_from(value).map_err(|_| corrupt(column));
        let maybe =
            |column, value: Option<i64>| value.map(|value| count(column, value)).transpose();
        let name = |column, value: Option<String>| {
            value
                .map(|text| Identifier::new(&text))
                .transpose()
                .map_err(|_| corrupt(column))
        };
        let (input, cache_write, cache_write_1h, cache_read, output, reasoning) = self.counts;
        let [model, service_tier, speed, inference_geo] = self.names;
        Ok(UsageRow {
            session_id: Identifier::new(&self.session_id)
                .map_err(|_| corrupt("usage.session_id"))?,
            turn: u32::try_from(self.turn).map_err(|_| corrupt("usage.turn"))?,
            at_ms: self.at_ms,
            model: name("usage.model", model)?,
            input: count("usage.input", input)?,
            cache_write: count("usage.cache_write", cache_write)?,
            cache_write_1h: maybe("usage.cache_write_1h", cache_write_1h)?,
            cache_read: count("usage.cache_read", cache_read)?,
            output: count("usage.output", output)?,
            reasoning: maybe("usage.reasoning", reasoning)?,
            service_tier: name("usage.service_tier", service_tier)?,
            speed: name("usage.speed", speed)?,
            inference_geo: name("usage.inference_geo", inference_geo)?,
        })
    }
}
