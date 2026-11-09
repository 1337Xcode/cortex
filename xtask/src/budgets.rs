//! The budget checks of `cargo xtask verify --full`: runs the benchmarks, reads the samples
//! criterion wrote, and fails when the 95th percentile of a benchmark's runs is over its budget.
//! Each sample must hold one run, so that the percentile is of runs and not of their means. Each
//! budget's line is printed with the measured value, within budget or not, and says so when the
//! budget is not checked on this platform.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::steps;

/// Where a budget holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platforms {
    /// Wherever the build runs.
    Every,
    /// On the machines hook latency is specified on, macOS and Linux. On Windows starting a
    /// process alone takes 6 to 7 ms at the median and over 10 ms at the 95th percentile, most of
    /// a hook's time and more than its cold start's budget; and the malware protection reads
    /// through the large files a benchmark writes, which made the same batch of upkeep take 26 ms
    /// or 755 ms. There these budgets are measured and printed, not checked.
    Reference,
}

/// One budget: a benchmark, what it measures, the most its 95th percentile may take, and where.
struct Budget {
    /// The benchmark group.
    group: &'static str,
    /// The benchmark within the group.
    function: &'static str,
    /// What the benchmark measures, for the printed line.
    what: &'static str,
    /// The largest 95th percentile within budget.
    limit: Duration,
    /// Where the budget is checked.
    platforms: Platforms,
}

/// The benchmarks the budgets read: each package and its benchmark target.
const BENCHES: [(&str, &str); 4] = [
    ("cortex-index", "index"),
    ("cortex-retrieve", "spread"),
    ("cortex-daemon", "upkeep"),
    ("cortex-cli", "hooks"),
];

/// Every budget a benchmark checks. The limits are the product's latency budgets.
const BUDGETS: [Budget; 24] = [
    Budget {
        group: "index",
        function: "initial_10k",
        what: "first index of 10,000 source files",
        limit: Duration::from_secs(60),
        platforms: Platforms::Every,
    },
    Budget {
        group: "index",
        function: "one_file_changed_10k",
        what: "index after one file changed, 10,000 source files",
        limit: Duration::from_millis(30),
        platforms: Platforms::Every,
    },
    Budget {
        group: "hooks",
        function: "session_start",
        what: "SessionStart hook, start to exit",
        limit: Duration::from_millis(120),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "session_start_compact",
        what: "SessionStart hook after a compaction, with its recap, start to exit",
        limit: Duration::from_millis(120),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "user_prompt_submit",
        what: "UserPromptSubmit hook, start to exit",
        limit: Duration::from_millis(150),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "user_prompt_submit_word",
        what: "UserPromptSubmit hook on a one-word prompt, which asks for no work order",
        limit: Duration::from_millis(150),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "pre_tool_use_read",
        what: "PreToolUse hook on a read, start to exit",
        limit: Duration::from_millis(40),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "post_tool_use_edit",
        what: "PostToolUse hook on an edit, start to exit",
        limit: Duration::from_millis(80),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "post_tool_use_shell",
        what: "PostToolUse hook on a shell command, start to exit",
        limit: Duration::from_millis(60),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks",
        function: "stop",
        what: "Stop hook, start to exit",
        limit: Duration::from_millis(60),
        platforms: Platforms::Reference,
    },
    Budget {
        // Start to exit bounds the budget's start to first byte read from above.
        group: "hooks",
        function: "cold_start",
        what: "hook cold start with no input, start to exit",
        limit: Duration::from_millis(10),
        platforms: Platforms::Reference,
    },
    // Spreading is one step of a retrieval, so it is held to the whole retrieval's budget at every
    // size; a huge repository spreads in two stages.
    Budget {
        group: "spread",
        function: "nodes/10000",
        what: "spreading over 10,000 nodes",
        limit: Duration::from_millis(120),
        platforms: Platforms::Every,
    },
    Budget {
        group: "spread",
        function: "nodes/100000",
        what: "spreading over 100,000 nodes",
        limit: Duration::from_millis(120),
        platforms: Platforms::Every,
    },
    Budget {
        group: "spread",
        function: "nodes/1000000",
        what: "spreading over 1,000,000 nodes",
        limit: Duration::from_millis(120),
        platforms: Platforms::Every,
    },
    Budget {
        group: "spread",
        function: "two_stage_nodes/1000000",
        what: "two-stage spreading over 1,000,000 nodes",
        limit: Duration::from_millis(120),
        platforms: Platforms::Every,
    },
    // Upkeep stops a run after 2 minutes and goes on at the next; a large store's whole pass
    // through its jobs is less work than one run may do. Between two batches it lets the writer
    // record the hook events waiting, so that a batch keeps them waiting no longer than an index
    // does: the index lets them in every 250 ms.
    Budget {
        group: "upkeep",
        function: "run_large",
        what: "a pass of upkeep through its jobs on a large store, its waits left out",
        limit: Duration::from_secs(120),
        platforms: Platforms::Every,
    },
    Budget {
        group: "upkeep",
        function: "longest_batch_large",
        what: "the longest batch of a pass of upkeep on a large store",
        limit: Duration::from_millis(250),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "retrieval",
        function: "medium_10k",
        what: "a prompt's retrieval in the daemon, 10,000 source files",
        limit: Duration::from_millis(120),
        platforms: Platforms::Every,
    },
    // With the daemon down a hook tries it, starts one and exits: each event keeps its budget.
    Budget {
        group: "hooks_daemon_down",
        function: "session_start",
        what: "SessionStart hook with the daemon down, start to exit",
        limit: Duration::from_millis(120),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks_daemon_down",
        function: "user_prompt_submit",
        what: "UserPromptSubmit hook with the daemon down, start to exit",
        limit: Duration::from_millis(150),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks_daemon_down",
        function: "pre_tool_use_read",
        what: "PreToolUse hook on a read with the daemon down, start to exit",
        limit: Duration::from_millis(40),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks_daemon_down",
        function: "post_tool_use_edit",
        what: "PostToolUse hook on an edit with the daemon down, start to exit",
        limit: Duration::from_millis(80),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks_daemon_down",
        function: "post_tool_use_shell",
        what: "PostToolUse hook on a shell command with the daemon down, start to exit",
        limit: Duration::from_millis(60),
        platforms: Platforms::Reference,
    },
    Budget {
        group: "hooks_daemon_down",
        function: "stop",
        what: "Stop hook with the daemon down, start to exit",
        limit: Duration::from_millis(60),
        platforms: Platforms::Reference,
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

/// Runs the benchmarks and checks every budget that holds on this platform, printing every
/// budget's measurement.
///
/// # Errors
///
/// A benchmark that fails to run, samples that cannot be read, or a budget breached.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    for (package, bench) in BENCHES {
        steps::run_cargo(
            root,
            "benchmarks",
            &["bench", "--locked", "-p", package, "--bench", bench],
        )?;
    }
    let criterion = target_dir(root).join("criterion");
    let mut breached = Vec::new();
    for budget in &BUDGETS {
        let measured = measured_p95(&criterion, budget)?;
        let limit_ms = budget.limit.as_millis();
        let verdict = if !is_checked_here(budget.platforms) {
            format!("the {limit_ms} ms budget holds on macOS and Linux and is not checked here")
        } else if measured <= budget.limit {
            format!("within the {limit_ms} ms budget")
        } else {
            breached.push(budget.what);
            format!("OVER the {limit_ms} ms budget")
        };
        println!(
            "xtask: budget: {}: p95 {:.1} ms, {verdict}",
            budget.what,
            measured.as_secs_f64() * 1000.0
        );
    }
    if !breached.is_empty() {
        bail!("over budget: {}", breached.join("; "));
    }
    Ok(())
}

