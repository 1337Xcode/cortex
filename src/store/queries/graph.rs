//! Graph structural queries using recursive CTEs for traversal.
//!
//! Provides functions for node lookup, call chain tracing (callers/callees),
//! architecture summary, dead code detection, and blast radius analysis.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::config::MAX_GRAPH_QUERY_RESULTS;
use crate::error::StoreError;
use crate::store::types::{Node, NodeKind};

// ---------------------------------------------------------------------------
// Additional types for graph queries
// ---------------------------------------------------------------------------

/// A node in a call path with traversal metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallPathNode {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub start_line: u32,
    pub depth: u32,
    pub confidence: f64,
    /// Number of inbound "Calls" edges to this node (relevance indicator).
    pub call_count: u32,
}

/// Summary of the architecture derived from the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureSummary {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub files_indexed: usize,
    pub languages: Vec<String>,
    pub counts_by_kind: Vec<(String, usize)>,
    pub top_level_modules: Vec<String>,
    pub entry_points: Vec<String>,
}

/// A hotspot node: a function with many callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotspotNode {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub caller_count: u32,
}

/// An edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub caller: String,
    pub callee: String,
}

// ---------------------------------------------------------------------------
// Helper: deserialize NodeKind from database string
// ---------------------------------------------------------------------------

fn parse_node_kind(s: &str) -> NodeKind {
    match s {
        "Function" => NodeKind::Function,
        "Class" => NodeKind::Class,
        "Module" => NodeKind::Module,
        "Route" => NodeKind::Route,
        "Interface" => NodeKind::Interface,
        "Type" => NodeKind::Type,
        "Enum" => NodeKind::Enum,
        "Constant" => NodeKind::Constant,
        "TypeAlias" => NodeKind::TypeAlias,
        "Trait" => NodeKind::Trait,
        "Namespace" => NodeKind::Namespace,
        _ => NodeKind::Function, // fallback
    }
}

/// Reads a Node from a rusqlite Row (columns: fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes).
fn node_from_row(row: &rusqlite::Row) -> rusqlite::Result<Node> {
    let kind_str: String = row.get(1)?;
    let attributes_str: String = row.get(7)?;
    Ok(Node {
        fqn: row.get(0)?,
        kind: parse_node_kind(&kind_str),
        file: row.get(2)?,
        start_line: row.get(3)?,
        end_line: row.get(4)?,
        file_hash: row.get(5)?,
        indexed_at: row.get(6)?,
        attributes: serde_json::from_str(&attributes_str).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Query functions
// ---------------------------------------------------------------------------

/// Exact lookup of a node by its fully-qualified name.
pub fn find_node_by_fqn(conn: &Connection, fqn: &str) -> Result<Option<Node>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes \
             FROM nodes WHERE fqn = ?1",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare find_node_by_fqn: {}", e),
        })?;

    let result = stmt
        .query_row(rusqlite::params![fqn], node_from_row)
        .optional()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute find_node_by_fqn: {}", e),
        })?;

    Ok(result)
}

/// Find nodes matching a glob pattern with optional kind filter.
///
/// The pattern uses `*` as wildcard (converted to SQL `%` for LIKE).
/// Results are limited to `limit` (defaults to MAX_GRAPH_QUERY_RESULTS).
pub fn find_nodes_by_pattern(
    conn: &Connection,
    pattern: &str,
    kind: Option<NodeKind>,
    limit: usize,
) -> Result<Vec<Node>, StoreError> {
    // Convert glob pattern (* -> %) for SQL LIKE
    let sql_pattern = pattern.replace('*', "%");
    let effective_limit = if limit == 0 {
        MAX_GRAPH_QUERY_RESULTS
    } else {
        limit
    };

    let nodes = if let Some(ref kind_filter) = kind {
        let kind_str = serialize_node_kind(kind_filter);
        let mut stmt = conn
            .prepare(
                "SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes \
                 FROM nodes WHERE fqn LIKE ?1 AND kind = ?2 LIMIT ?3",
            )
            .map_err(|e| StoreError::QueryFailed {
                reason: format!("failed to prepare find_nodes_by_pattern: {}", e),
            })?;

        stmt.query_map(
            rusqlite::params![&sql_pattern, kind_str, effective_limit as i64],
            node_from_row,
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute find_nodes_by_pattern: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect find_nodes_by_pattern results: {}", e),
        })?
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes \
                 FROM nodes WHERE fqn LIKE ?1 LIMIT ?2",
            )
            .map_err(|e| StoreError::QueryFailed {
                reason: format!("failed to prepare find_nodes_by_pattern: {}", e),
            })?;

        stmt.query_map(
            rusqlite::params![&sql_pattern, effective_limit as i64],
            node_from_row,
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute find_nodes_by_pattern: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect find_nodes_by_pattern results: {}", e),
        })?
    };

    Ok(nodes)
}

