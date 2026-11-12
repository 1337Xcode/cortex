//! `api/receipts`: the receipts report `cortex receipts` prints, for the map's receipts panel: the
//! held-out and treated arms with the comparison once each has its 30 tasks, the precision of the
//! work orders and warnings Cortex gave, and the receipt of each session that started in the last
//! [`WINDOW_DAYS`] days, the newest [`MAX_SESSIONS`] of them.
//!
//! Costs are estimated from the user's price file when there is one, read and never written (the
//! map must not create the user's files), and from the table Cortex ships when there is none, which
//! the answer says.

use axum::http::StatusCode;
use axum::response::Response;
use cortex_meter::{PriceTable, Recorded, SHIPPED_PRICES, Selection, load};
use serde::Serialize;

use super::Ask;
use super::reply::{Failure, json, respond};
use crate::map::site::Site;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// How many days back the sessions shown started: a week, as `cortex receipts` shows by default.
const WINDOW_DAYS: u32 = 7;

/// The most sessions shown, the newest: enough for a week of heavy work, few enough that the
/// answer stays small whatever the store holds.
const MAX_SESSIONS: usize = 100;

/// The milliseconds in a day.
const DAY_MS: i64 = 86_400_000;

/// The body.
#[derive(Debug, Serialize)]
struct Receipts {
    /// The shape's version.
    version: u8,
    /// How many days back the sessions shown started.
    window_days: u32,
    /// The prices costs were estimated from.
    prices: Prices,
    /// What the store holds.
    recorded: Recorded,
}

/// The prices costs were estimated from.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Prices {
    /// The user's price file.
    Read {
        /// The day its prices were read, as `YYYY-MM-DD`.
        updated: String,
    },
    /// The table Cortex ships: the user has no price file yet.
    Shipped {
        /// The day its prices were read, as `YYYY-MM-DD`.
        updated: String,
    },
    /// No table could be read, so no cost is estimated.
    Missing {
        /// Why.
        why: String,
    },
}

/// The receipts of the store `site` reads, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading the map's receipts", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        }
        let source = site.source();
        let (prices, table) = prices(source.pricing_path());
        let since_ms = source
            .now_ms()
            .saturating_sub(i64::from(WINDOW_DAYS) * DAY_MS);
        let reader = source.open()?;
        let mut recorded = reader
            .consistently(|queries| {
                Ok::<_, cortex_store::StoreError>(cortex_meter::recorded(
                    queries,
                    &Selection::StartedSince(since_ms),
                    table.as_ref(),
                ))
            })?
            .map_err(|error| Failure::Broken(error.to_string()))?;
        // Sessions come oldest first: the newest are the last.
        let extra = recorded.sessions.len().saturating_sub(MAX_SESSIONS);
        recorded.sessions.drain(..extra);
        json(&Receipts {
            version: VERSION,
            window_days: WINDOW_DAYS,
            prices,
            recorded,
        })
    })
}

/// The prices of the user's file at `path` when there is one, else the shipped table's.
fn prices(path: Option<&std::path::Path>) -> (Prices, Option<PriceTable>) {
    let read = match path {
        Some(path) => load(path),
        None => Ok(None),
    };
    match read {
        Ok(Some(table)) => (
            Prices::Read {
                updated: table.updated().to_owned(),
            },
            Some(table),
        ),
        Ok(None) => match PriceTable::parse(SHIPPED_PRICES) {
            Ok(table) => (
                Prices::Shipped {
                    updated: table.updated().to_owned(),
                },
                Some(table),
            ),
            Err(error) => (
                Prices::Missing {
                    why: error.to_string(),
                },
                None,
            ),
        },
        Err(error) => (
            Prices::Missing {
                why: error.to_string(),
            },
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// No home folder, or no price file in it, prices from the shipped table; the user's file wins
    /// when there is one, and is never written when there is none; a file that is no price table
    /// prices nothing and says why.
    #[test]
    fn the_user_s_prices_win_and_are_never_written() -> TestResult {
        let shipped = PriceTable::parse(SHIPPED_PRICES)?;
        let (none, table) = prices(None);
        assert!(matches!(none, Prices::Shipped { .. }), "{none:?}");
        assert_eq!(table.as_ref(), Some(&shipped));
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("pricing.toml");
        let (missing_file, _) = prices(Some(&path));
        assert!(
            matches!(missing_file, Prices::Shipped { .. }),
            "{missing_file:?}"
        );
        assert!(!path.exists());
        std::fs::write(&path, SHIPPED_PRICES)?;
        let (read, table) = prices(Some(&path));
        assert!(matches!(read, Prices::Read { .. }), "{read:?}");
        assert_eq!(table, Some(shipped));
        std::fs::write(&path, "not = [a table")?;
        let (broken, table) = prices(Some(&path));
        assert!(matches!(broken, Prices::Missing { .. }), "{broken:?}");
        assert_eq!(table, None);
        Ok(())
    }
}
