//! CLI query subcommand implementations.
//!
//! Each function gets a read connection from the store, calls the appropriate
//! graph query function, serializes the result as pretty-printed JSON, and
//! prints to stdout.

use crate::store::db::StoreManager;
use crate::store::queries::graph;
use crate::store::types::NodeKind;

/// Parse a kind string to NodeKind.
fn parse_kind(kind: &str) -> Option<NodeKind> {
    match kind {
        "Function" => Some(NodeKind::Function),
        "Class" => Some(NodeKind::Class),
        "Module" => Some(NodeKind::Module),
        "Route" => Some(NodeKind::Route),
        "Interface" => Some(NodeKind::Interface),
        "Type" => Some(NodeKind::Type),
        _ => None,
    }
}

/// Trace callers of a fully qualified name and print as JSON.
pub fn callers(fqn: &str, depth: u32, store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let results = graph::trace_callers(&conn, fqn, depth)?;
    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");
    Ok(())
}

/// Trace callees of a fully qualified name and print as JSON.
pub fn callees(fqn: &str, depth: u32, store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let results = graph::trace_callees(&conn, fqn, depth)?;
    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");
    Ok(())
}

/// Find nodes matching a pattern with optional kind filter and print as JSON.
pub fn find(
    pattern: &str,
    kind: Option<&str>,
    limit: usize,
    store: &StoreManager,
) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let kind_filter = kind.and_then(parse_kind);
    let results = graph::find_nodes_by_pattern(&conn, pattern, kind_filter, limit)?;
    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");
    Ok(())
}

/// Print architecture summary as JSON.
pub fn architecture(store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let summary = graph::get_architecture_summary(&conn)?;
    let json = serde_json::to_string_pretty(&summary)?;
    println!("{json}");
    Ok(())
}

/// Find dead code and print as JSON.
pub fn dead_code(limit: usize, store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let results = graph::find_dead_code(&conn, limit)?;
    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");
    Ok(())
}

/// Compute blast radius for a fully qualified name and print as JSON.
pub fn blast_radius(fqn: &str, depth: u32, store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();
    let results = graph::blast_radius(&conn, fqn, depth)?;
    let json = serde_json::to_string_pretty(&results)?;
    println!("{json}");
    Ok(())
}

