//! Benchmark infrastructure for Cortex correctness and token savings validation.
//!
//! Provides a benchmark suite format (JSON-based test cases with ground-truth answers),
//! a runner that compares Cortex tool output against ground truth, and reporting of
//! pass rate, per-tool accuracy, and average token savings.
//!
//! Satisfies Requirements 26.1, 26.2.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::mcp::dispatch::dispatch_tool;
use crate::store::db::StoreManager;

// ---------------------------------------------------------------------------
// Benchmark Suite Types
// ---------------------------------------------------------------------------

/// A complete benchmark suite containing multiple test cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    /// Human-readable name for this suite.
    pub name: String,
    /// Description of what this suite validates.
    pub description: String,
    /// The test cases in this suite.
    pub cases: Vec<BenchmarkCase>,
}

/// A single benchmark test case with input, expected output, and tolerance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkCase {
    /// Unique identifier for this case.
    pub id: String,
    /// The MCP tool being tested (e.g., "trace_callers", "blast_radius", "ask", "get_task_context").
    pub tool_name: String,
    /// Input arguments for the tool (JSON object matching tool parameters).
    pub input_args: serde_json::Value,
    /// Expected output (ground truth) to compare against.
    pub expected_output: ExpectedOutput,
    /// Tolerance settings for comparison.
    #[serde(default)]
    pub tolerance: Tolerance,
}

/// Ground truth expected output for a benchmark case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedOutput {
    /// Expected FQNs that should appear in results (for trace/blast tools).
    #[serde(default)]
    pub expected_fqns: Vec<String>,
    /// Minimum number of results expected.
    #[serde(default)]
    pub min_results: Option<usize>,
    /// Maximum number of results expected.
    #[serde(default)]
    pub max_results: Option<usize>,
    /// Expected keywords that should appear in the response text.
    #[serde(default)]
    pub expected_keywords: Vec<String>,
    /// Whether a fallback suggestion is expected (for low-confidence scenarios).
    #[serde(default)]
    pub expects_fallback: bool,
    /// Expected confidence range [min, max].
    #[serde(default)]
    pub confidence_range: Option<(f64, f64)>,
}

/// Tolerance settings for comparing actual vs expected output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tolerance {
    /// Fraction of expected_fqns that must be present (0.0 to 1.0). Default: 0.7.
    #[serde(default = "default_fqn_match_ratio")]
    pub fqn_match_ratio: f64,
    /// Allow extra results beyond expected (default: true).
    #[serde(default = "default_allow_extra")]
    pub allow_extra_results: bool,
}

fn default_fqn_match_ratio() -> f64 {
    0.7
}

fn default_allow_extra() -> bool {
    true
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            fqn_match_ratio: 0.7,
            allow_extra_results: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmark Results
// ---------------------------------------------------------------------------

/// Result of running the entire benchmark suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Overall pass rate (correct / total).
    pub pass_rate: f64,
    /// Per-tool accuracy breakdown.
    pub per_tool_accuracy: HashMap<String, ToolAccuracy>,
    /// Average token savings vs grep baseline across all cases.
    pub avg_token_savings: f64,
    /// Total cases run.
    pub total_cases: usize,
    /// Total cases passed.
    pub cases_passed: usize,
    /// Individual case results.
    pub case_results: Vec<CaseResult>,
}

/// Accuracy metrics for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAccuracy {
    /// Tool name.
    pub tool_name: String,
    /// Number of cases for this tool.
    pub total: usize,
    /// Number of cases passed.
    pub passed: usize,
    /// Pass rate for this tool.
    pub accuracy: f64,
}

/// Result of a single benchmark case execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    /// Case ID.
    pub case_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Whether this case passed.
    pub passed: bool,
    /// Reason for failure (if failed).
    pub failure_reason: Option<String>,
    /// Token savings for this case (baseline - actual).
    pub token_savings: i64,
}

// ---------------------------------------------------------------------------
// Benchmark Runner
// ---------------------------------------------------------------------------