/// Count inbound "Calls" edges for each FQN in the given list.
///
/// Returns a map from FQN to inbound edge count.
fn count_inbound_edges(
    conn: &Connection,
    fqns: &[&str],
) -> Result<std::collections::HashMap<String, u32>, StoreError> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, u32> = HashMap::new();

    if fqns.is_empty() {
        return Ok(counts);
    }

    // Query inbound edge counts for all FQNs in the result set
    let mut stmt = conn
        .prepare(
            "SELECT target_fqn, COUNT(*) FROM edges \
             WHERE kind = 'Calls' GROUP BY target_fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare count_inbound_edges: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((fqn, count as u32))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute count_inbound_edges: {}", e),
        })?;

    for row in rows {
        let (fqn, count) = row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read count_inbound_edges row: {}", e),
        })?;
        counts.insert(fqn, count);
    }

    Ok(counts)
}

/// Trace all callers of a node using a recursive CTE (BFS over inbound Calls edges).
///
/// Depth is capped at 5 (MAX_TRAVERSAL_DEPTH).
/// Results are ranked by call_count (most-connected first), with confidence as tiebreaker.
pub fn trace_callers(
    conn: &Connection,
    fqn: &str,
    depth: u32,
) -> Result<Vec<CallPathNode>, StoreError> {
    let effective_depth = depth.min(crate::config::MAX_TRAVERSAL_DEPTH);

    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE callers(fqn, depth, confidence) AS ( \
                 SELECT source_fqn, 1, confidence FROM edges WHERE target_fqn = ?1 AND kind = 'Calls' \
                 UNION ALL \
                 SELECT e.source_fqn, c.depth + 1, MIN(c.confidence, e.confidence) \
                 FROM edges e JOIN callers c ON e.target_fqn = c.fqn \
                 WHERE c.depth < ?2 AND e.kind = 'Calls' \
             ) \
             SELECT DISTINCT n.fqn, n.kind, n.file, n.start_line, c.depth, c.confidence \
             FROM callers c JOIN nodes n ON n.fqn = c.fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare trace_callers: {}", e),
        })?;

    let mut results: Vec<CallPathNode> = stmt
        .query_map(rusqlite::params![fqn, effective_depth], |row| {
            Ok(CallPathNode {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                start_line: row.get(3)?,
                depth: row.get(4)?,
                confidence: row.get(5)?,
                call_count: 0, // will be filled below
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute trace_callers: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect trace_callers results: {}", e),
        })?;

    // Populate call_count from inbound edge counts
    let fqn_refs: Vec<&str> = results.iter().map(|n| n.fqn.as_str()).collect();
    let edge_counts = count_inbound_edges(conn, &fqn_refs)?;
    for node in &mut results {
        node.call_count = edge_counts.get(&node.fqn).copied().unwrap_or(0);
    }

    // Sort by call_count descending, then confidence descending as tiebreaker
    results.sort_by(|a, b| {
        b.call_count
            .cmp(&a.call_count)
            .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
    });

    Ok(results)
}

/// Trace all callees of a node using a recursive CTE (BFS over outbound Calls edges).
///
/// Depth is capped at 5 (MAX_TRAVERSAL_DEPTH).
/// Results are ranked by call_count (most-connected first), with confidence as tiebreaker.
pub fn trace_callees(
    conn: &Connection,
    fqn: &str,
    depth: u32,
) -> Result<Vec<CallPathNode>, StoreError> {
    let effective_depth = depth.min(crate::config::MAX_TRAVERSAL_DEPTH);

    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE callees(fqn, depth, confidence) AS ( \
                 SELECT target_fqn, 1, confidence FROM edges WHERE source_fqn = ?1 AND kind = 'Calls' \
                 UNION ALL \
                 SELECT e.target_fqn, c.depth + 1, MIN(c.confidence, e.confidence) \
                 FROM edges e JOIN callees c ON e.source_fqn = c.fqn \
                 WHERE c.depth < ?2 AND e.kind = 'Calls' \
             ) \
             SELECT DISTINCT n.fqn, n.kind, n.file, n.start_line, c.depth, c.confidence \
             FROM callees c JOIN nodes n ON n.fqn = c.fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare trace_callees: {}", e),
        })?;

    let mut results: Vec<CallPathNode> = stmt
        .query_map(rusqlite::params![fqn, effective_depth], |row| {
            Ok(CallPathNode {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                start_line: row.get(3)?,
                depth: row.get(4)?,
                confidence: row.get(5)?,
                call_count: 0, // will be filled below
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute trace_callees: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect trace_callees results: {}", e),
        })?;

    // Populate call_count from inbound edge counts
    let fqn_refs: Vec<&str> = results.iter().map(|n| n.fqn.as_str()).collect();
    let edge_counts = count_inbound_edges(conn, &fqn_refs)?;
    for node in &mut results {
        node.call_count = edge_counts.get(&node.fqn).copied().unwrap_or(0);
    }

    // Sort by call_count descending, then confidence descending as tiebreaker
    results.sort_by(|a, b| {
        b.call_count
            .cmp(&a.call_count)
            .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
    });

    Ok(results)
}

/// Get an architecture summary: counts by kind, top-level modules, entry points.
pub fn get_architecture_summary(conn: &Connection) -> Result<ArchitectureSummary, StoreError> {
    // Total nodes
    let total_nodes: usize = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to count nodes: {}", e),
        })? as usize;

    // Counts by kind
    let mut stmt = conn
        .prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind ORDER BY COUNT(*) DESC")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare counts_by_kind: {}", e),
        })?;

    let counts_by_kind: Vec<(String, usize)> = stmt
        .query_map([], |row| {
            let kind: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((kind, count as usize))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute counts_by_kind: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect counts_by_kind: {}", e),
        })?;

    // Top-level modules: distinct first path segment of file
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT \
                 CASE \
                     WHEN INSTR(file, '/') > 0 THEN SUBSTR(file, 1, INSTR(file, '/') - 1) \
                     ELSE file \
                 END AS top_module \
             FROM nodes ORDER BY top_module",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare top_level_modules: {}", e),
        })?;

    let top_level_modules: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute top_level_modules: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect top_level_modules: {}", e),
        })?;

    // Entry points: Route nodes + nodes with "main" in FQN
    let mut stmt = conn
        .prepare(
            "SELECT fqn FROM nodes WHERE kind = 'Route' OR fqn LIKE '%main%' ORDER BY fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare entry_points: {}", e),
        })?;

    let entry_points: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute entry_points: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect entry_points: {}", e),
        })?;

    Ok(ArchitectureSummary {
        total_nodes,
        total_edges: conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0) as usize,
        files_indexed: conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0) as usize,
        languages: detect_languages(&top_level_modules, conn),
        counts_by_kind,
        top_level_modules,
        entry_points,
    })
}

