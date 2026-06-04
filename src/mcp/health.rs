//! Index health gate for the MCP server.
//!
//! Validates that the graph database is in a healthy state before serving
//! query results. If the index is empty or corrupt, all MCP tools return
//! a structured `HealthError` with a fallback suggestion instead of
//! confidently wrong answers.

use serde::{Deserialize, Serialize};

use crate::store::db::StoreManager;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Health check result with specific metrics.
///
/// Returned by `check_health()` to indicate whether the index is ready
/// to serve queries. When `healthy` is false, MCP tools should return
/// a `HealthError` instead of graph results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Whether the index passes all health checks.
    pub healthy: bool,
    /// Number of files indexed in the graph database.
    pub files_indexed: usize,
    /// Total number of nodes in the graph.
    pub node_count: usize,
    /// Total number of edges in the graph.
    pub edge_count: usize,
    /// Percentage of files with SCIP coverage (0.0 to 1.0).
    pub scip_coverage_percent: f64,
    /// List of active framework adapters.
    pub framework_coverage: Vec<String>,
    /// Human-readable reason if the health check failed.
    pub failure_reason: Option<String>,
    /// Suggested action to fix the health issue.
    pub suggested_action: Option<String>,
}

/// Structured error returned by all MCP tools when the health check fails.
///
/// Contains enough context for an AI agent to understand why the query
/// failed and what to do next (run `cortex index`, use grep, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthError {
    /// Short error identifier.
    pub error: String,
    /// Human-readable explanation of why the index is unhealthy.
    pub reason: String,
    /// Suggested action to restore health.
    pub suggested_action: String,
    /// Fallback suggestion with grep commands and file-read alternatives.
    pub fallback: FallbackSuggestion,
}

/// Structured fallback suggestion when confidence is below threshold
/// or the index is unhealthy.
///
/// Provides alternative approaches (grep commands, file reads) so the
/// AI agent can still make progress without Cortex graph data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FallbackSuggestion {
    /// Why the fallback is being suggested.
    pub reason: String,
    /// Shell grep commands the agent can run as alternatives.
    pub grep_commands: Vec<String>,
    /// Specific files the agent should consider reading.
    pub file_read_suggestions: Vec<String>,
    /// Explanation of why confidence is low or unavailable.
    pub confidence_explanation: String,
}

// ---------------------------------------------------------------------------
// Health check implementation
// ---------------------------------------------------------------------------

