//! Persistent token savings store.
//!
//! Records per-tool-call token usage and savings into the `token_savings` SQLite
//! table, and provides query functions for cumulative totals, per-tool breakdowns,
//! daily time-series, and naive cost estimates.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Model pricing configuration (dollars per million tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Map of model name to price per million tokens (USD).
    pub prices: HashMap<String, f64>,
}

impl Default for ModelPricing {
    fn default() -> Self {
        let mut prices = HashMap::new();
        prices.insert("gpt-4o".to_string(), 2.50);
        prices.insert("claude-sonnet".to_string(), 3.00);
        prices.insert("gemini-pro".to_string(), 1.25);
        Self { prices }
    }
}

/// Time period for aggregation queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimePeriod {
    /// Last hour.
    Hour,
    /// Last 24 hours.
    Day,
    /// Last 7 days.
    Week,
    /// All recorded history.
    AllTime,
}

/// Cumulative token savings totals for a given time period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CumulativeTotals {
    /// Total tokens consumed across all tool calls.
    pub total_tokens_used: u64,
    /// Total tokens saved across all tool calls.
    pub total_tokens_saved: u64,
    /// Total number of tool calls recorded.
    pub total_tool_calls: u64,
    /// Naive cost estimate in USD based on model pricing.
    pub naive_cost_estimate: f64,
}

/// Per-tool breakdown of token savings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBreakdown {
    /// Name of the tool.
    pub tool_name: String,
    /// Number of times this tool was called.
    pub call_count: u64,
    /// Total tokens saved by this tool.
    pub total_tokens_saved: u64,
    /// Average tokens saved per call.
    pub average_tokens_saved: f64,
}

/// Daily savings aggregate for time-series display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySavings {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Total tokens saved on this day.
    pub tokens_saved: u64,
    /// Total tokens used on this day.
    pub tokens_used: u64,
    /// Number of tool calls on this day.
    pub tool_calls: u64,
}

/// Record a tool call's token usage into the `token_savings` table.
pub fn record_savings(
    conn: &Connection,
    tool_name: &str,
    tokens_used: usize,
    tokens_saved: usize,
    agent_id: &str,
    model_name: &str,
) -> Result<(), StoreError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO token_savings (tool_name, tokens_used, tokens_saved, timestamp, agent_id, model_name) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![tool_name, tokens_used as i64, tokens_saved as i64, now, agent_id, model_name],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to record savings: {}", e),
    })?;

    Ok(())
}

/// Compute the UTC Unix timestamp boundary for a given time period.
/// Returns the earliest timestamp to include in the query.
fn period_boundary(period: TimePeriod) -> Option<i64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    match period {
        TimePeriod::Hour => Some(now - 3600),
        TimePeriod::Day => Some(now - 86400),
        TimePeriod::Week => Some(now - 604800),
        TimePeriod::AllTime => None,
    }
}

/// Query cumulative totals grouped by time period.
///
/// If `agent_id` is `Some`, filters to that agent only.
pub fn query_cumulative(
    conn: &Connection,
    agent_id: Option<&str>,
    period: TimePeriod,
) -> Result<CumulativeTotals, StoreError> {
    let boundary = period_boundary(period);

    // Build the query dynamically based on filters
    let mut sql = String::from(
        "SELECT COALESCE(SUM(tokens_used), 0), COALESCE(SUM(tokens_saved), 0), COUNT(*) \
         FROM token_savings WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(b) = boundary {
        sql.push_str(" AND timestamp >= ?");
        params.push(Box::new(b));
    }

    if let Some(aid) = agent_id {
        sql.push_str(" AND agent_id = ?");
        params.push(Box::new(aid.to_string()));
    }

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare query_cumulative: {}", e),
    })?;

    let result = stmt
        .query_row(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute query_cumulative: {}", e),
        })?;

    let (total_used, total_saved, total_calls) = result;
    let pricing = ModelPricing::default();
    let naive_cost_estimate = compute_cost_estimate(total_saved as usize, None, &pricing);

    Ok(CumulativeTotals {
        total_tokens_used: total_used as u64,
        total_tokens_saved: total_saved as u64,
        total_tool_calls: total_calls as u64,
        naive_cost_estimate,
    })
}