/// Detect languages from file extensions in the database.
fn detect_languages(
    _modules: &[String],
    conn: &Connection,
) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT \
                 CASE \
                     WHEN file LIKE '%.py' THEN 'Python' \
                     WHEN file LIKE '%.ts' OR file LIKE '%.tsx' THEN 'TypeScript' \
                     WHEN file LIKE '%.js' OR file LIKE '%.jsx' THEN 'JavaScript' \
                     WHEN file LIKE '%.go' THEN 'Go' \
                     WHEN file LIKE '%.rs' THEN 'Rust' \
                     WHEN file LIKE '%.java' THEN 'Java' \
                     WHEN file LIKE '%.cs' THEN 'C#' \
                     WHEN file LIKE '%.cpp' OR file LIKE '%.cc' OR file LIKE '%.h' THEN 'C++' \
                     WHEN file LIKE '%.c' THEN 'C' \
                     WHEN file LIKE '%.rb' THEN 'Ruby' \
                     WHEN file LIKE '%.kt' THEN 'Kotlin' \
                     WHEN file LIKE '%.swift' THEN 'Swift' \
                     WHEN file LIKE '%.php' THEN 'PHP' \
                     ELSE NULL \
                 END AS lang \
             FROM nodes WHERE lang IS NOT NULL ORDER BY lang",
        )
        .ok();

    match stmt {
        Some(ref mut s) => s
            .query_map([], |row| row.get::<_, String>(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Get the most-called nodes (hotspots) sorted by inbound edge count.
pub fn get_hotspot_nodes(conn: &Connection, limit: usize) -> Result<Vec<HotspotNode>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT n.fqn, n.kind, n.file, COUNT(e.source_fqn) as caller_count \
             FROM nodes n \
             LEFT JOIN edges e ON e.target_fqn = n.fqn AND e.kind = 'Calls' \
             GROUP BY n.fqn \
             ORDER BY caller_count DESC \
             LIMIT ?1",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare get_hotspot_nodes: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(HotspotNode {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                caller_count: row.get(3)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute get_hotspot_nodes: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect get_hotspot_nodes: {}", e),
        })?;

    Ok(results)
}

