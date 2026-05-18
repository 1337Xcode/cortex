//! Hotspots command: find high-churn, high-complexity code.
//!
//! `cortex hotspots` combines git commit frequency with call graph
//! connectivity to identify the riskiest code in the repository.
//! High churn + high caller count = maintenance risk.

use std::collections::HashMap;
use std::process::Command as ProcessCommand;
use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::graph;

/// A single hotspot result combining git churn with graph connectivity.
struct HotspotResult {
    file: String,
    fqn: String,
    churn_count: u32,
    caller_count: u32,
    risk_score: u64,
}

/// Run the hotspots analysis and print results.
pub fn run(
    _config: &crate::config::Config,
    store: &Arc<StoreManager>,
    months: u32,
    limit: usize,
) -> Result<(), anyhow::Error> {
    // Step 1: Get git churn data (file change frequency).
    let churn_map = match get_git_churn(months) {
        Ok(map) => map,
        Err(e) => {
            eprintln!("warning: could not read git history: {}", e);
            eprintln!("hint: make sure git is installed and you are inside a git repository.");
            HashMap::new()
        }
    };

    if churn_map.is_empty() {
        println!("No git history found (or git is not available).");
        println!("Run this command inside a git repository with commit history.");
        return Ok(());
    }

    // Step 2: Query the nodes table for all functions with their caller counts.
    let conn = store.read_conn();
    let hotspot_nodes = graph::get_hotspot_nodes(&conn, 500)?;

    if hotspot_nodes.is_empty() {
        println!("No nodes in the graph. Run `cortex index` first.");
        return Ok(());
    }

    // Step 3: Join git churn with caller counts to compute risk scores.
    let mut results: Vec<HotspotResult> = Vec::new();

    for node in &hotspot_nodes {
        let churn = churn_map.get(&node.file).copied().unwrap_or(0);
        if churn == 0 {
            continue;
        }
        let risk_score = churn as u64 * node.caller_count as u64;
        if risk_score == 0 {
            continue;
        }
        results.push(HotspotResult {
            file: node.file.clone(),
            fqn: node.fqn.clone(),
            churn_count: churn,
            caller_count: node.caller_count,
            risk_score,
        });
    }

    // Step 4: Sort by risk score descending and take top N.
    results.sort_by(|a, b| b.risk_score.cmp(&a.risk_score));
    results.truncate(limit);

    // Step 5: Print results.
    println!();
    println!("GIT HOTSPOTS (last {} months)", months);
    println!("{}", "━".repeat(70));
    println!();

    if results.is_empty() {
        println!("  No hotspots found. Either git history does not overlap with indexed files,");
        println!("  or no functions have both churn and callers.");
    } else {
        println!(
            "  {:<6} {:<8} {:<8} {}",
            "RISK", "CHURN", "CALLERS", "FUNCTION"
        );
        println!("  {}", "─".repeat(66));

        for r in &results {
            println!(
                "  {:<6} {:<8} {:<8} {}",
                r.risk_score, r.churn_count, r.caller_count, r.fqn
            );
            println!("  {:<6} {:<8} {:<8} {}", "", "", "", r.file);
            println!();
        }

        println!("{}", "━".repeat(70));
        println!();
        println!(
            "  {} hotspot{} found. Risk = churn * callers (higher = more maintenance risk).",
            results.len(),
            if results.len() == 1 { "" } else { "s" }
        );
    }

    println!();

    Ok(())
}

/// Run `git log` to count how many commits touched each file in the given time window.
fn get_git_churn(months: u32) -> Result<HashMap<String, u32>, anyhow::Error> {
    let since_arg = format!("{} months ago", months);

    let output = ProcessCommand::new("git")
        .args(["log", "--format=format:", "--name-only", "--since", &since_arg])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git log failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut churn_map: HashMap<String, u32> = HashMap::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Normalize path separators to forward slashes.
        let normalized = trimmed.replace('\\', "/");
        *churn_map.entry(normalized).or_insert(0) += 1;
    }

    Ok(churn_map)
}

/// Get git churn data for use by the MCP tool (public interface).
pub fn get_git_churn_public(months: u32) -> Result<HashMap<String, u32>, anyhow::Error> {
    get_git_churn(months)
}