/// Query per-tool breakdown of token savings.
pub fn query_per_tool(conn: &Connection) -> Result<Vec<ToolBreakdown>, StoreError> {
    let sql = "SELECT tool_name, COUNT(*) as call_count, \
               COALESCE(SUM(tokens_saved), 0) as total_saved, \
               COALESCE(AVG(tokens_saved), 0.0) as avg_saved \
               FROM token_savings \
               GROUP BY tool_name \
               ORDER BY total_saved DESC";

    let mut stmt = conn.prepare(sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare query_per_tool: {}", e),
    })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ToolBreakdown {
                tool_name: row.get(0)?,
                call_count: row.get::<_, i64>(1)? as u64,
                total_tokens_saved: row.get::<_, i64>(2)? as u64,
                average_tokens_saved: row.get(3)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute query_per_tool: {}", e),
        })?;

    let mut breakdowns = Vec::new();
    for row in rows {
        breakdowns.push(row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read tool breakdown row: {}", e),
        })?);
    }

    Ok(breakdowns)
}

/// Query daily savings aggregates for the last `days` days.
pub fn query_daily_series(conn: &Connection, days: u32) -> Result<Vec<DailySavings>, StoreError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let boundary = now - (days as i64 * 86400);

    // Group by date using SQLite's date function on Unix timestamps.
    // We use strftime to convert the integer timestamp to a date string.
    let sql = "SELECT date(timestamp, 'unixepoch') as day, \
               COALESCE(SUM(tokens_saved), 0), \
               COALESCE(SUM(tokens_used), 0), \
               COUNT(*) \
               FROM token_savings \
               WHERE timestamp >= ?1 \
               GROUP BY day \
               ORDER BY day ASC";

    let mut stmt = conn.prepare(sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare query_daily_series: {}", e),
    })?;

    let rows = stmt
        .query_map(rusqlite::params![boundary], |row| {
            Ok(DailySavings {
                date: row.get(0)?,
                tokens_saved: row.get::<_, i64>(1)? as u64,
                tokens_used: row.get::<_, i64>(2)? as u64,
                tool_calls: row.get::<_, i64>(3)? as u64,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute query_daily_series: {}", e),
        })?;

    let mut series = Vec::new();
    for row in rows {
        series.push(row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read daily savings row: {}", e),
        })?);
    }

    Ok(series)
}