/// Get all edges (up to a limit) for graph visualization.
pub fn get_all_edges(conn: &Connection, limit: usize) -> Result<Vec<EdgeRecord>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT source_fqn, target_fqn FROM edges LIMIT ?1")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare get_all_edges: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(EdgeRecord {
                caller: row.get(0)?,
                callee: row.get(1)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute get_all_edges: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect get_all_edges: {}", e),
        })?;

    Ok(results)
}

/// Find dead code: nodes with zero inbound Calls edges, excluding Route kind
/// and nodes with "main" or "test" in FQN.
pub fn find_dead_code(conn: &Connection, limit: usize) -> Result<Vec<Node>, StoreError> {
    let effective_limit = if limit == 0 {
        MAX_GRAPH_QUERY_RESULTS
    } else {
        limit
    };

    let mut stmt = conn
        .prepare(
            "SELECT n.fqn, n.kind, n.file, n.start_line, n.end_line, n.file_hash, n.indexed_at, n.attributes \
             FROM nodes n \
             WHERE n.kind != 'Route' \
             AND n.fqn NOT LIKE '%main%' \
             AND n.fqn NOT LIKE '%test%' \
             AND NOT EXISTS ( \
                 SELECT 1 FROM edges e WHERE e.target_fqn = n.fqn AND e.kind = 'Calls' \
             ) \
             LIMIT ?1",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare find_dead_code: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![effective_limit as i64], node_from_row)
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute find_dead_code: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect find_dead_code results: {}", e),
        })?;

    Ok(results)
}

/// Compute the blast radius of a node: all nodes that transitively depend on it
/// (BFS over inbound edges of all kinds).
///
/// Depth is capped at 5 (MAX_TRAVERSAL_DEPTH).
pub fn blast_radius(
    conn: &Connection,
    fqn: &str,
    depth: u32,
) -> Result<Vec<Node>, StoreError> {
    let effective_depth = depth.min(crate::config::MAX_TRAVERSAL_DEPTH);

    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE dependents(fqn, depth) AS ( \
                 SELECT source_fqn, 1 FROM edges WHERE target_fqn = ?1 \
                 UNION ALL \
                 SELECT e.source_fqn, d.depth + 1 \
                 FROM edges e JOIN dependents d ON e.target_fqn = d.fqn \
                 WHERE d.depth < ?2 \
             ) \
             SELECT DISTINCT n.fqn, n.kind, n.file, n.start_line, n.end_line, n.file_hash, n.indexed_at, n.attributes \
             FROM dependents d JOIN nodes n ON n.fqn = d.fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare blast_radius: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![fqn, effective_depth], node_from_row)
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute blast_radius: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect blast_radius results: {}", e),
        })?;

    Ok(results)
}

// ---------------------------------------------------------------------------
// Helper: serialize NodeKind to string
// ---------------------------------------------------------------------------

fn serialize_node_kind(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Function => "Function",
        NodeKind::Class => "Class",
        NodeKind::Module => "Module",
        NodeKind::Route => "Route",
        NodeKind::Interface => "Interface",
        NodeKind::Type => "Type",
        NodeKind::Enum => "Enum",
        NodeKind::Constant => "Constant",
        NodeKind::TypeAlias => "TypeAlias",
        NodeKind::Trait => "Trait",
        NodeKind::Namespace => "Namespace",
    }
}

// ---------------------------------------------------------------------------
// Extension trait for optional query results
// ---------------------------------------------------------------------------

