//! Hierarchy command: print class inheritance and interface implementation tree.
//!
//! `cortex hierarchy UserViewController` prints the full class tree:
//! parents (what it extends), interfaces (what it implements), and
//! children (what extends it).

use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::graph;

/// Run the hierarchy command and print the class tree.
pub fn run(fqn: &str, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    // Find the node by exact FQN
    let node = graph::find_node_by_fqn(&conn, fqn)?;
    let node = match node {
        Some(n) => n,
        None => {
            // Try fuzzy match
            let candidates = graph::find_nodes_by_pattern(&conn, &format!("*{}*", fqn), None, 10)?;
            if candidates.is_empty() {
                eprintln!("Node not found: {}", fqn);
                eprintln!("No matching symbols in the graph. Run `cortex index` first.");
                return Ok(());
            }
            eprintln!("Exact match not found for '{}'. Did you mean:", fqn);
            eprintln!();
            for c in &candidates {
                eprintln!("  {} ({}, {})", c.fqn, format!("{:?}", c.kind), c.file);
            }
            eprintln!();
            eprintln!("Use the full FQN for an exact match.");
            return Ok(());
        }
    };

    // Query parents: edges where source_fqn = fqn AND kind = "Inherits"
    // (this node inherits from target_fqn)
    let mut parents_stmt = conn.prepare(
        "SELECT target_fqn FROM edges WHERE source_fqn = ?1 AND kind = 'Inherits'",
    )?;
    let parents: Vec<String> = parents_stmt
        .query_map(rusqlite::params![&node.fqn], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Query children: edges where target_fqn = fqn AND kind = "Inherits"
    // (other nodes inherit from this node)
    let mut children_stmt = conn.prepare(
        "SELECT source_fqn FROM edges WHERE target_fqn = ?1 AND kind = 'Inherits'",
    )?;
    let children: Vec<String> = children_stmt
        .query_map(rusqlite::params![&node.fqn], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Query interfaces: edges where source_fqn = fqn AND kind = "Implements"
    let mut implements_stmt = conn.prepare(
        "SELECT target_fqn FROM edges WHERE source_fqn = ?1 AND kind = 'Implements'",
    )?;
    let interfaces: Vec<String> = implements_stmt
        .query_map(rusqlite::params![&node.fqn], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Query implementors: edges where target_fqn = fqn AND kind = "Implements"
    // (other nodes implement this interface)
    let mut implementors_stmt = conn.prepare(
        "SELECT source_fqn FROM edges WHERE target_fqn = ?1 AND kind = 'Implements'",
    )?;
    let implementors: Vec<String> = implementors_stmt
        .query_map(rusqlite::params![&node.fqn], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Print the hierarchy tree
    println!();
    println!("HIERARCHY: {}", node.fqn);
    println!("{}", "━".repeat(50));
    println!();
    println!("  Kind: {:?}", node.kind);
    println!("  File: {}:{}", node.file, node.start_line);
    println!();

    // Parents (superclasses)
    if parents.is_empty() {
        println!("  Extends: (none, root class)");
    } else {
        println!("  Extends:");
        for parent in &parents {
            println!("    ▲ {}", parent);
        }
    }

    // Interfaces
    if !interfaces.is_empty() {
        println!();
        println!("  Implements:");
        for iface in &interfaces {
            println!("    ◆ {}", iface);
        }
    }

    // The node itself
    println!();
    println!("  ─── {} ───", node.fqn);
    println!();

    // Children (subclasses)
    if children.is_empty() {
        println!("  Children: (none, leaf class)");
    } else {
        println!("  Children ({}):", children.len());
        for child in &children {
            println!("    ▼ {}", child);
        }
    }

    // Implementors (if this is an interface/trait)
    if !implementors.is_empty() {
        println!();
        println!("  Implemented by ({}):", implementors.len());
        for imp in &implementors {
            println!("    ◇ {}", imp);
        }
    }

    println!();
    println!("{}", "━".repeat(50));
    println!();

    Ok(())
}