/// Load a benchmark suite from a JSON file.
pub fn load_suite(path: &Path) -> Result<BenchmarkSuite, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read benchmark suite at '{}': {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse benchmark suite: {}", e))
}

/// Run a benchmark suite against the current Cortex index.
///
/// For each case, dispatches the tool call via the MCP dispatch system,
/// compares output to ground truth, and computes pass rate, per-tool accuracy,
/// and token savings.
pub fn run_benchmark(store: &StoreManager, suite: &BenchmarkSuite) -> BenchmarkResult {
    let mut case_results = Vec::new();
    let mut per_tool_counts: HashMap<String, (usize, usize)> = HashMap::new();
    let mut total_savings: i64 = 0;

    for case in &suite.cases {
        let result = execute_case(store, case);

        let entry = per_tool_counts
            .entry(case.tool_name.clone())
            .or_insert((0, 0));
        entry.0 += 1;
        if result.passed {
            entry.1 += 1;
        }

        total_savings += result.token_savings;
        case_results.push(result);
    }

    let total_cases = case_results.len();
    let cases_passed = case_results.iter().filter(|r| r.passed).count();
    let pass_rate = if total_cases > 0 {
        cases_passed as f64 / total_cases as f64
    } else {
        0.0
    };

    let avg_token_savings = if total_cases > 0 {
        total_savings as f64 / total_cases as f64
    } else {
        0.0
    };

    let per_tool_accuracy: HashMap<String, ToolAccuracy> = per_tool_counts
        .into_iter()
        .map(|(tool_name, (total, passed))| {
            let accuracy = if total > 0 {
                passed as f64 / total as f64
            } else {
                0.0
            };
            (
                tool_name.clone(),
                ToolAccuracy {
                    tool_name,
                    total,
                    passed,
                    accuracy,
                },
            )
        })
        .collect();

    BenchmarkResult {
        pass_rate,
        per_tool_accuracy,
        avg_token_savings,
        total_cases,
        cases_passed,
        case_results,
    }
}

/// Execute a single benchmark case by dispatching through the MCP tool system
/// and comparing the result against ground truth.
fn execute_case(store: &StoreManager, case: &BenchmarkCase) -> CaseResult {
    // Dispatch the tool call through the standard MCP dispatch system
    let dispatch_result = dispatch_tool(store, &case.tool_name, &case.input_args);

    match dispatch_result {
        Ok(response) => evaluate_response(&response, case),
        Err(e) => {
            // If the tool dispatch failed, check if we expected a fallback
            if case.expected_output.expects_fallback {
                CaseResult {
                    case_id: case.id.clone(),
                    tool_name: case.tool_name.clone(),
                    passed: true,
                    failure_reason: None,
                    token_savings: 0,
                }
            } else {
                CaseResult {
                    case_id: case.id.clone(),
                    tool_name: case.tool_name.clone(),
                    passed: false,
                    failure_reason: Some(format!("Tool dispatch error: {}", e)),
                    token_savings: 0,
                }
            }
        }
    }
}