/// The 95th percentile of the runs of `budget`'s benchmark, from the samples under `criterion`.
///
/// # Errors
///
/// The samples cannot be read or hold no single runs.
fn measured_p95(criterion: &Path, budget: &Budget) -> anyhow::Result<Duration> {
    let path = criterion
        .join(budget.group)
        .join(budget.function)
        .join("new")
        .join("sample.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let samples: Samples =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    p95(&samples).with_context(|| format!("reading {}", path.display()))
}

/// Whether a budget that holds on `platforms` is checked on the platform xtask runs on.
fn is_checked_here(platforms: Platforms) -> bool {
    platforms == Platforms::Every || !cfg!(windows)
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

    /// A budget for every platform is checked everywhere; one for the reference machines is
    /// checked everywhere but Windows.
    #[test]
    fn reference_budgets_are_not_checked_on_windows() {
        assert!(is_checked_here(Platforms::Every));
        assert_eq!(is_checked_here(Platforms::Reference), !cfg!(windows));
    }

    /// Every hook the benchmarks run has a budget, read from the hooks group with the daemon running
    /// and from its own group with the daemon down; the index's two budgets, spreading's four,
    /// upkeep's pass and retrieval's hold everywhere, spreading and retrieval within 120 ms; upkeep's
    /// pass within 2 minutes, and its longest batch within 250 ms where the hooks' budgets hold.
    #[test]
    fn every_hook_benchmark_has_its_budget() {
        let hooks: Vec<(&str, u128)> = BUDGETS
            .iter()
            .filter(|budget| budget.group == "hooks")
            .map(|budget| (budget.function, budget.limit.as_millis()))
            .collect();
        assert_eq!(
            hooks,
            [
                ("session_start", 120),
                ("session_start_compact", 120),
                ("user_prompt_submit", 150),
                ("user_prompt_submit_word", 150),
                ("pre_tool_use_read", 40),
                ("post_tool_use_edit", 80),
                ("post_tool_use_shell", 60),
                ("stop", 60),
                ("cold_start", 10)
            ]
        );
        let daemon_down: Vec<(&str, u128)> = BUDGETS
            .iter()
            .filter(|budget| budget.group == "hooks_daemon_down")
            .map(|budget| (budget.function, budget.limit.as_millis()))
            .collect();
        assert_eq!(
            daemon_down,
            [
                ("session_start", 120),
                ("user_prompt_submit", 150),
                ("pre_tool_use_read", 40),
                ("post_tool_use_edit", 80),
                ("post_tool_use_shell", 60),
                ("stop", 60)
            ]
        );
        let everywhere: Vec<&str> = BUDGETS
            .iter()
            .filter(|budget| budget.platforms == Platforms::Every)
            .map(|budget| budget.function)
            .collect();
        assert_eq!(
            everywhere,
            [
                "initial_10k",
                "one_file_changed_10k",
                "nodes/10000",
                "nodes/100000",
                "nodes/1000000",
                "two_stage_nodes/1000000",
                "run_large",
                "medium_10k"
            ]
        );
        let upkeep: Vec<(&str, u128, Platforms)> = BUDGETS
            .iter()
            .filter(|budget| budget.group == "upkeep")
            .map(|budget| (budget.function, budget.limit.as_millis(), budget.platforms))
            .collect();
        assert_eq!(
            upkeep,
            [
                ("run_large", 120_000, Platforms::Every),
                ("longest_batch_large", 250, Platforms::Reference)
            ]
        );
        let retrieving: Vec<u128> = BUDGETS
            .iter()
            .filter(|budget| budget.group == "spread" || budget.group == "retrieval")
            .map(|budget| budget.limit.as_millis())
            .collect();
        assert_eq!(retrieving, [120, 120, 120, 120, 120]);
    }
}