/// Extension trait to add `.optional()` to rusqlite Results.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::types::NodeKind;
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

    /// Sets up a call chain: A -> B -> C -> D
    fn setup_call_chain(conn: &Connection) {
        insert_node(conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);
        insert_node(conn, "src/b.rs::func_b", "Function", "src/b.rs", 1);
        insert_node(conn, "src/c.rs::func_c", "Function", "src/c.rs", 1);
        insert_node(conn, "src/d.rs::func_d", "Function", "src/d.rs", 1);

        insert_edge(conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls", 1.0);
        insert_edge(conn, "src/b.rs::func_b", "src/c.rs::func_c", "Calls", 0.9);
        insert_edge(conn, "src/c.rs::func_c", "src/d.rs::func_d", "Calls", 0.8);
    }

    // -----------------------------------------------------------------------
    // find_node_by_fqn tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_node_by_fqn_found() {
        let conn = setup_db();
        insert_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1);

        let result = find_node_by_fqn(&conn, "src/main.rs::main").unwrap();
        assert!(result.is_some());
        let node = result.unwrap();
        assert_eq!(node.fqn, "src/main.rs::main");
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.file, "src/main.rs");
    }

    #[test]
    fn test_find_node_by_fqn_not_found() {
        let conn = setup_db();
        let result = find_node_by_fqn(&conn, "nonexistent::foo").unwrap();
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // find_nodes_by_pattern tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_nodes_by_pattern_wildcard() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/a.rs::bar", "Function", "src/a.rs", 20);
        insert_node(&conn, "src/b.rs::baz", "Class", "src/b.rs", 1);

        // Pattern matching all in src/a.rs
        let results = find_nodes_by_pattern(&conn, "src/a.rs::*", None, 100).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_find_nodes_by_pattern_with_kind_filter() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/a.rs::MyClass", "Class", "src/a.rs", 20);

        let results =
            find_nodes_by_pattern(&conn, "src/a.rs::*", Some(NodeKind::Class), 100).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].fqn, "src/a.rs::MyClass");
    }

    #[test]
    fn test_find_nodes_by_pattern_limit() {
        let conn = setup_db();
        for i in 0..10 {
            insert_node(
                &conn,
                &format!("src/mod.rs::func_{}", i),
                "Function",
                "src/mod.rs",
                i * 10,
            );
        }

        let results = find_nodes_by_pattern(&conn, "src/mod.rs::*", None, 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    // -----------------------------------------------------------------------
    // trace_callers tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_trace_callers_single_depth() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callers of func_b at depth 1 should be func_a
        let callers = trace_callers(&conn, "src/b.rs::func_b", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].fqn, "src/a.rs::func_a");
        assert_eq!(callers[0].depth, 1);
    }

    #[test]
    fn test_trace_callers_multi_depth() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callers of func_d at depth 5 should find func_c (depth 1), func_b (depth 2), func_a (depth 3)
        let callers = trace_callers(&conn, "src/d.rs::func_d", 5).unwrap();
        assert_eq!(callers.len(), 3);

        let fqns: Vec<&str> = callers.iter().map(|c| c.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/c.rs::func_c"));
        assert!(fqns.contains(&"src/b.rs::func_b"));
        assert!(fqns.contains(&"src/a.rs::func_a"));
    }

    #[test]
    fn test_trace_callers_depth_limiting() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callers of func_d at depth 1 should only find func_c
        let callers = trace_callers(&conn, "src/d.rs::func_d", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].fqn, "src/c.rs::func_c");
    }

    #[test]
    fn test_trace_callers_no_callers() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // func_a has no callers
        let callers = trace_callers(&conn, "src/a.rs::func_a", 5).unwrap();
        assert!(callers.is_empty());
    }

    #[test]
    fn test_trace_callers_confidence_propagation() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callers of func_d: func_c has confidence 0.8, func_b should have min(0.9, 0.8) = 0.8
        let callers = trace_callers(&conn, "src/d.rs::func_d", 5).unwrap();
        for caller in &callers {
            if caller.fqn == "src/c.rs::func_c" {
                assert!((caller.confidence - 0.8).abs() < 0.01);
            }
            if caller.fqn == "src/b.rs::func_b" {
                // min(0.9, 0.8) = 0.8
                assert!(caller.confidence <= 0.9);
            }
        }
    }

    // -----------------------------------------------------------------------
    // trace_callees tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_trace_callees_single_depth() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callees of func_a at depth 1 should be func_b
        let callees = trace_callees(&conn, "src/a.rs::func_a", 1).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].fqn, "src/b.rs::func_b");
        assert_eq!(callees[0].depth, 1);
    }

    #[test]
    fn test_trace_callees_multi_depth() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callees of func_a at depth 5 should find func_b, func_c, func_d
        let callees = trace_callees(&conn, "src/a.rs::func_a", 5).unwrap();
        assert_eq!(callees.len(), 3);

        let fqns: Vec<&str> = callees.iter().map(|c| c.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/b.rs::func_b"));
        assert!(fqns.contains(&"src/c.rs::func_c"));
        assert!(fqns.contains(&"src/d.rs::func_d"));
    }

    #[test]
    fn test_trace_callees_depth_limiting() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Callees of func_a at depth 2 should find func_b (depth 1) and func_c (depth 2)
        let callees = trace_callees(&conn, "src/a.rs::func_a", 2).unwrap();
        assert_eq!(callees.len(), 2);
    }

    #[test]
    fn test_trace_callees_no_callees() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // func_d has no callees
        let callees = trace_callees(&conn, "src/d.rs::func_d", 5).unwrap();
        assert!(callees.is_empty());
    }

    // -----------------------------------------------------------------------
    // get_architecture_summary tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_architecture_summary_counts() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/a.rs::bar", "Function", "src/a.rs", 20);
        insert_node(&conn, "src/b.rs::MyClass", "Class", "src/b.rs", 1);
        insert_node(&conn, "routes/api.rs::get_users", "Route", "routes/api.rs", 1);

        let summary = get_architecture_summary(&conn).unwrap();
        assert_eq!(summary.total_nodes, 4);
        assert_eq!(summary.counts_by_kind.len(), 3); // Function, Class, Route
    }

    #[test]
    fn test_architecture_summary_top_level_modules() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1);
        insert_node(&conn, "lib/b.rs::bar", "Function", "lib/b.rs", 1);
        insert_node(&conn, "src/c.rs::baz", "Function", "src/c.rs", 1);

        let summary = get_architecture_summary(&conn).unwrap();
        assert!(summary.top_level_modules.contains(&"src".to_string()));
        assert!(summary.top_level_modules.contains(&"lib".to_string()));
        assert_eq!(summary.top_level_modules.len(), 2);
    }

    #[test]
    fn test_architecture_summary_entry_points() {
        let conn = setup_db();
        insert_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1);
        insert_node(&conn, "src/routes.rs::get_users", "Route", "src/routes.rs", 1);
        insert_node(&conn, "src/lib.rs::helper", "Function", "src/lib.rs", 1);

        let summary = get_architecture_summary(&conn).unwrap();
        assert!(summary.entry_points.contains(&"src/main.rs::main".to_string()));
        assert!(summary.entry_points.contains(&"src/routes.rs::get_users".to_string()));
        assert!(!summary.entry_points.contains(&"src/lib.rs::helper".to_string()));
    }

    // -----------------------------------------------------------------------
    // find_dead_code tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_dead_code_basic() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::used_func", "Function", "src/a.rs", 1);
        insert_node(&conn, "src/b.rs::dead_func", "Function", "src/b.rs", 1);
        insert_node(&conn, "src/c.rs::caller", "Function", "src/c.rs", 1);

        // caller -> used_func (so used_func is NOT dead)
        insert_edge(&conn, "src/c.rs::caller", "src/a.rs::used_func", "Calls", 1.0);

        let dead = find_dead_code(&conn, 100).unwrap();
        let dead_fqns: Vec<&str> = dead.iter().map(|n| n.fqn.as_str()).collect();

        // dead_func and caller have no inbound Calls edges
        assert!(dead_fqns.contains(&"src/b.rs::dead_func"));
        assert!(dead_fqns.contains(&"src/c.rs::caller"));
        // used_func has an inbound Calls edge, so it's NOT dead
        assert!(!dead_fqns.contains(&"src/a.rs::used_func"));
    }

    #[test]
    fn test_find_dead_code_excludes_routes() {
        let conn = setup_db();
        insert_node(&conn, "src/routes.rs::get_users", "Route", "src/routes.rs", 1);
        insert_node(&conn, "src/lib.rs::orphan", "Function", "src/lib.rs", 1);

        let dead = find_dead_code(&conn, 100).unwrap();
        let dead_fqns: Vec<&str> = dead.iter().map(|n| n.fqn.as_str()).collect();

        // Route nodes are excluded from dead code
        assert!(!dead_fqns.contains(&"src/routes.rs::get_users"));
        assert!(dead_fqns.contains(&"src/lib.rs::orphan"));
    }

    #[test]
    fn test_find_dead_code_excludes_main_and_test() {
        let conn = setup_db();
        insert_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1);
        insert_node(&conn, "tests/test_foo.rs::test_it", "Function", "tests/test_foo.rs", 1);
        insert_node(&conn, "src/lib.rs::orphan", "Function", "src/lib.rs", 1);

        let dead = find_dead_code(&conn, 100).unwrap();
        let dead_fqns: Vec<&str> = dead.iter().map(|n| n.fqn.as_str()).collect();

        // main and test nodes are excluded
        assert!(!dead_fqns.contains(&"src/main.rs::main"));
        assert!(!dead_fqns.contains(&"tests/test_foo.rs::test_it"));
        assert!(dead_fqns.contains(&"src/lib.rs::orphan"));
    }

    #[test]
    fn test_find_dead_code_limit() {
        let conn = setup_db();
        for i in 0..10 {
            insert_node(
                &conn,
                &format!("src/mod.rs::func_{}", i),
                "Function",
                "src/mod.rs",
                i * 10,
            );
        }

        let dead = find_dead_code(&conn, 3).unwrap();
        assert_eq!(dead.len(), 3);
    }

    // -----------------------------------------------------------------------
    // blast_radius tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_blast_radius_basic() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Blast radius of func_d: who depends on func_d?
        // func_c calls func_d, func_b calls func_c, func_a calls func_b
        let radius = blast_radius(&conn, "src/d.rs::func_d", 5).unwrap();
        let fqns: Vec<&str> = radius.iter().map(|n| n.fqn.as_str()).collect();

        assert!(fqns.contains(&"src/c.rs::func_c"));
        assert!(fqns.contains(&"src/b.rs::func_b"));
        assert!(fqns.contains(&"src/a.rs::func_a"));
    }

    #[test]
    fn test_blast_radius_depth_limiting() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // Blast radius of func_d at depth 1: only func_c
        let radius = blast_radius(&conn, "src/d.rs::func_d", 1).unwrap();
        assert_eq!(radius.len(), 1);
        assert_eq!(radius[0].fqn, "src/c.rs::func_c");
    }

    #[test]
    fn test_blast_radius_no_dependents() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // func_a has no inbound edges
        let radius = blast_radius(&conn, "src/a.rs::func_a", 5).unwrap();
        assert!(radius.is_empty());
    }

    #[test]
    fn test_blast_radius_includes_all_edge_kinds() {
        let conn = setup_db();
        insert_node(&conn, "src/base.rs::Base", "Class", "src/base.rs", 1);
        insert_node(&conn, "src/child.rs::Child", "Class", "src/child.rs", 1);
        insert_node(&conn, "src/user.rs::use_child", "Function", "src/user.rs", 1);

        // Child inherits Base, use_child calls Child
        insert_edge(&conn, "src/child.rs::Child", "src/base.rs::Base", "Inherits", 1.0);
        insert_edge(&conn, "src/user.rs::use_child", "src/child.rs::Child", "Calls", 1.0);

        // Blast radius of Base: Child depends on it (Inherits), use_child depends on Child
        let radius = blast_radius(&conn, "src/base.rs::Base", 5).unwrap();
        let fqns: Vec<&str> = radius.iter().map(|n| n.fqn.as_str()).collect();

        assert!(fqns.contains(&"src/child.rs::Child"));
        assert!(fqns.contains(&"src/user.rs::use_child"));
    }

    // -----------------------------------------------------------------------
    // Depth capping test
    // -----------------------------------------------------------------------

    #[test]
    fn test_depth_capped_at_max_traversal_depth() {
        let conn = setup_db();
        // Create a chain of 7 nodes: n0 -> n1 -> n2 -> n3 -> n4 -> n5 -> n6
        for i in 0..7 {
            insert_node(
                &conn,
                &format!("src/n{}.rs::func", i),
                "Function",
                &format!("src/n{}.rs", i),
                1,
            );
        }
        for i in 0..6 {
            insert_edge(
                &conn,
                &format!("src/n{}.rs::func", i),
                &format!("src/n{}.rs::func", i + 1),
                "Calls",
                1.0,
            );
        }

        // trace_callees from n0 with depth 10 (should be capped to 5)
        let callees = trace_callees(&conn, "src/n0.rs::func", 10).unwrap();
        // Should find at most 5 nodes (n1 through n5), not n6
        assert!(callees.len() <= 5);
        let fqns: Vec<&str> = callees.iter().map(|c| c.fqn.as_str()).collect();
        assert!(!fqns.contains(&"src/n6.rs::func"));
    }

    // -----------------------------------------------------------------------
    // Coarse-to-fine traversal ranking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_trace_callers_ranked_by_call_count() {
        let conn = setup_db();

        // Create a target node and several callers with different inbound edge counts
        insert_node(&conn, "src/target.rs::target", "Function", "src/target.rs", 1);
        insert_node(&conn, "src/popular.rs::popular", "Function", "src/popular.rs", 1);
        insert_node(&conn, "src/rare.rs::rare", "Function", "src/rare.rs", 1);
        insert_node(&conn, "src/medium.rs::medium", "Function", "src/medium.rs", 1);

        // All three call target
        insert_edge(&conn, "src/popular.rs::popular", "src/target.rs::target", "Calls", 1.0);
        insert_edge(&conn, "src/rare.rs::rare", "src/target.rs::target", "Calls", 1.0);
        insert_edge(&conn, "src/medium.rs::medium", "src/target.rs::target", "Calls", 1.0);

        // Make "popular" heavily called by many other nodes (5 callers)
        for i in 0..5 {
            let caller_fqn = format!("src/caller{}.rs::caller{}", i, i);
            insert_node(&conn, &caller_fqn, "Function", &format!("src/caller{}.rs", i), 1);
            insert_edge(&conn, &caller_fqn, "src/popular.rs::popular", "Calls", 1.0);
        }

        // Make "medium" moderately called (2 callers)
        for i in 0..2 {
            let caller_fqn = format!("src/med_caller{}.rs::med_caller{}", i, i);
            insert_node(&conn, &caller_fqn, "Function", &format!("src/med_caller{}.rs", i), 1);
            insert_edge(&conn, &caller_fqn, "src/medium.rs::medium", "Calls", 1.0);
        }

        // "rare" has no additional callers (only the edge to target counts as outbound, not inbound)

        // Trace callers of target
        let callers = trace_callers(&conn, "src/target.rs::target", 1).unwrap();
        assert_eq!(callers.len(), 3);

        // The heavily-called "popular" should appear first
        assert_eq!(callers[0].fqn, "src/popular.rs::popular");
        assert_eq!(callers[0].call_count, 5);

        // "medium" should appear second
        assert_eq!(callers[1].fqn, "src/medium.rs::medium");
        assert_eq!(callers[1].call_count, 2);

        // "rare" should appear last (0 inbound Calls edges)
        assert_eq!(callers[2].fqn, "src/rare.rs::rare");
        assert_eq!(callers[2].call_count, 0);
    }

    #[test]
    fn test_trace_callees_ranked_by_call_count() {
        let conn = setup_db();

        // Create a source node and several callees with different inbound edge counts
        insert_node(&conn, "src/source.rs::source", "Function", "src/source.rs", 1);
        insert_node(&conn, "src/hot.rs::hot_func", "Function", "src/hot.rs", 1);
        insert_node(&conn, "src/cold.rs::cold_func", "Function", "src/cold.rs", 1);

        // source calls both hot_func and cold_func
        insert_edge(&conn, "src/source.rs::source", "src/hot.rs::hot_func", "Calls", 1.0);
        insert_edge(&conn, "src/source.rs::source", "src/cold.rs::cold_func", "Calls", 1.0);

        // Make hot_func heavily called by many other nodes (4 additional callers)
        for i in 0..4 {
            let caller_fqn = format!("src/user{}.rs::user{}", i, i);
            insert_node(&conn, &caller_fqn, "Function", &format!("src/user{}.rs", i), 1);
            insert_edge(&conn, &caller_fqn, "src/hot.rs::hot_func", "Calls", 1.0);
        }

        // cold_func has only 1 inbound edge (from source)

        let callees = trace_callees(&conn, "src/source.rs::source", 1).unwrap();
        assert_eq!(callees.len(), 2);

        // hot_func (5 inbound edges: source + 4 users) should appear first
        assert_eq!(callees[0].fqn, "src/hot.rs::hot_func");
        assert_eq!(callees[0].call_count, 5);

        // cold_func (1 inbound edge: source) should appear second
        assert_eq!(callees[1].fqn, "src/cold.rs::cold_func");
        assert_eq!(callees[1].call_count, 1);
    }

    #[test]
    fn test_call_path_node_includes_call_count() {
        let conn = setup_db();
        setup_call_chain(&conn);

        // trace_callers should include call_count field
        let callers = trace_callers(&conn, "src/b.rs::func_b", 1).unwrap();
        assert_eq!(callers.len(), 1);
        // func_a has 0 inbound Calls edges in this setup
        assert_eq!(callers[0].call_count, 0);

        // func_b has 1 inbound Calls edge (from func_a)
        let callers = trace_callers(&conn, "src/c.rs::func_c", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].fqn, "src/b.rs::func_b");
        assert_eq!(callers[0].call_count, 1);
    }
}
