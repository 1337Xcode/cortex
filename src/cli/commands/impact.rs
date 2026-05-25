//! Impact command: show what breaks if you change a function.
//!
//! `cortex impact UserService.getUser` prints every call chain that
//! would be affected by a change to that function. Uses the existing
//! blast_radius graph traversal.

use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::graph;

/// Run the impact analysis and print results.
pub fn run(fqn: &str, depth: u32, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    // Get the node itself
    let node = graph::find_node_by_fqn(&conn, fqn)?;
    let node = match node {
        Some(n) => n,
        None => {
            // Try fuzzy match
            let candidates = graph::find_nodes_by_pattern(&conn, &format!("*{}*", fqn), None, 5)?;
            if candidates.is_empty() {
                eprintln!("Node not found: {}", fqn);
                eprintln!(
                    "Try `cortex query find {}` to search for matching symbols.",
                    fqn
                );
                return Ok(());
            }
            eprintln!("Exact match not found. Did you mean:");
            for c in &candidates {
                eprintln!("  {} ({})", c.fqn, c.file);
            }
            return Ok(());
        }
    };

    // Get blast radius (all transitive dependents)
    let affected = graph::blast_radius(&conn, fqn, depth)?;

    println!();
    println!("IMPACT ANALYSIS: {}", fqn);
    println!("{}", "━".repeat(50));
    println!();
    println!("  Location: {}:{}", node.file, node.start_line);
    println!("  Kind: {:?}", node.kind);
    println!();

    if affected.is_empty() {
        println!("  No other functions depend on this node.");
    } else {
        // Group by file
        let mut by_file: std::collections::BTreeMap<&str, Vec<&crate::store::types::Node>> =
            std::collections::BTreeMap::new();
        for n in &affected {
            by_file.entry(n.file.as_str()).or_default().push(n);
        }

        println!(
            "  {} function{} affected across {} file{}:",
            affected.len(),
            if affected.len() == 1 { "" } else { "s" },
            by_file.len(),
            if by_file.len() == 1 { "" } else { "s" },
        );
        println!();

        for (file, nodes) in &by_file {
            println!("  {}:", file);
            for n in nodes {
                println!("    └─ {} (line {})", n.fqn, n.start_line);
            }
        }
    }

    println!();
    println!("{}", "━".repeat(50));
    println!();

    Ok(())
}
