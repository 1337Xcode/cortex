//! The budget checks of `cargo xtask verify --full`: runs the benchmarks, reads the samples
//! criterion wrote, and fails when the 95th percentile of a benchmark's runs is over its budget.
//! Each sample must hold one run, so that the percentile is of runs and not of their means. Each
//! budget's line is printed with the measured value, within budget or not.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::steps;

/// One budget: a benchmark, what it measures, and the most its 95th percentile may take.
struct Budget {
    /// The benchmark group.
    group: &'static str,
    /// The benchmark within the group.
    function: &'static str,
    /// What the benchmark measures, for the printed line.
    what: &'static str,
    /// The largest 95th percentile within budget.
    limit: Duration,
}

/// Every budget a benchmark checks. The limits are the product's latency budgets.
const BUDGETS: [Budget; 2] = [
    Budget {
        group: "index",
        function: "initial_10k",
        what: "first index of 10,000 source files",
        limit: Duration::from_secs(60),
    },
    Budget {
        group: "index",
        function: "one_file_changed_10k",
        what: "index after one file changed, 10,000 source files",
        limit: Duration::from_millis(30),
    },
];

/// The samples criterion records for a benchmark: per sample, how many iterations ran and their
/// total time in nanoseconds.
#[derive(Debug, Deserialize)]
struct Samples {
    /// Iterations per sample.
    iters: Vec<f64>,
    /// Total nanoseconds per sample.
    times: Vec<f64>,
}

/// Runs the benchmarks and checks every budget.
///
/// # Errors
///
/// A benchmark that fails to run, samples that cannot be read, or a budget breached.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    steps::run_cargo(
        root,
        "benchmarks",
        &[
            "bench",
            "--locked",
            "-p",
            "cortex-index",
            "--bench",
            "index",
        ],
    )?;
    let criterion = target_dir(root).join("criterion");
    let mut breached = Vec::new();
    for budget in &BUDGETS {
        let path = criterion
            .join(budget.group)
            .join(budget.function)
            .join("new")
            .join("sample.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let samples: Samples =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let measured = p95(&samples).with_context(|| format!("reading {}", path.display()))?;
        let verdict = if measured <= budget.limit {
            "within"
        } else {
            breached.push(budget.what);
            "OVER"
        };
        println!(
            "xtask: budget: {}: p95 {:.1} ms, {verdict} the {} ms budget",
            budget.what,
            measured.as_secs_f64() * 1000.0,
            budget.limit.as_millis()
        );
    }
    if !breached.is_empty() {
        bail!("over budget: {}", breached.join("; "));
    }
    Ok(())
}

/// Where cargo builds: `CARGO_TARGET_DIR` when set, else `target` in the workspace.
fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

/// The 95th percentile, by nearest rank, of the samples' times, each sample one run.
///
/// # Errors
///
/// No samples, or a sample of more than one run: its time would be a mean, hiding the slow runs
/// the percentile is there to show.
fn p95(samples: &Samples) -> anyhow::Result<Duration> {
    if let Some(runs) = samples.iters.iter().find(|runs| **runs != 1.0) {
        bail!("a sample holds {runs} runs; the percentile must be of single runs");
    }
    let mut times = samples.times.clone();
    times.sort_by(f64::total_cmp);
    // Nearest rank: the smallest value with at least 95% of the samples at or below it.
    let rank = (times.len() * 95).div_ceil(100);
    let nanoseconds = rank
        .checked_sub(1)
        .and_then(|index| times.get(index))
        .context("no samples")?;
    Ok(Duration::from_secs_f64(nanoseconds / 1e9))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty samples of one run taking 1 to 20 ms: 95% of 20 is 19, so the 19th smallest, 19 ms.
    /// Samples of several runs are refused, and no samples make no percentile.
    #[test]
    fn p95_is_the_nearest_rank_of_single_runs() -> anyhow::Result<()> {
        let twenty = Samples {
            iters: vec![1.0; 20],
            times: (1..=20).rev().map(|ms| f64::from(ms) * 1e6).collect(),
        };
        assert_eq!(p95(&twenty)?, Duration::from_millis(19));
        let grouped = Samples {
            iters: vec![1.0, 4.0],
            times: vec![10e6, 40e6],
        };
        assert!(p95(&grouped).is_err());
        let none = Samples {
            iters: Vec::new(),
            times: Vec::new(),
        };
        assert!(p95(&none).is_err());
        Ok(())
    }
}
