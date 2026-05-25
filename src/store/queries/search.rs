//! Full-text search queries using FTS5 with BM25 ranking.
//!
//! Provides sanitized FTS5 search over the `nodes_fts` virtual table,
//! which mirrors the `nodes` table via sync triggers.

use rusqlite::Connection;
use serde::Serialize;

use crate::error::StoreError;

/// A single search result from the FTS5 index.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub rank: f64,
    /// Confidence score (0.0-1.0). For FTS5 results: 0.5 * normalized_bm25_rank.
    pub confidence: f64,
}

/// Sanitizes user input for safe use in FTS5 MATCH queries.
///
/// FTS5 has special syntax characters and keywords that could cause query
/// parse errors or unexpected behavior. This function escapes them by
/// wrapping individual terms in double quotes.
///
/// Special characters/keywords handled:
/// - `"` (double quote)
/// - `*` (prefix wildcard)
/// - `(` and `)` (grouping)
/// - Boolean operators: `OR`, `AND`, `NOT`, `NEAR`
fn sanitize_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Split on whitespace and process each token
    let tokens: Vec<String> = trimmed
        .split_whitespace()
        .map(|token| {
            // Check if the token is a boolean keyword (case-insensitive)
            let upper = token.to_uppercase();
            if matches!(upper.as_str(), "OR" | "AND" | "NOT" | "NEAR") {
                // Wrap keywords in quotes to treat them as literals
                return format!("\"{}\"", token);
            }

            // Check if token contains special FTS5 characters
            if token.contains('"')
                || token.contains('*')
                || token.contains('(')
                || token.contains(')')
                || token.contains('^')
                || token.contains(':')
            {
                // Escape embedded double quotes by doubling them, then wrap in quotes
                let escaped = token.replace('"', "\"\"");
                return format!("\"{}\"", escaped);
            }

            token.to_string()
        })
        .collect();

    tokens.join(" ")
}

/// Performs a full-text search over the `nodes_fts` index using BM25 ranking.
///
/// The query is sanitized to prevent FTS5 syntax injection. Results are
/// ordered by BM25 rank (lower rank = better match in FTS5).
///
/// # Arguments
///
/// * `conn` - A database connection (read-only is sufficient)
/// * `query` - The user-provided search query string
/// * `limit` - Maximum number of results to return
///
/// # Returns
///
/// A vector of `SearchResult` ordered by relevance (best match first).
pub fn search_fts(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, StoreError> {
    let sanitized = sanitize_fts_query(query);
    if sanitized.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT fqn, kind, file, rank FROM nodes_fts WHERE nodes_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare FTS5 search: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![sanitized, limit as i64], |row| {
            Ok(SearchResult {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                rank: row.get(3)?,
                confidence: 0.0, // placeholder, computed below
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("FTS5 search failed: {}", e),
        })?;

    let mut output = Vec::new();
    for result in results {
        let r = result.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read FTS5 result row: {}", e),
        })?;
        output.push(r);
    }

    // Compute confidence = 0.5 * normalized_bm25_rank
    // FTS5 rank is negative (more negative = better match), so we normalize:
    // best rank (most negative) gets normalized value 1.0, worst gets closer to 0.0
    if !output.is_empty() {
        // Find the best (most negative) rank - this is the max absolute value
        let best_rank = output.iter().map(|r| r.rank).fold(f64::INFINITY, f64::min); // most negative

        if best_rank < 0.0 {
            for result in &mut output {
                // normalized = result.rank / best_rank (both negative, so ratio is 0..1)
                let normalized = result.rank / best_rank;
                result.confidence = 0.5 * normalized;
            }
        }
    }

    // Sort by confidence descending
    output.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(output)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_empty_query() {
        assert_eq!(sanitize_fts_query(""), "");
        assert_eq!(sanitize_fts_query("   "), "");
    }

    #[test]
    fn test_sanitize_plain_query() {
        assert_eq!(sanitize_fts_query("hello world"), "hello world");
        assert_eq!(sanitize_fts_query("process_order"), "process_order");
    }

    #[test]
    fn test_sanitize_boolean_keywords() {
        assert_eq!(sanitize_fts_query("foo OR bar"), "foo \"OR\" bar");
        assert_eq!(sanitize_fts_query("NOT something"), "\"NOT\" something");
        assert_eq!(sanitize_fts_query("a AND b"), "a \"AND\" b");
        assert_eq!(sanitize_fts_query("NEAR test"), "\"NEAR\" test");
    }

    #[test]
    fn test_sanitize_special_characters() {
        assert_eq!(sanitize_fts_query("test*"), "\"test*\"");
        assert_eq!(sanitize_fts_query("(group)"), "\"(group)\"");
        assert_eq!(sanitize_fts_query("col:value"), "\"col:value\"");
    }

    #[test]
    fn test_sanitize_embedded_quotes() {
        assert_eq!(sanitize_fts_query("say\"hello"), "\"say\"\"hello\"");
    }

    #[test]
    fn test_sanitize_mixed_input() {
        assert_eq!(
            sanitize_fts_query("process* OR handler"),
            "\"process*\" \"OR\" handler"
        );
    }
}