/// Evaluate a successful tool response against the expected output.
fn evaluate_response(response: &serde_json::Value, case: &BenchmarkCase) -> CaseResult {
    // Extract the text content from the MCP response format
    let response_text = extract_response_text(response);

    // Extract token savings from _meta if available
    let token_savings = extract_token_savings(response);

    // Check expected FQNs
    if !case.expected_output.expected_fqns.is_empty() {
        let matched = case
            .expected_output
            .expected_fqns
            .iter()
            .filter(|expected| response_text.contains(expected.as_str()))
            .count();

        let required = (case.expected_output.expected_fqns.len() as f64
            * case.tolerance.fqn_match_ratio)
            .ceil() as usize;

        if matched < required {
            let missing: Vec<_> = case
                .expected_output
                .expected_fqns
                .iter()
                .filter(|e| !response_text.contains(e.as_str()))
                .collect();

            return CaseResult {
                case_id: case.id.clone(),
                tool_name: case.tool_name.clone(),
                passed: false,
                failure_reason: Some(format!(
                    "FQN match: {}/{} (required {}). Missing: {:?}",
                    matched,
                    case.expected_output.expected_fqns.len(),
                    required,
                    missing
                )),
                token_savings,
            };
        }
    }

    // Check expected keywords
    if !case.expected_output.expected_keywords.is_empty() {
        let matched_keywords = case
            .expected_output
            .expected_keywords
            .iter()
            .filter(|kw| response_text.to_lowercase().contains(&kw.to_lowercase()))
            .count();

        let required = (case.expected_output.expected_keywords.len() as f64
            * case.tolerance.fqn_match_ratio)
            .ceil() as usize;

        if matched_keywords < required {
            return CaseResult {
                case_id: case.id.clone(),
                tool_name: case.tool_name.clone(),
                passed: false,
                failure_reason: Some(format!(
                    "Keyword match: {}/{} (required {}). Keywords: {:?}",
                    matched_keywords,
                    case.expected_output.expected_keywords.len(),
                    required,
                    case.expected_output.expected_keywords
                )),
                token_savings,
            };
        }
    }

    // Check min_results (count occurrences of "fqn" fields in response)
    if let Some(min) = case.expected_output.min_results {
        let result_count = count_results_in_response(response);
        if result_count < min {
            return CaseResult {
                case_id: case.id.clone(),
                tool_name: case.tool_name.clone(),
                passed: false,
                failure_reason: Some(format!(
                    "Expected at least {} results, got {}",
                    min, result_count
                )),
                token_savings,
            };
        }
    }

    // Check max_results
    if let Some(max) = case.expected_output.max_results {
        if !case.tolerance.allow_extra_results {
            let result_count = count_results_in_response(response);
            if result_count > max {
                return CaseResult {
                    case_id: case.id.clone(),
                    tool_name: case.tool_name.clone(),
                    passed: false,
                    failure_reason: Some(format!(
                        "Expected at most {} results, got {}",
                        max, result_count
                    )),
                    token_savings,
                };
            }
        }
    }

    // Check confidence range
    if let Some((min_conf, max_conf)) = case.expected_output.confidence_range {
        if let Some(confidence) = extract_confidence(response) {
            if confidence < min_conf || confidence > max_conf {
                return CaseResult {
                    case_id: case.id.clone(),
                    tool_name: case.tool_name.clone(),
                    passed: false,
                    failure_reason: Some(format!(
                        "Confidence {:.2} outside expected range [{:.2}, {:.2}]",
                        confidence, min_conf, max_conf
                    )),
                    token_savings,
                };
            }
        }
    }

    // All checks passed
    CaseResult {
        case_id: case.id.clone(),
        tool_name: case.tool_name.clone(),
        passed: true,
        failure_reason: None,
        token_savings,
    }
}

/// Extract the full text content from an MCP tool response.
fn extract_response_text(response: &serde_json::Value) -> String {
    // MCP responses have format: { "content": [{"type": "text", "text": "..."}], "_meta": {...} }
    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        content
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        // Fallback: serialize the entire response
        serde_json::to_string(response).unwrap_or_default()
    }
}