/// Run a health check against the index database.
///
/// Validates that:
/// - `files_indexed > 0` (at least one file has been indexed)
/// - `node_count > 0` (the graph contains symbols)
/// - `edge_count > 0` (the graph contains relationships)
///
/// Returns a `HealthStatus` with `healthy = true` if all checks pass,
/// or `healthy = false` with a failure reason and suggested action.
pub fn check_health(store: &StoreManager) -> HealthStatus {
    let conn = store.read_conn();

    // Query the index_health singleton row
    let result = conn.query_row(
        "SELECT files_indexed, node_count, edge_count, scip_coverage_percent, frameworks_detected \
         FROM index_health WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    );

    match result {
        Ok((files_indexed, node_count, edge_count, scip_coverage_percent, frameworks_json)) => {
            let files_indexed = files_indexed as usize;
            let node_count = node_count as usize;
            let edge_count = edge_count as usize;

            // Parse frameworks_detected JSON array
            let framework_coverage: Vec<String> =
                serde_json::from_str(&frameworks_json).unwrap_or_default();

            // Validate health conditions
            let mut failure_reasons = Vec::new();

            if files_indexed == 0 {
                failure_reasons.push("no files indexed".to_string());
            }
            if node_count == 0 {
                failure_reasons.push("no nodes in graph".to_string());
            }
            if edge_count == 0 {
                failure_reasons.push("no edges in graph".to_string());
            }

            let healthy = failure_reasons.is_empty();

            let (failure_reason, suggested_action) = if healthy {
                (None, None)
            } else {
                let reason = format!("Index health check failed: {}", failure_reasons.join(", "));
                let action = "Run `cortex index` to build the graph database.".to_string();
                (Some(reason), Some(action))
            };

            HealthStatus {
                healthy,
                files_indexed,
                node_count,
                edge_count,
                scip_coverage_percent,
                framework_coverage,
                failure_reason,
                suggested_action,
            }
        }
        Err(_) => {
            // index_health table doesn't exist or query failed
            HealthStatus {
                healthy: false,
                files_indexed: 0,
                node_count: 0,
                edge_count: 0,
                scip_coverage_percent: 0.0,
                framework_coverage: Vec::new(),
                failure_reason: Some(
                    "Index health check failed: unable to query index_health table".to_string(),
                ),
                suggested_action: Some(
                    "Run `cortex index` to initialize and build the graph database.".to_string(),
                ),
            }
        }
    }
}

/// Build a `HealthError` from an unhealthy `HealthStatus`.
///
/// This is a convenience function for MCP tool dispatch to convert
/// a failed health check into the structured error format.
pub fn build_health_error(status: &HealthStatus) -> HealthError {
    let reason = status
        .failure_reason
        .clone()
        .unwrap_or_else(|| "Unknown health failure".to_string());

    let suggested_action = status
        .suggested_action
        .clone()
        .unwrap_or_else(|| "Run `cortex index` to build the graph database.".to_string());

    HealthError {
        error: "index_unhealthy".to_string(),
        reason: reason.clone(),
        suggested_action,
        fallback: FallbackSuggestion {
            reason,
            grep_commands: vec![
                "grep -r '<search_term>' --include='*.rs' .".to_string(),
                "grep -r '<search_term>' --include='*.ts' .".to_string(),
                "grep -r '<search_term>' --include='*.py' .".to_string(),
            ],
            file_read_suggestions: vec![
                "Read the project's README.md for an overview".to_string(),
                "Check package.json, Cargo.toml, or pyproject.toml for dependencies".to_string(),
            ],
            confidence_explanation:
                "The Cortex index is empty or corrupt. Graph-based results are unavailable. \
                 Use grep and file reading as alternatives until the index is rebuilt."
                    .to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::store::db::StoreManager;

    /// Path to the migrations directory.
    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    /// Create a test StoreManager with migrations applied.
    fn setup_test_store() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();

        // Apply migrations
        let migrations_path = migrations_dir();
        let conn = store.write_conn();
        crate::store::migrations::run_migrations(&conn, &migrations_path).unwrap();
        drop(conn);

        (store, tmp)
    }

    #[test]
    fn test_check_health_empty_index_is_unhealthy() {
        let (store, _tmp) = setup_test_store();

        let status = check_health(&store);

        assert!(!status.healthy);
        assert_eq!(status.files_indexed, 0);
        assert_eq!(status.node_count, 0);
        assert_eq!(status.edge_count, 0);
        assert!(status.failure_reason.is_some());
        assert!(status.suggested_action.is_some());
    }

    #[test]
    fn test_check_health_populated_index_is_healthy() {
        let (store, _tmp) = setup_test_store();

        // Simulate a populated index
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 10, node_count = 50, edge_count = 30, \
                 scip_coverage_percent = 0.5, frameworks_detected = '[\"FastAPI\",\"React\"]' \
                 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        let status = check_health(&store);

        assert!(status.healthy);
        assert_eq!(status.files_indexed, 10);
        assert_eq!(status.node_count, 50);
        assert_eq!(status.edge_count, 30);
        assert!((status.scip_coverage_percent - 0.5).abs() < f64::EPSILON);
        assert_eq!(status.framework_coverage, vec!["FastAPI", "React"]);
        assert!(status.failure_reason.is_none());
        assert!(status.suggested_action.is_none());
    }

    #[test]
    fn test_check_health_partial_failure_no_edges() {
        let (store, _tmp) = setup_test_store();

        // Files and nodes exist but no edges
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 5, node_count = 20, edge_count = 0 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        let status = check_health(&store);

        assert!(!status.healthy);
        assert_eq!(status.files_indexed, 5);
        assert_eq!(status.node_count, 20);
        assert_eq!(status.edge_count, 0);
        assert!(status.failure_reason.as_ref().unwrap().contains("no edges"));
    }

    #[test]
    fn test_check_health_partial_failure_no_nodes() {
        let (store, _tmp) = setup_test_store();

        // Files exist but no nodes or edges
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 5, node_count = 0, edge_count = 0 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        let status = check_health(&store);

        assert!(!status.healthy);
        assert!(status.failure_reason.as_ref().unwrap().contains("no nodes"));
        assert!(status.failure_reason.as_ref().unwrap().contains("no edges"));
    }

    #[test]
    fn test_build_health_error_from_unhealthy_status() {
        let status = HealthStatus {
            healthy: false,
            files_indexed: 0,
            node_count: 0,
            edge_count: 0,
            scip_coverage_percent: 0.0,
            framework_coverage: Vec::new(),
            failure_reason: Some("no files indexed, no nodes in graph, no edges in graph".to_string()),
            suggested_action: Some("Run `cortex index` to build the graph database.".to_string()),
        };

        let error = build_health_error(&status);

        assert_eq!(error.error, "index_unhealthy");
        assert!(error.reason.contains("no files indexed"));
        assert!(error.suggested_action.contains("cortex index"));
        assert!(!error.fallback.grep_commands.is_empty());
        assert!(!error.fallback.file_read_suggestions.is_empty());
        assert!(!error.fallback.confidence_explanation.is_empty());
    }

    #[test]
    fn test_health_status_serialization_roundtrip() {
        let status = HealthStatus {
            healthy: true,
            files_indexed: 42,
            node_count: 100,
            edge_count: 200,
            scip_coverage_percent: 0.75,
            framework_coverage: vec!["Express".to_string(), "React".to_string()],
            failure_reason: None,
            suggested_action: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: HealthStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.healthy, status.healthy);
        assert_eq!(deserialized.files_indexed, status.files_indexed);
        assert_eq!(deserialized.node_count, status.node_count);
        assert_eq!(deserialized.edge_count, status.edge_count);
        assert!((deserialized.scip_coverage_percent - status.scip_coverage_percent).abs() < f64::EPSILON);
        assert_eq!(deserialized.framework_coverage, status.framework_coverage);
    }

    #[test]
    fn test_health_error_serialization_roundtrip() {
        let error = HealthError {
            error: "index_unhealthy".to_string(),
            reason: "no files indexed".to_string(),
            suggested_action: "Run `cortex index`".to_string(),
            fallback: FallbackSuggestion {
                reason: "Index is empty".to_string(),
                grep_commands: vec!["grep -r 'foo' .".to_string()],
                file_read_suggestions: vec!["README.md".to_string()],
                confidence_explanation: "No data available".to_string(),
            },
        };

        let json = serde_json::to_string(&error).unwrap();
        let deserialized: HealthError = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.error, error.error);
        assert_eq!(deserialized.reason, error.reason);
        assert_eq!(deserialized.suggested_action, error.suggested_action);
        assert_eq!(deserialized.fallback.reason, error.fallback.reason);
        assert_eq!(deserialized.fallback.grep_commands, error.fallback.grep_commands);
    }

    #[test]
    fn test_fallback_suggestion_serialization_roundtrip() {
        let fallback = FallbackSuggestion {
            reason: "Low confidence".to_string(),
            grep_commands: vec![
                "grep -rn 'pattern' src/".to_string(),
                "grep -rn 'other' lib/".to_string(),
            ],
            file_read_suggestions: vec!["src/main.rs".to_string()],
            confidence_explanation: "Results below MEDIUM threshold".to_string(),
        };

        let json = serde_json::to_string(&fallback).unwrap();
        let deserialized: FallbackSuggestion = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.reason, fallback.reason);
        assert_eq!(deserialized.grep_commands, fallback.grep_commands);
        assert_eq!(deserialized.file_read_suggestions, fallback.file_read_suggestions);
        assert_eq!(deserialized.confidence_explanation, fallback.confidence_explanation);
    }

    // ─── Property-Based Tests ─────────────────────────────────────────────────

    use proptest::prelude::*;

    /// **Feature: cortex-intelligence-overhaul**
    ///
    /// **Property 13: Health gate blocks unhealthy queries**
    ///
    /// For any MCP tool call when index health check fails (files_indexed=0 OR
    /// node_count=0 OR edge_count=0), check_health returns unhealthy.
    /// Generate arbitrary health states with at least one zero field and verify
    /// `healthy == false`.
    ///
    /// **Validates: Requirements 13.3**
    mod prop_health_gate {
        use super::*;

        /// Strategy that generates health states where at least one of
        /// files_indexed, node_count, or edge_count is zero.
        fn unhealthy_state_strategy() -> impl Strategy<Value = (i64, i64, i64)> {
            // Generate three values where at least one is zero
            prop_oneof![
                // files_indexed = 0
                (Just(0i64), 0i64..=1000, 0i64..=1000),
                // node_count = 0
                (0i64..=1000, Just(0i64), 0i64..=1000),
                // edge_count = 0
                (0i64..=1000, 0i64..=1000, Just(0i64)),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(200))]

            #[test]
            fn prop_health_gate_blocks_unhealthy(
                (files_indexed, node_count, edge_count) in unhealthy_state_strategy()
            ) {
                let (store, _tmp) = setup_test_store();

                // Set the index_health row to the generated unhealthy state
                {
                    let conn = store.write_conn();
                    conn.execute(
                        "UPDATE index_health SET files_indexed = ?1, node_count = ?2, edge_count = ?3 WHERE id = 1",
                        rusqlite::params![files_indexed, node_count, edge_count],
                    ).unwrap();
                }

                let status = check_health(&store);

                // At least one field is zero, so health must be false
                prop_assert!(
                    !status.healthy,
                    "Expected unhealthy for files={}, nodes={}, edges={}",
                    files_indexed, node_count, edge_count
                );
                prop_assert!(status.failure_reason.is_some());
                prop_assert!(status.suggested_action.is_some());
            }
        }
    }
}
