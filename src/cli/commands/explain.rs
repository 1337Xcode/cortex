//! Explain command: offline function explanation from the graph.
//!
//! `cortex explain DatabasePool.acquire` prints what a function does,
//! what calls it, what it calls, and any security flags - all from
//! the graph, zero LLM calls.

use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::{graph, memory};

/// Run the explain command.
pub fn run(fqn: &str, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    // Find the node
    let node = graph::find_node_by_fqn(&conn, fqn)?;
    let node = match node {
        Some(n) => n,
        None => {
            let candidates = graph::find_nodes_by_pattern(&conn, &format!("*{}*", fqn), None, 5)?;
            if candidates.is_empty() {
                eprintln!("Node not found: {}", fqn);
                return Ok(());
            }
            eprintln!("Exact match not found. Did you mean:");
            for c in &candidates {
                eprintln!("  {} ({})", c.fqn, c.file);
            }
            return Ok(());
        }
    };

    println!();
    println!("EXPLAIN — {}", node.fqn);
    println!("{}", "━".repeat(50));
    println!();
    println!("  Kind:     {:?}", node.kind);
    println!("  File:     {}:{}-{}", node.file, node.start_line, node.end_line);
    println!("  Lines:    {}", node.end_line - node.start_line + 1);

    // Callers
    let callers = graph::trace_callers(&conn, fqn, 1)?;
    println!();
    if callers.is_empty() {
        println!("  Called by: nothing (entry point or dead code)");
    } else {
        println!("  Called by ({}):", callers.len());
        for caller in callers.iter().take(10) {
            println!("    ← {} ({})", caller.fqn, caller.file);
        }
        if callers.len() > 10 {
            println!("    ... and {} more", callers.len() - 10);
        }
    }

    // Callees
    let callees = graph::trace_callees(&conn, fqn, 1)?;
    println!();
    if callees.is_empty() {
        println!("  Calls:    nothing (leaf function)");
    } else {
        println!("  Calls ({}):", callees.len());
        for callee in callees.iter().take(10) {
            println!("    → {} ({})", callee.fqn, callee.file);
        }
        if callees.len() > 10 {
            println!("    ... and {} more", callees.len() - 10);
        }
    }

    // Security flags
    if let Some(attrs) = node.attributes.get("taint_source") {
        println!();
        println!("  ⚠ SECURITY: Taint source ({})", attrs);
    }
    if let Some(attrs) = node.attributes.get("taint_sink") {
        println!();
        println!("  ⚠ SECURITY: Taint sink ({})", attrs);
    }

    // Observations (memory)
    let observations = memory::read_observations(&conn, fqn, true)?;
    if !observations.is_empty() {
        println!();
        println!("  Observations ({}):", observations.len());
        for obs in &observations {
            let stale_marker = if obs.status == "stale" { " [STALE]" } else { "" };
            println!("    • {}{}", obs.observation_text, stale_marker);
        }
    }

    println!();
    println!("{}", "━".repeat(50));
    println!();

    Ok(())
}