/// Extract token savings from the _meta field of an MCP response.
fn extract_token_savings(response: &serde_json::Value) -> i64 {
    response
        .get("_meta")
        .and_then(|meta| meta.get("net_saved"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Extract confidence from the response text (parsed as JSON).
fn extract_confidence(response: &serde_json::Value) -> Option<f64> {
    let text = extract_response_text(response);
    // Try to parse the text content as JSON and look for a confidence field
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
        parsed.get("confidence").and_then(|c| c.as_f64())
    } else {
        None
    }
}

/// Count the number of results in a response by looking for array items in the parsed content.
fn count_results_in_response(response: &serde_json::Value) -> usize {
    let text = extract_response_text(response);
    // Try to parse the text content as JSON
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
        // Look for common array fields in tool responses
        if let Some(arr) = parsed.as_array() {
            return arr.len();
        }
        if let Some(arr) = parsed.get("results").and_then(|r| r.as_array()) {
            return arr.len();
        }
        if let Some(arr) = parsed.get("symbols").and_then(|r| r.as_array()) {
            return arr.len();
        }
        if let Some(arr) = parsed.get("callers").and_then(|r| r.as_array()) {
            return arr.len();
        }
        if let Some(arr) = parsed.get("nodes").and_then(|r| r.as_array()) {
            return arr.len();
        }
        // If it's a single object, count as 1
        if parsed.is_object() {
            return 1;
        }
    }
    // Fallback: count lines that look like results (contain "fqn")
    text.lines().filter(|l| l.contains("fqn")).count()
}

/// Print formatted benchmark results to stdout.
pub fn print_results(result: &BenchmarkResult, suite_name: &str) {
    println!("Cortex Benchmark Results: {}", suite_name);
    println!("{}", "═".repeat(60));
    println!();

    // Overall pass rate
    let pass_pct = result.pass_rate * 100.0;
    let status = if pass_pct >= 70.0 { "PASS" } else { "FAIL" };
    println!(
        "  Overall:  {} ({}/{} cases, {:.1}%)",
        status, result.cases_passed, result.total_cases, pass_pct
    );
    println!(
        "  Avg token savings: {:.0} tokens/query",
        result.avg_token_savings
    );
    println!();

    // Per-tool breakdown
    println!("  Per-Tool Accuracy:");
    println!("  {}", "-".repeat(50));

    let mut tools: Vec<_> = result.per_tool_accuracy.values().collect();
    tools.sort_by(|a, b| a.tool_name.cmp(&b.tool_name));

    for tool in &tools {
        let tool_status = if tool.accuracy >= 0.7 { "PASS" } else { "FAIL" };
        println!(
            "    {} {:<20} {}/{} ({:.0}%)",
            tool_status,
            tool.tool_name,
            tool.passed,
            tool.total,
            tool.accuracy * 100.0
        );
    }

    // Failed cases detail
    let failures: Vec<_> = result.case_results.iter().filter(|r| !r.passed).collect();
    if !failures.is_empty() {
        println!();
        println!("  Failed Cases:");
        println!("  {}", "-".repeat(50));
        for f in &failures {
            println!("    [{}] {}", f.tool_name, f.case_id);
            if let Some(ref reason) = f.failure_reason {
                println!("      Reason: {}", reason);
            }
        }
    }

    println!();

    // Warning if below 70%
    if pass_pct < 70.0 {
        println!(
            "  WARNING: Pass rate {:.1}% is below the 70% threshold.",
            pass_pct
        );
        println!("    This binary may have regressions. Run `cortex status` for details.");
        println!();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_suite_deserialize() {
        let json = r#"{
            "name": "test-suite",
            "description": "A test benchmark suite",
            "cases": [
                {
                    "id": "case-1",
                    "tool_name": "trace_callers",
                    "input_args": {"fqn": "src/main.rs::main", "depth": 3},
                    "expected_output": {
                        "expected_fqns": ["src/lib.rs::helper"],
                        "min_results": 1
                    }
                }
            ]
        }"#;

        let suite: BenchmarkSuite = serde_json::from_str(json).unwrap();
        assert_eq!(suite.name, "test-suite");
        assert_eq!(suite.cases.len(), 1);
        assert_eq!(suite.cases[0].tool_name, "trace_callers");
        assert_eq!(
            suite.cases[0].expected_output.expected_fqns,
            vec!["src/lib.rs::helper"]
        );
    }

    #[test]
    fn test_tolerance_defaults() {
        let t = Tolerance::default();
        assert!((t.fqn_match_ratio - 0.7).abs() < f64::EPSILON);
        assert!(t.allow_extra_results);
    }

    #[test]
    fn test_expected_output_defaults() {
        let json = r#"{"expected_fqns": ["foo"]}"#;
        let output: ExpectedOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.expected_fqns, vec!["foo"]);
        assert!(output.min_results.is_none());
        assert!(output.max_results.is_none());
        assert!(output.expected_keywords.is_empty());
        assert!(!output.expects_fallback);
        assert!(output.confidence_range.is_none());
    }

    #[test]
    fn test_benchmark_result_serialization() {
        let result = BenchmarkResult {
            pass_rate: 0.85,
            per_tool_accuracy: HashMap::new(),
            avg_token_savings: 1200.0,
            total_cases: 20,
            cases_passed: 17,
            case_results: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: BenchmarkResult = serde_json::from_str(&json).unwrap();
        assert!((deserialized.pass_rate - 0.85).abs() < f64::EPSILON);
        assert_eq!(deserialized.total_cases, 20);
        assert_eq!(deserialized.cases_passed, 17);
    }

    #[test]
    fn test_extract_response_text_mcp_format() {
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "hello world"}
            ],
            "_meta": {}
        });
        let text = extract_response_text(&response);
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_extract_response_text_multi_content() {
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "line1"},
                {"type": "text", "text": "line2"}
            ]
        });
        let text = extract_response_text(&response);
        assert_eq!(text, "line1\nline2");
    }

    #[test]
    fn test_extract_token_savings() {
        let response = serde_json::json!({
            "content": [],
            "_meta": {"net_saved": 1500}
        });
        assert_eq!(extract_token_savings(&response), 1500);

        let response_no_meta = serde_json::json!({"content": []});
        assert_eq!(extract_token_savings(&response_no_meta), 0);
    }

    #[test]
    fn test_count_results_array() {
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "[{\"fqn\":\"a\"},{\"fqn\":\"b\"},{\"fqn\":\"c\"}]"}
            ]
        });
        assert_eq!(count_results_in_response(&response), 3);
    }

    #[test]
    fn test_evaluate_response_fqn_match() {
        let case = BenchmarkCase {
            id: "test-1".to_string(),
            tool_name: "trace_callers".to_string(),
            input_args: serde_json::json!({}),
            expected_output: ExpectedOutput {
                expected_fqns: vec!["foo::bar".to_string(), "baz::qux".to_string()],
                min_results: None,
                max_results: None,
                expected_keywords: vec![],
                expects_fallback: false,
                confidence_range: None,
            },
            tolerance: Tolerance::default(),
        };

        // Response contains both expected FQNs
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "{\"callers\":[{\"fqn\":\"foo::bar\"},{\"fqn\":\"baz::qux\"}]}"}
            ]
        });
        let result = evaluate_response(&response, &case);
        assert!(result.passed);

        // Response missing one FQN (1/2 = 50% < 70% threshold)
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "{\"callers\":[{\"fqn\":\"foo::bar\"}]}"}
            ]
        });
        let result = evaluate_response(&response, &case);
        assert!(!result.passed);
    }

    #[test]
    fn test_evaluate_response_keyword_match() {
        let case = BenchmarkCase {
            id: "test-2".to_string(),
            tool_name: "ask".to_string(),
            input_args: serde_json::json!({}),
            expected_output: ExpectedOutput {
                expected_fqns: vec![],
                min_results: None,
                max_results: None,
                expected_keywords: vec!["authentication".to_string(), "token".to_string()],
                expects_fallback: false,
                confidence_range: None,
            },
            tolerance: Tolerance {
                fqn_match_ratio: 0.5,
                allow_extra_results: true,
            },
        };

        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "The authentication module handles token validation."}
            ]
        });
        let result = evaluate_response(&response, &case);
        assert!(result.passed);
    }

    #[test]
    fn test_load_suite_invalid_path() {
        let result = load_suite(Path::new("/nonexistent/path.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_run_benchmark_empty_suite() {
        // Create a minimal store for testing
        let tmp = tempfile::TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();
        let conn = store.write_conn();
        crate::store::migrations::run_embedded_migrations(&conn).unwrap();
        drop(conn);

        let suite = BenchmarkSuite {
            name: "empty".to_string(),
            description: "Empty suite".to_string(),
            cases: vec![],
        };

        let result = run_benchmark(&store, &suite);
        assert_eq!(result.total_cases, 0);
        assert_eq!(result.cases_passed, 0);
        assert!((result.pass_rate - 0.0).abs() < f64::EPSILON);
    }
}