/// Compute naive cost estimate based on tokens saved and model pricing.
///
/// When no model is specified, defaults to Claude Sonnet pricing ($3.00/M tokens).
pub fn compute_cost_estimate(
    tokens_saved: usize,
    model: Option<&str>,
    pricing: &ModelPricing,
) -> f64 {
    let price_per_million = model
        .and_then(|m| pricing.prices.get(m))
        .copied()
        .unwrap_or(3.00); // Default to Claude Sonnet pricing
    (tokens_saved as f64 / 1_000_000.0) * price_per_million
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// Returns the path to the migrations directory relative to the crate root.
    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    /// Creates an in-memory SQLite connection with migrations applied.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        // Apply migrations in order
        let mut entries: Vec<_> = std::fs::read_dir(migrations_dir())
            .expect("failed to read migrations dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let sql = std::fs::read_to_string(entry.path())
                .unwrap_or_else(|_| panic!("failed to read {:?}", entry.path()));
            conn.execute_batch(&sql)
                .unwrap_or_else(|e| panic!("failed to apply {:?}: {}", entry.path(), e));
        }
        conn
    }

    #[test]
    fn test_model_pricing_default() {
        let pricing = ModelPricing::default();
        assert_eq!(pricing.prices.get("gpt-4o"), Some(&2.50));
        assert_eq!(pricing.prices.get("claude-sonnet"), Some(&3.00));
        assert_eq!(pricing.prices.get("gemini-pro"), Some(&1.25));
        assert_eq!(pricing.prices.len(), 3);
    }

    #[test]
    fn test_compute_cost_estimate_with_model() {
        let pricing = ModelPricing::default();
        // 1,000,000 tokens saved at $2.50/M = $2.50
        let cost = compute_cost_estimate(1_000_000, Some("gpt-4o"), &pricing);
        assert!((cost - 2.50).abs() < f64::EPSILON);

        // 500,000 tokens saved at $3.00/M = $1.50
        let cost = compute_cost_estimate(500_000, Some("claude-sonnet"), &pricing);
        assert!((cost - 1.50).abs() < f64::EPSILON);

        // 2,000,000 tokens saved at $1.25/M = $2.50
        let cost = compute_cost_estimate(2_000_000, Some("gemini-pro"), &pricing);
        assert!((cost - 2.50).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_cost_estimate_default_model() {
        let pricing = ModelPricing::default();
        // No model specified -> defaults to Claude Sonnet $3.00/M
        let cost = compute_cost_estimate(1_000_000, None, &pricing);
        assert!((cost - 3.00).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_cost_estimate_unknown_model() {
        let pricing = ModelPricing::default();
        // Unknown model -> defaults to $3.00/M
        let cost = compute_cost_estimate(1_000_000, Some("unknown-model"), &pricing);
        assert!((cost - 3.00).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_cost_estimate_zero_tokens() {
        let pricing = ModelPricing::default();
        let cost = compute_cost_estimate(0, Some("gpt-4o"), &pricing);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_record_savings_inserts_row() {
        let conn = setup_db();
        record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o")
            .expect("record_savings should succeed");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_savings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (tool, used, saved, agent, model): (String, i64, i64, String, String) = conn
            .query_row(
                "SELECT tool_name, tokens_used, tokens_saved, agent_id, model_name FROM token_savings",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(tool, "search_symbols");
        assert_eq!(used, 100);
        assert_eq!(saved, 750);
        assert_eq!(agent, "cursor");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn test_record_savings_sets_timestamp() {
        let conn = setup_db();
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet").unwrap();

        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let ts: i64 = conn
            .query_row("SELECT timestamp FROM token_savings", [], |row| row.get(0))
            .unwrap();
        assert!(ts >= before && ts <= after);
    }

    #[test]
    fn test_query_cumulative_empty_db() {
        let conn = setup_db();
        let totals = query_cumulative(&conn, None, TimePeriod::AllTime).unwrap();
        assert_eq!(totals.total_tokens_used, 0);
        assert_eq!(totals.total_tokens_saved, 0);
        assert_eq!(totals.total_tool_calls, 0);
        assert!((totals.naive_cost_estimate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_query_cumulative_all_time() {
        let conn = setup_db();
        record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "ask", 200, 1500, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "trace_callers", 50, 500, "kiro", "claude-sonnet").unwrap();

        let totals = query_cumulative(&conn, None, TimePeriod::AllTime).unwrap();
        assert_eq!(totals.total_tokens_used, 350);
        assert_eq!(totals.total_tokens_saved, 2750);
        assert_eq!(totals.total_tool_calls, 3);
        // 2750 / 1_000_000 * 3.00 = 0.00825
        assert!((totals.naive_cost_estimate - 0.00825).abs() < 1e-10);
    }

    #[test]
    fn test_query_cumulative_filtered_by_agent() {
        let conn = setup_db();
        record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet").unwrap();

        let totals = query_cumulative(&conn, Some("cursor"), TimePeriod::AllTime).unwrap();
        assert_eq!(totals.total_tokens_used, 100);
        assert_eq!(totals.total_tokens_saved, 750);
        assert_eq!(totals.total_tool_calls, 1);
    }

    #[test]
    fn test_query_per_tool_empty_db() {
        let conn = setup_db();
        let breakdown = query_per_tool(&conn).unwrap();
        assert!(breakdown.is_empty());
    }

    #[test]
    fn test_query_per_tool_aggregation() {
        let conn = setup_db();
        record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "search_symbols", 120, 800, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "ask", 200, 1500, "kiro", "claude-sonnet").unwrap();

        let breakdown = query_per_tool(&conn).unwrap();
        assert_eq!(breakdown.len(), 2);

        // Ordered by total_saved DESC: search_symbols (1550) > ask (1500)
        assert_eq!(breakdown[0].tool_name, "search_symbols");
        assert_eq!(breakdown[0].call_count, 2);
        assert_eq!(breakdown[0].total_tokens_saved, 1550);
        assert!((breakdown[0].average_tokens_saved - 775.0).abs() < f64::EPSILON);

        assert_eq!(breakdown[1].tool_name, "ask");
        assert_eq!(breakdown[1].call_count, 1);
        assert_eq!(breakdown[1].total_tokens_saved, 1500);
        assert!((breakdown[1].average_tokens_saved - 1500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_query_daily_series_empty_db() {
        let conn = setup_db();
        let series = query_daily_series(&conn, 30).unwrap();
        assert!(series.is_empty());
    }

    #[test]
    fn test_query_daily_series_groups_by_day() {
        let conn = setup_db();
        // Insert records with known timestamps (today)
        record_savings(&conn, "search_symbols", 100, 750, "cursor", "gpt-4o").unwrap();
        record_savings(&conn, "ask", 200, 1500, "cursor", "gpt-4o").unwrap();

        let series = query_daily_series(&conn, 30).unwrap();
        // Both records are from today, so we should have 1 day entry
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].tokens_saved, 2250);
        assert_eq!(series[0].tokens_used, 300);
        assert_eq!(series[0].tool_calls, 2);
    }

    #[test]
    fn test_query_daily_series_respects_days_boundary() {
        let conn = setup_db();
        // Insert a record with a timestamp from 40 days ago (outside 30-day window)
        let old_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (40 * 86400);

        conn.execute(
            "INSERT INTO token_savings (tool_name, tokens_used, tokens_saved, timestamp, agent_id, model_name) \
             VALUES ('old_tool', 50, 300, ?1, 'agent', 'model')",
            rusqlite::params![old_ts],
        )
        .unwrap();

        // Insert a recent record
        record_savings(&conn, "new_tool", 100, 750, "cursor", "gpt-4o").unwrap();

        let series = query_daily_series(&conn, 30).unwrap();
        // Only the recent record should appear
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].tokens_saved, 750);
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    /// Helper to insert a savings record with a specific timestamp directly via SQL,
    /// bypassing the `record_savings` function which always uses `now()`.
    fn insert_savings_at(
        conn: &Connection,
        tool_name: &str,
        tokens_used: i64,
        tokens_saved: i64,
        timestamp: i64,
        agent_id: &str,
        model_name: &str,
    ) {
        conn.execute(
            "INSERT INTO token_savings (tool_name, tokens_used, tokens_saved, timestamp, agent_id, model_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![tool_name, tokens_used, tokens_saved, timestamp, agent_id, model_name],
        )
        .expect("insert_savings_at should succeed");
    }

    // **Property 12: Savings aggregation queries are correct**
    //
    // For any set of token_savings records inserted, the cumulative totals query
    // SHALL return total_tokens_used equal to the sum of all tokens_used values,
    // total_tokens_saved equal to the sum of all tokens_saved values, and
    // total_tool_calls equal to the count of records, when filtered by the
    // specified agent_id and time period.
    //
    // **Validates: Requirements 6.4, 6.5**

    proptest! {
        /// Property 12a: For any set of records with the same agent_id inserted
        /// with recent timestamps, query_cumulative with AllTime returns correct
        /// sums and count.
        ///
        /// **Validates: Requirements 6.4, 6.5**
        #[test]
        fn prop_savings_aggregation_alltime_correct(
            records in proptest::collection::vec(
                (1i64..=100_000i64, 1i64..=1_000_000i64),
                1..=20
            ),
        ) {
            let conn = setup_db();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let expected_used: i64 = records.iter().map(|(u, _)| u).sum();
            let expected_saved: i64 = records.iter().map(|(_, s)| s).sum();
            let expected_count = records.len() as u64;

            for (i, (tokens_used, tokens_saved)) in records.iter().enumerate() {
                insert_savings_at(
                    &conn,
                    &format!("tool_{}", i % 5),
                    *tokens_used,
                    *tokens_saved,
                    now - (i as i64), // slightly staggered but all recent
                    "test_agent",
                    "claude-sonnet",
                );
            }

            let totals = query_cumulative(&conn, Some("test_agent"), TimePeriod::AllTime).unwrap();

            prop_assert_eq!(totals.total_tokens_used, expected_used as u64);
            prop_assert_eq!(totals.total_tokens_saved, expected_saved as u64);
            prop_assert_eq!(totals.total_tool_calls, expected_count);
        }

        /// Property 12b: For records split across two agents, query_cumulative
        /// filtered by one agent returns only that agent's totals.
        ///
        /// **Validates: Requirements 6.4, 6.5**
        #[test]
        fn prop_savings_aggregation_agent_filter(
            agent_a_records in proptest::collection::vec(
                (1i64..=50_000i64, 1i64..=500_000i64),
                1..=10
            ),
            agent_b_records in proptest::collection::vec(
                (1i64..=50_000i64, 1i64..=500_000i64),
                1..=10
            ),
        ) {
            let conn = setup_db();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let expected_a_used: i64 = agent_a_records.iter().map(|(u, _)| u).sum();
            let expected_a_saved: i64 = agent_a_records.iter().map(|(_, s)| s).sum();
            let expected_a_count = agent_a_records.len() as u64;

            for (i, (tokens_used, tokens_saved)) in agent_a_records.iter().enumerate() {
                insert_savings_at(
                    &conn,
                    "tool_a",
                    *tokens_used,
                    *tokens_saved,
                    now - (i as i64),
                    "agent_alpha",
                    "gpt-4o",
                );
            }

            for (i, (tokens_used, tokens_saved)) in agent_b_records.iter().enumerate() {
                insert_savings_at(
                    &conn,
                    "tool_b",
                    *tokens_used,
                    *tokens_saved,
                    now - (i as i64),
                    "agent_beta",
                    "gemini-pro",
                );
            }

            let totals_a = query_cumulative(&conn, Some("agent_alpha"), TimePeriod::AllTime).unwrap();

            prop_assert_eq!(totals_a.total_tokens_used, expected_a_used as u64);
            prop_assert_eq!(totals_a.total_tokens_saved, expected_a_saved as u64);
            prop_assert_eq!(totals_a.total_tool_calls, expected_a_count);
        }

        /// Property 12c: query_cumulative with no agent filter returns the sum
        /// of all records regardless of agent_id.
        ///
        /// **Validates: Requirements 6.4, 6.5**
        #[test]
        fn prop_savings_aggregation_no_agent_filter(
            records in proptest::collection::vec(
                (1i64..=100_000i64, 1i64..=1_000_000i64, 0u8..=2u8),
                1..=15
            ),
        ) {
            let conn = setup_db();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let agents = ["cursor", "kiro", "copilot"];
            let expected_used: i64 = records.iter().map(|(u, _, _)| u).sum();
            let expected_saved: i64 = records.iter().map(|(_, s, _)| s).sum();
            let expected_count = records.len() as u64;

            for (i, (tokens_used, tokens_saved, agent_idx)) in records.iter().enumerate() {
                insert_savings_at(
                    &conn,
                    "some_tool",
                    *tokens_used,
                    *tokens_saved,
                    now - (i as i64),
                    agents[*agent_idx as usize],
                    "claude-sonnet",
                );
            }

            let totals = query_cumulative(&conn, None, TimePeriod::AllTime).unwrap();

            prop_assert_eq!(totals.total_tokens_used, expected_used as u64);
            prop_assert_eq!(totals.total_tokens_saved, expected_saved as u64);
            prop_assert_eq!(totals.total_tool_calls, expected_count);
        }
    }

    // **Property 13: Savings cost computation is correct**
    //
    // For any tokens_saved value and model pricing configuration, the
    // naive_cost_estimate SHALL equal (tokens_saved / 1_000_000.0) *
    // price_per_million_tokens for the specified model. When no model is
    // specified, the default Claude Sonnet pricing ($3.00/M) SHALL be used.
    //
    // **Validates: Requirements 6.6, 6.8**

    proptest! {
        /// Property 13a: For any tokens_saved and any known model, the cost
        /// estimate equals (tokens_saved / 1_000_000.0) * price_per_million.
        ///
        /// **Validates: Requirements 6.6, 6.8**
        #[test]
        fn prop_cost_computation_known_model(
            tokens_saved in 0usize..=100_000_000,
            model_idx in 0usize..=2,
        ) {
            let pricing = ModelPricing::default();
            let models = ["gpt-4o", "claude-sonnet", "gemini-pro"];
            let model = models[model_idx];
            let price_per_million = pricing.prices[model];

            let result = compute_cost_estimate(tokens_saved, Some(model), &pricing);
            let expected = (tokens_saved as f64 / 1_000_000.0) * price_per_million;

            prop_assert!(
                (result - expected).abs() < 1e-10,
                "For tokens_saved={}, model={}: expected {} but got {}",
                tokens_saved, model, expected, result
            );
        }

        /// Property 13b: When no model is specified, the cost estimate uses
        /// the default Claude Sonnet pricing ($3.00/M).
        ///
        /// **Validates: Requirements 6.6, 6.8**
        #[test]
        fn prop_cost_computation_default_model(
            tokens_saved in 0usize..=100_000_000,
        ) {
            let pricing = ModelPricing::default();
            let result = compute_cost_estimate(tokens_saved, None, &pricing);
            let expected = (tokens_saved as f64 / 1_000_000.0) * 3.00;

            prop_assert!(
                (result - expected).abs() < 1e-10,
                "For tokens_saved={} with no model: expected {} but got {}",
                tokens_saved, expected, result
            );
        }

        /// Property 13c: When an unknown model is specified, the cost estimate
        /// falls back to the default Claude Sonnet pricing ($3.00/M).
        ///
        /// **Validates: Requirements 6.6, 6.8**
        #[test]
        fn prop_cost_computation_unknown_model(
            tokens_saved in 0usize..=100_000_000,
            model_name in "[a-z]{3,10}",
        ) {
            let pricing = ModelPricing::default();
            // Only test with model names that are NOT in the pricing map
            prop_assume!(!pricing.prices.contains_key(&model_name));

            let result = compute_cost_estimate(tokens_saved, Some(&model_name), &pricing);
            let expected = (tokens_saved as f64 / 1_000_000.0) * 3.00;

            prop_assert!(
                (result - expected).abs() < 1e-10,
                "For tokens_saved={} with unknown model '{}': expected {} but got {}",
                tokens_saved, model_name, expected, result
            );
        }

        /// Property 13d: Cost estimate is always non-negative for any valid input.
        ///
        /// **Validates: Requirements 6.6, 6.8**
        #[test]
        fn prop_cost_computation_non_negative(
            tokens_saved in 0usize..=100_000_000,
            price_per_million in 0.0f64..=100.0,
        ) {
            let mut pricing = ModelPricing {
                prices: HashMap::new(),
            };
            pricing.prices.insert("custom".to_string(), price_per_million);

            let result = compute_cost_estimate(tokens_saved, Some("custom"), &pricing);
            prop_assert!(
                result >= 0.0,
                "Cost estimate should be non-negative, got {} for tokens_saved={}, price={}",
                result, tokens_saved, price_per_million
            );
        }
    }
}