/// Detect changes since a timestamp and print as JSON.
pub fn changes(since: u64, store: &StoreManager) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    let mut stmt = conn.prepare(
        "SELECT n.fqn, n.kind, n.file, n.start_line, n.end_line \
         FROM nodes n \
         INNER JOIN file_snapshots fs ON n.file = fs.file \
         WHERE fs.indexed_at > ?1 \
         ORDER BY fs.indexed_at DESC",
    )?;

    let rows: Vec<serde_json::Value> = stmt
        .query_map([since], |row| {
            let fqn: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let file: String = row.get(2)?;
            let start_line: u32 = row.get(3)?;
            let end_line: u32 = row.get(4)?;
            Ok(serde_json::json!({
                "fqn": fqn,
                "kind": kind,
                "file": file,
                "start_line": start_line,
                "end_line": end_line,
                "change_kind": "modified",
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let total_changes = rows.len();
    let risk_score: f64 = if total_changes == 0 {
        0.0
    } else {
        (total_changes as f64 / 10.0).min(1.0)
    };

    let result = serde_json::json!({
        "since": since,
        "changes": rows,
        "total_changes": total_changes,
        "risk_score": risk_score,
    });

    let json = serde_json::to_string_pretty(&result)?;
    println!("{json}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::queries::graph;
    use rusqlite::Connection;

    /// Creates an in-memory SQLite connection with migrations applied.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("failed to enable foreign keys");

        let migration_0001 = include_str!("../../../migrations/0001_initial_schema.sql");
        conn.execute_batch(migration_0001)
            .expect("failed to apply migration 0001");

        conn
    }

    fn insert_node(conn: &Connection, fqn: &str, kind: &str, file: &str, start_line: u32) {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![fqn, kind, file, start_line, start_line + 10, "hash_a", 1000, "{}"],
        )
        .unwrap();
    }

    fn insert_edge(conn: &Connection, source: &str, target: &str, kind: &str, confidence: f64) {
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![source, target, kind, confidence, "{}"],
        )
        .unwrap();
    }

    #[test]
    fn test_callers_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls", 1.0);

        let results = graph::trace_callers(&conn, "src/b.rs::func_b", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["fqn"], "src/a.rs::func_a");
        assert_eq!(arr[0]["depth"], 1);
    }

    #[test]
    fn test_callees_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls", 1.0);

        let results = graph::trace_callees(&conn, "src/a.rs::func_a", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["fqn"], "src/b.rs::func_b");
        assert_eq!(arr[0]["depth"], 1);
    }

    #[test]
    fn test_find_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/a.rs::bar", "Class", "src/a.rs", 20);

        let results = graph::find_nodes_by_pattern(&conn, "src/a.rs::*", None, 50).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_find_with_kind_filter() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/a.rs::bar", "Class", "src/a.rs", 20);

        let results =
            graph::find_nodes_by_pattern(&conn, "src/a.rs::*", Some(NodeKind::Function), 50)
                .unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["fqn"], "src/a.rs::foo");
    }

    #[test]
    fn test_architecture_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::bar", "Class", "src/b.rs", 1);

        let summary = graph::get_architecture_summary(&conn).unwrap();
        let json = serde_json::to_string_pretty(&summary).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_object());
        assert_eq!(parsed["total_nodes"], 2);
        assert!(parsed["counts_by_kind"].is_array());
        assert!(parsed["top_level_modules"].is_array());
        assert!(parsed["entry_points"].is_array());
    }

    #[test]
    fn test_dead_code_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::orphan", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::used", "Function", "src/b.rs", 1);
        insert_node(&conn, "src/c.rs::caller", "Function", "src/c.rs", 1);
        insert_edge(&conn, "src/c.rs::caller", "src/b.rs::used", "Calls", 1.0);

        let results = graph::find_dead_code(&conn, 50).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        // orphan and caller have no inbound Calls edges
        let fqns: Vec<&str> = arr.iter().map(|n| n["fqn"].as_str().unwrap()).collect();
        assert!(fqns.contains(&"src/a.rs::orphan"));
        assert!(fqns.contains(&"src/c.rs::caller"));
        assert!(!fqns.contains(&"src/b.rs::used"));
    }

    #[test]
    fn test_blast_radius_produces_correct_json() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1);
        // b calls a, c calls a => blast radius of a includes b and c
        insert_edge(&conn, "src/b.rs::func_b", "src/a.rs::func_a", "Calls", 1.0);
        insert_edge(&conn, "src/c.rs::func_c", "src/a.rs::func_a", "Calls", 0.9);

        let results = graph::blast_radius(&conn, "src/a.rs::func_a", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let fqns: Vec<&str> = arr.iter().map(|n| n["fqn"].as_str().unwrap()).collect();
        assert!(fqns.contains(&"src/b.rs::func_b"));
        assert!(fqns.contains(&"src/c.rs::func_c"));
    }

    #[test]
    fn test_non_existent_fqn_returns_empty_callers() {
        let conn = setup_db();
        let results = graph::trace_callers(&conn, "nonexistent::foo", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_non_existent_fqn_returns_empty_callees() {
        let conn = setup_db();
        let results = graph::trace_callees(&conn, "nonexistent::foo", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_non_existent_fqn_returns_empty_blast_radius() {
        let conn = setup_db();
        let results = graph::blast_radius(&conn, "nonexistent::foo", 3).unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_depth_respected_callers() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1);
        // a -> b -> c
        insert_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls", 1.0);
        insert_edge(&conn, "src/b.rs::func_b", "src/c.rs::func_c", "Calls", 0.9);

        // Callers of c at depth 1 should only find b
        let results = graph::trace_callers(&conn, "src/c.rs::func_c", 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].fqn, "src/b.rs::func_b");

        // Callers of c at depth 2 should find both a and b
        let results = graph::trace_callers(&conn, "src/c.rs::func_c", 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_depth_respected_callees() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1);
        // a -> b -> c
        insert_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls", 1.0);
        insert_edge(&conn, "src/b.rs::func_b", "src/c.rs::func_c", "Calls", 0.9);

        // Callees of a at depth 1 should only find b
        let results = graph::trace_callees(&conn, "src/a.rs::func_a", 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].fqn, "src/b.rs::func_b");

        // Callees of a at depth 2 should find both b and c
        let results = graph::trace_callees(&conn, "src/a.rs::func_a", 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_kind_valid() {
        assert_eq!(parse_kind("Function"), Some(NodeKind::Function));
        assert_eq!(parse_kind("Class"), Some(NodeKind::Class));
        assert_eq!(parse_kind("Module"), Some(NodeKind::Module));
        assert_eq!(parse_kind("Route"), Some(NodeKind::Route));
        assert_eq!(parse_kind("Interface"), Some(NodeKind::Interface));
        assert_eq!(parse_kind("Type"), Some(NodeKind::Type));
    }

    #[test]
    fn test_parse_kind_invalid() {
        assert_eq!(parse_kind("invalid"), None);
        assert_eq!(parse_kind("function"), None);
        assert_eq!(parse_kind(""), None);
    }
}
