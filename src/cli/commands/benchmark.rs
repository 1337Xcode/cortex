//! Benchmark command implementation.
//!
//! Runs the Cortex benchmark suite against the current binary and reports
//! pass rate, per-tool accuracy, and average token savings.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::benchmark::{self, BenchmarkResult};
use crate::store::db::StoreManager;

/// Cached benchmark result stored on disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedBenchmarkResult {
    pub pass_rate: f64,
    pub total_cases: usize,
    pub cases_passed: usize,
    pub avg_token_savings: f64,
    pub run_at: i64,
}

/// Path to the cached benchmark result file.
fn result_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("benchmark_result.json")
}

/// Load the most recent benchmark result from cache (if available).
pub fn load_benchmark_result(data_dir: &Path) -> Option<CachedBenchmarkResult> {
    let path = result_cache_path(data_dir);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save a benchmark result to the cache.
fn save_benchmark_result(data_dir: &Path, result: &BenchmarkResult) {
    let cached = CachedBenchmarkResult {
        pass_rate: result.pass_rate,
        total_cases: result.total_cases,
        cases_passed: result.cases_passed,
        avg_token_savings: result.avg_token_savings,
        run_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    let path = result_cache_path(data_dir);
    if let Ok(json) = serde_json::to_string_pretty(&cached) {
        let _ = std::fs::write(&path, json);
    }
}

/// Run the benchmark command.
///
/// If `suite` is provided, loads that specific suite file.
/// Otherwise, looks for the default suite at `benchmark/placeholder_suite.json`.
pub fn run(
    store: &Arc<StoreManager>,
    suite: Option<PathBuf>,
    data_dir: &Path,
) -> Result<(), String> {
    let suite_path = match suite {
        Some(p) => p,
        None => {
            // Look for default suite relative to repo root
            let default_path = PathBuf::from("benchmark/placeholder_suite.json");
            if !default_path.exists() {
                return Err(
                    "No benchmark suite found. Provide a suite with --suite <path>, or create benchmark/placeholder_suite.json".to_string()
                );
            }
            default_path
        }
    };

    let suite_data = benchmark::load_suite(&suite_path)?;
    println!("Running benchmark suite: {}", suite_data.name);
    println!("  {} cases", suite_data.cases.len());
    println!();

    let result = benchmark::run_benchmark(store, &suite_data);

    // Print results
    benchmark::print_results(&result, &suite_data.name);

    // Cache the result
    save_benchmark_result(data_dir, &result);

    // Exit with error if below threshold
    if result.pass_rate < 0.7 {
        Err(format!(
            "Benchmark pass rate {:.1}% is below the 70% threshold.",
            result.pass_rate * 100.0
        ))
    } else {
        Ok(())
    }
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cached_benchmark_result_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Create a BenchmarkResult to save
        let result = BenchmarkResult {
            pass_rate: 0.85,
            per_tool_accuracy: std::collections::HashMap::new(),
            avg_token_savings: 1200.0,
            total_cases: 20,
            cases_passed: 17,
            case_results: vec![],
        };

        save_benchmark_result(tmp.path(), &result);
        let loaded = load_benchmark_result(tmp.path()).unwrap();

        assert!((loaded.pass_rate - 0.85).abs() < f64::EPSILON);
        assert_eq!(loaded.total_cases, 20);
        assert_eq!(loaded.cases_passed, 17);
        assert!((loaded.avg_token_savings - 1200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_load_benchmark_result_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = load_benchmark_result(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_cached_benchmark_result_below_threshold() {
        let tmp = tempfile::TempDir::new().unwrap();

        let result = BenchmarkResult {
            pass_rate: 0.55,
            per_tool_accuracy: std::collections::HashMap::new(),
            avg_token_savings: 500.0,
            total_cases: 20,
            cases_passed: 11,
            case_results: vec![],
        };

        save_benchmark_result(tmp.path(), &result);
        let loaded = load_benchmark_result(tmp.path()).unwrap();

        assert!(loaded.pass_rate < 0.7);
    }

    #[test]
    fn test_cached_benchmark_result_at_threshold() {
        let tmp = tempfile::TempDir::new().unwrap();

        let result = BenchmarkResult {
            pass_rate: 0.7,
            per_tool_accuracy: std::collections::HashMap::new(),
            avg_token_savings: 800.0,
            total_cases: 10,
            cases_passed: 7,
            case_results: vec![],
        };

        save_benchmark_result(tmp.path(), &result);
        let loaded = load_benchmark_result(tmp.path()).unwrap();

        // Exactly at threshold should NOT trigger warning (< 0.7 triggers)
        assert!(loaded.pass_rate >= 0.7);
    }
}
