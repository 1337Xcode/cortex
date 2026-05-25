//! Modules command: show detected module boundaries from Leiden community detection.
//!
//! `cortex modules` prints the detected clusters of tightly-coupled code,
//! their core functions, coupling scores, and file membership.

use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::community;

/// Run the modules command.
pub fn run(store: &Arc<StoreManager>, coupling_threshold: f64) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    let result = community::detect_communities(&conn, None, coupling_threshold)?;

    if result.communities.is_empty() {
        println!("Not enough call graph edges to detect module boundaries.");
        println!("Index more files or lower the coupling threshold (--threshold).");
        return Ok(());
    }

    println!();
    println!("DETECTED MODULES (Leiden community detection)");
    println!("{}", "=".repeat(50));
    println!();
    println!(
        "  {} modules detected (coupling threshold: {:.1})",
        result.communities.len(),
        coupling_threshold
    );
    println!();

    for (i, comm) in result.communities.iter().enumerate() {
        println!(
            "  Module {} ({} nodes, {} files)",
            i + 1,
            comm.node_count,
            comm.files.len()
        );
        println!("  {}", "-".repeat(40));

        // Show files
        for file in comm.files.iter().take(8) {
            println!("    {}", file);
        }
        if comm.files.len() > 8 {
            println!("    ... and {} more files", comm.files.len() - 8);
        }

        // Show top members (API surface)
        if !comm.suggested_api_surface.is_empty() {
            println!();
            println!("    Core functions:");
            for member in comm.suggested_api_surface.iter().take(5) {
                println!("      {}", member);
            }
            if comm.suggested_api_surface.len() > 5 {
                println!(
                    "      ... and {} more",
                    comm.suggested_api_surface.len() - 5
                );
            }
        }

        println!();
    }

    println!("{}", "=".repeat(50));
    println!();
    println!("  Modules with high inter-cluster coupling are refactoring candidates.");
    println!("  Use `cortex impact <fqn>` to check cross-module dependencies.");
    println!();

    Ok(())
}
