//! Diff command: compare call graphs between two git branches.
//!
//! `cortex diff main feature-branch` shows what architectural changes
//! a branch introduces - new edges, removed edges, new nodes, deleted nodes.

use std::process::Command;
use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::graph;

/// Run the diff command.
pub fn run(base: &str, head: &str, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    // Get list of files changed between the two branches
    let output = Command::new("git")
        .args(["diff", "--name-only", &format!("{}...{}", base, head)])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr);
    }

    let changed_files: Vec<&str> = std::str::from_utf8(&output.stdout)?
        .lines()
        .filter(|l| !l.is_empty())
        .collect();

    if changed_files.is_empty() {
        println!("No file differences between {} and {}.", base, head);
        return Ok(());
    }

    let conn = store.read_conn();

    println!();
    println!("GRAPH DIFF — {} → {}", base, head);
    println!("{}", "━".repeat(50));
    println!();
    println!("  {} file{} changed", changed_files.len(), if changed_files.len() == 1 { "" } else { "s" });
    println!();

    // For each changed file, find all nodes defined in it and their edges
    let mut affected_nodes: Vec<String> = Vec::new();
    let mut affected_files_with_nodes: Vec<(&str, Vec<String>)> = Vec::new();

    for file in &changed_files {
        let pattern = format!("{}::*", file);
        let nodes = graph::find_nodes_by_pattern(&conn, &pattern, None, 200)?;
        if !nodes.is_empty() {
            let fqns: Vec<String> = nodes.iter().map(|n| n.fqn.clone()).collect();
            affected_nodes.extend(fqns.clone());
            affected_files_with_nodes.push((file, fqns));
        }
    }

    if affected_nodes.is_empty() {
        println!("  No indexed symbols found in changed files.");
        println!("  (Files may be non-code or not yet indexed.)");
    } else {
        println!("  Symbols in changed files:");
        println!();
        for (file, fqns) in &affected_files_with_nodes {
            println!("  {}:", file);
            for fqn in fqns.iter().take(10) {
                println!("    • {}", fqn);
            }
            if fqns.len() > 10 {
                println!("    ... and {} more", fqns.len() - 10);
            }
        }

        // Compute total blast radius
        println!();
        println!("  Blast radius:");
        let mut total_affected: std::collections::HashSet<String> = std::collections::HashSet::new();
        for fqn in affected_nodes.iter().take(50) {
            let radius = graph::blast_radius(&conn, fqn, 2)?;
            for n in &radius {
                total_affected.insert(n.fqn.clone());
            }
        }
        // Remove the changed nodes themselves
        for fqn in &affected_nodes {
            total_affected.remove(fqn);
        }

        if total_affected.is_empty() {
            println!("    No downstream dependencies affected.");
        } else {
            println!("    {} downstream function{} potentially affected",
                total_affected.len(),
                if total_affected.len() == 1 { "" } else { "s" }
            );
            for fqn in total_affected.iter().take(15) {
                println!("    └─ {}", fqn);
            }
            if total_affected.len() > 15 {
                println!("    ... and {} more", total_affected.len() - 15);
            }
        }
    }

    println!();
    println!("{}", "━".repeat(50));
    println!();

    Ok(())
}
