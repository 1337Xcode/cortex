//! Ego-graph query with BFS traversal and node capping.
//!
//! Builds a subgraph centered on a given node by performing BFS outward
//! (following both inbound and outbound edges). Results are capped at
//! `EGO_GRAPH_MAX_NODES` with priority sorting: depth ascending, then
//! caller_count descending.

use std::collections::{HashMap, HashSet, VecDeque};

use rusqlite::Connection;
use serde::Serialize;

use crate::error::StoreError;
use crate::store::types::{Edge, EdgeKind, Node, NodeKind};

/// Maximum number of nodes returned in an ego-graph result.
pub const EGO_GRAPH_MAX_NODES: usize = 500;

/// Result of an ego-graph query.
#[derive(Debug, Clone, Serialize)]
pub struct EgoGraphResult {
    /// Nodes in the subgraph (at most `EGO_GRAPH_MAX_NODES`).
    pub nodes: Vec<EgoNode>,
    /// Edges between the selected nodes.
    pub edges: Vec<Edge>,
    /// Whether the result was truncated due to the node cap.
    pub truncated: bool,
    /// Total number of reachable nodes before capping.
    pub total_reachable: usize,
}

/// A node in the ego-graph with traversal metadata.
#[derive(Debug, Clone, Serialize)]
pub struct EgoNode {
    /// The underlying graph node (flattened in JSON output).
    #[serde(flatten)]
    pub node: Node,
    /// BFS depth from the center node.
    pub depth: u32,
    /// Number of inbound "Calls" edges to this node.
    pub caller_count: u32,
}

// ---------------------------------------------------------------------------
// Helper: parse NodeKind from database string
// ---------------------------------------------------------------------------

fn parse_node_kind(s: &str) -> NodeKind {
    match s {
        "Function" => NodeKind::Function,
        "Method" => NodeKind::Method,
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

fn parse_edge_kind(s: &str) -> EdgeKind {
    match s {
        "Calls" => EdgeKind::Calls,
        "Imports" => EdgeKind::Imports,
        "Inherits" => EdgeKind::Inherits,
        "Implements" => EdgeKind::Implements,
        "HttpLink" => EdgeKind::HttpLink,
        "DataFlow" => EdgeKind::DataFlow,
        _ => EdgeKind::Calls, // fallback
    }
}

// ---------------------------------------------------------------------------
// Main ego-graph query
// ---------------------------------------------------------------------------

/// Build an ego-graph centered on `center_fqn` using BFS traversal.
///
/// Traverses both inbound and outbound edges up to `max_depth` hops.
/// Results are sorted by (depth ASC, caller_count DESC) and capped at
/// `EGO_GRAPH_MAX_NODES`. Edges are filtered to only include those
/// between selected nodes.
///
/// Returns an empty result with `truncated = false` if the center node
/// does not exist.
pub fn ego_graph(
    conn: &Connection,
    center_fqn: &str,
    max_depth: u32,
) -> Result<EgoGraphResult, StoreError> {
    // Check if center node exists
    let center_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE fqn = ?1",
            rusqlite::params![center_fqn],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to check center node existence: {}", e),
        })?;

    if !center_exists {
        return Ok(EgoGraphResult {
            nodes: Vec::new(),
            edges: Vec::new(),
            truncated: false,
            total_reachable: 0,
        });
    }

    // BFS traversal collecting all reachable nodes with their depth
    let mut visited: HashMap<String, u32> = HashMap::new(); // fqn -> depth
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    visited.insert(center_fqn.to_string(), 0);
    queue.push_back((center_fqn.to_string(), 0));

    while let Some((current_fqn, current_depth)) = queue.pop_front() {
        if current_depth >= max_depth {
            continue;
        }

        let next_depth = current_depth + 1;

        // Get neighbors via outbound edges (current -> target)
        let outbound = get_neighbors_outbound(conn, &current_fqn)?;
        for neighbor_fqn in outbound {
            if !visited.contains_key(&neighbor_fqn) {
                visited.insert(neighbor_fqn.clone(), next_depth);
                queue.push_back((neighbor_fqn, next_depth));
            }
        }

        // Get neighbors via inbound edges (source -> current)
        let inbound = get_neighbors_inbound(conn, &current_fqn)?;
        for neighbor_fqn in inbound {
            if !visited.contains_key(&neighbor_fqn) {
                visited.insert(neighbor_fqn.clone(), next_depth);
                queue.push_back((neighbor_fqn, next_depth));
            }
        }
    }

    let total_reachable = visited.len();

    // Get caller counts for all visited nodes
    let caller_counts = get_caller_counts(conn, &visited.keys().cloned().collect::<Vec<_>>())?;

    // Build EgoNode list with depth and caller_count
    let mut ego_nodes: Vec<EgoNode> = Vec::with_capacity(total_reachable);
    for (fqn, depth) in &visited {
        if let Some(node) = load_node(conn, fqn)? {
            let caller_count = caller_counts.get(fqn).copied().unwrap_or(0);
            ego_nodes.push(EgoNode {
                node,
                depth: *depth,
                caller_count,
            });
        }
    }

    // Sort by (depth ASC, caller_count DESC)
    ego_nodes.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| b.caller_count.cmp(&a.caller_count))
    });

    // Cap at EGO_GRAPH_MAX_NODES
    let truncated = ego_nodes.len() > EGO_GRAPH_MAX_NODES;
    ego_nodes.truncate(EGO_GRAPH_MAX_NODES);

    // Collect selected FQNs for edge filtering
    let selected_fqns: HashSet<&str> = ego_nodes.iter().map(|n| n.node.fqn.as_str()).collect();

    // Filter edges to only those between selected nodes
    let edges = get_edges_between(conn, &selected_fqns)?;

    Ok(EgoGraphResult {
        nodes: ego_nodes,
        edges,
        truncated,
        total_reachable,
    })
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Get FQNs of nodes reachable via outbound edges from `fqn`.
fn get_neighbors_outbound(conn: &Connection, fqn: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT target_fqn FROM edges WHERE source_fqn = ?1")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare outbound neighbors query: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![fqn], |row| row.get::<_, String>(0))
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute outbound neighbors query: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect outbound neighbors: {}", e),
        })?;

    Ok(results)
}

/// Get FQNs of nodes reachable via inbound edges to `fqn`.
fn get_neighbors_inbound(conn: &Connection, fqn: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT source_fqn FROM edges WHERE target_fqn = ?1")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare inbound neighbors query: {}", e),
        })?;

    let results = stmt
        .query_map(rusqlite::params![fqn], |row| row.get::<_, String>(0))
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute inbound neighbors query: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect inbound neighbors: {}", e),
        })?;

    Ok(results)
}

/// Get caller counts (inbound "Calls" edges) for a set of FQNs.
fn get_caller_counts(
    conn: &Connection,
    fqns: &[String],
) -> Result<HashMap<String, u32>, StoreError> {
    let mut counts: HashMap<String, u32> = HashMap::new();

    if fqns.is_empty() {
        return Ok(counts);
    }

    let mut stmt = conn
        .prepare(
            "SELECT target_fqn, COUNT(*) FROM edges \
             WHERE kind = 'Calls' GROUP BY target_fqn",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare caller counts query: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((fqn, count as u32))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute caller counts query: {}", e),
        })?;

    let fqn_set: HashSet<&str> = fqns.iter().map(|s| s.as_str()).collect();
    for row in rows {
        let (fqn, count) = row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read caller count row: {}", e),
        })?;
        if fqn_set.contains(fqn.as_str()) {
            counts.insert(fqn, count);
        }
    }

    Ok(counts)
}

/// Load a single node by FQN.
fn load_node(conn: &Connection, fqn: &str) -> Result<Option<Node>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes \
             FROM nodes WHERE fqn = ?1",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare load_node: {}", e),
        })?;

    let result = stmt
        .query_row(rusqlite::params![fqn], |row| {
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
        })
        .optional()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute load_node: {}", e),
        })?;

    Ok(result)
}

/// Get all edges where both source and target are in the selected set.
fn get_edges_between(
    conn: &Connection,
    selected_fqns: &HashSet<&str>,
) -> Result<Vec<Edge>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT id, source_fqn, target_fqn, kind, confidence, attributes FROM edges")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare edges query: {}", e),
        })?;

    let all_edges = stmt
        .query_map([], |row| {
            let kind_str: String = row.get(3)?;
            let attributes_str: String = row.get(5)?;
            Ok(Edge {
                id: row.get(0)?,
                source_fqn: row.get(1)?,
                target_fqn: row.get(2)?,
                kind: parse_edge_kind(&kind_str),
                confidence: row.get(4)?,
                attributes: serde_json::from_str(&attributes_str).unwrap_or_default(),
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute edges query: {}", e),
        })?;

    let mut filtered_edges = Vec::new();
    for edge_result in all_edges {
        let edge = edge_result.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read edge row: {}", e),
        })?;
        if selected_fqns.contains(edge.source_fqn.as_str())
            && selected_fqns.contains(edge.target_fqn.as_str())
        {
            filtered_edges.push(edge);
        }
    }

    Ok(filtered_edges)
}

// ---------------------------------------------------------------------------
// Extension trait for optional query results
// ---------------------------------------------------------------------------

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
    use proptest::prelude::*;
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
    fn test_nonexistent_center_returns_empty() {
        let conn = setup_db();
        let result = ego_graph(&conn, "nonexistent::node", 3).unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert!(!result.truncated);
        assert_eq!(result.total_reachable, 0);
    }

    #[test]
    fn test_single_node_no_edges() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1);

        let result = ego_graph(&conn, "src/a.rs::func_a", 3).unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].node.fqn, "src/a.rs::func_a");
        assert_eq!(result.nodes[0].depth, 0);
        assert!(result.edges.is_empty());
        assert!(!result.truncated);
        assert_eq!(result.total_reachable, 1);
    }

    #[test]
    fn test_linear_chain_bfs() {
        let conn = setup_db();
        // A -> B -> C
        insert_node(&conn, "a::func_a", "Function", "a.rs", 1);
        insert_node(&conn, "b::func_b", "Function", "b.rs", 1);
        insert_node(&conn, "c::func_c", "Function", "c.rs", 1);
        insert_edge(&conn, "a::func_a", "b::func_b", "Calls", 1.0);
        insert_edge(&conn, "b::func_b", "c::func_c", "Calls", 1.0);

        // BFS from B with depth 1 should find A and C
        let result = ego_graph(&conn, "b::func_b", 1).unwrap();
        assert_eq!(result.total_reachable, 3); // B + A + C
        assert_eq!(result.nodes.len(), 3);
        assert!(!result.truncated);

        // Center node (depth 0) should be first
        assert_eq!(result.nodes[0].node.fqn, "b::func_b");
        assert_eq!(result.nodes[0].depth, 0);
    }

    #[test]
    fn test_depth_limiting() {
        let conn = setup_db();
        // A -> B -> C -> D
        insert_node(&conn, "a::f", "Function", "a.rs", 1);
        insert_node(&conn, "b::f", "Function", "b.rs", 1);
        insert_node(&conn, "c::f", "Function", "c.rs", 1);
        insert_node(&conn, "d::f", "Function", "d.rs", 1);
        insert_edge(&conn, "a::f", "b::f", "Calls", 1.0);
        insert_edge(&conn, "b::f", "c::f", "Calls", 1.0);
        insert_edge(&conn, "c::f", "d::f", "Calls", 1.0);

        // BFS from A with depth 1 should only find B
        let result = ego_graph(&conn, "a::f", 1).unwrap();
        assert_eq!(result.total_reachable, 2); // A + B
        assert_eq!(result.nodes.len(), 2);
    }

    #[test]
    fn test_priority_ordering() {
        let conn = setup_db();
        // Center -> A, Center -> B, Center -> C
        // A has 3 callers, B has 1 caller, C has 2 callers
        insert_node(&conn, "center::f", "Function", "center.rs", 1);
        insert_node(&conn, "a::f", "Function", "a.rs", 1);
        insert_node(&conn, "b::f", "Function", "b.rs", 1);
        insert_node(&conn, "c::f", "Function", "c.rs", 1);
        insert_node(&conn, "x1::f", "Function", "x1.rs", 1);
        insert_node(&conn, "x2::f", "Function", "x2.rs", 1);
        insert_node(&conn, "x3::f", "Function", "x3.rs", 1);

        insert_edge(&conn, "center::f", "a::f", "Calls", 1.0);
        insert_edge(&conn, "center::f", "b::f", "Calls", 1.0);
        insert_edge(&conn, "center::f", "c::f", "Calls", 1.0);

        // A has 3 callers (center + x1 + x2)
        insert_edge(&conn, "x1::f", "a::f", "Calls", 1.0);
        insert_edge(&conn, "x2::f", "a::f", "Calls", 1.0);

        // C has 2 callers (center + x3)
        insert_edge(&conn, "x3::f", "c::f", "Calls", 1.0);

        // B has 1 caller (center only)

        let result = ego_graph(&conn, "center::f", 1).unwrap();

        // Depth 0: center
        assert_eq!(result.nodes[0].node.fqn, "center::f");
        assert_eq!(result.nodes[0].depth, 0);

        // Depth 1: sorted by caller_count DESC -> A(3), C(2), B(1)
        let depth_1_nodes: Vec<&EgoNode> = result.nodes.iter().filter(|n| n.depth == 1).collect();
        assert_eq!(depth_1_nodes[0].node.fqn, "a::f");
        assert_eq!(depth_1_nodes[0].caller_count, 3);
        assert_eq!(depth_1_nodes[1].node.fqn, "c::f");
        assert_eq!(depth_1_nodes[1].caller_count, 2);
        assert_eq!(depth_1_nodes[2].node.fqn, "b::f");
        assert_eq!(depth_1_nodes[2].caller_count, 1);
    }

    #[test]
    fn test_edges_filtered_to_selected_nodes() {
        let conn = setup_db();
        insert_node(&conn, "a::f", "Function", "a.rs", 1);
        insert_node(&conn, "b::f", "Function", "b.rs", 1);
        insert_node(&conn, "c::f", "Function", "c.rs", 1);
        insert_edge(&conn, "a::f", "b::f", "Calls", 1.0);
        insert_edge(&conn, "b::f", "c::f", "Calls", 1.0);

        // BFS from A with depth 1: only A and B are selected
        let result = ego_graph(&conn, "a::f", 1).unwrap();
        assert_eq!(result.nodes.len(), 2);

        // Only the edge A->B should be included (not B->C since C is not selected)
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].source_fqn, "a::f");
        assert_eq!(result.edges[0].target_fqn, "b::f");
    }

    #[test]
    fn test_truncation_flag() {
        let conn = setup_db();

        // Create a star graph with center + 501 neighbors (exceeds cap)
        insert_node(&conn, "center::f", "Function", "center.rs", 1);
        for i in 0..501 {
            let fqn = format!("node_{}::f", i);
            insert_node(&conn, &fqn, "Function", &format!("n{}.rs", i), 1);
            insert_edge(&conn, "center::f", &fqn, "Calls", 1.0);
        }

        let result = ego_graph(&conn, "center::f", 1).unwrap();
        assert_eq!(result.total_reachable, 502); // center + 501 neighbors
        assert_eq!(result.nodes.len(), EGO_GRAPH_MAX_NODES);
        assert!(result.truncated);
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    // **Validates: Requirements 6.1, 6.3**

    proptest! {
        /// **Property 5: Ego-graph node cap invariant**
        ///
        /// For any graph and starting node, the ego-graph result contains at most
        /// 500 nodes, and `truncated == (total_reachable > 500)`.
        #[test]
        fn prop_ego_graph_node_cap_invariant(
            num_nodes in 1_usize..800,
            num_edges in 0_usize..1500,
            start_idx in 0_usize..800,
            max_depth in 1_u32..6,
        ) {
            // Clamp start_idx to valid range
            let start_idx = start_idx % num_nodes;

            let conn = setup_db();

            // Insert nodes
            let node_fqns: Vec<String> = (0..num_nodes)
                .map(|i| format!("mod_{}::func_{}", i / 10, i))
                .collect();

            for (i, fqn) in node_fqns.iter().enumerate() {
                insert_node(
                    &conn,
                    fqn,
                    "Function",
                    &format!("src/f{}.rs", i),
                    (i as u32) + 1,
                );
            }

            // Insert random edges (deterministic from proptest seed)
            for edge_idx in 0..num_edges {
                let src_idx = edge_idx % num_nodes;
                let tgt_idx = (edge_idx * 7 + 3) % num_nodes;
                if src_idx != tgt_idx {
                    // Ignore duplicate edge errors
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            &node_fqns[src_idx],
                            &node_fqns[tgt_idx],
                            "Calls",
                            1.0,
                            "{}"
                        ],
                    );
                }
            }

            let center_fqn = &node_fqns[start_idx];
            let result = ego_graph(&conn, center_fqn, max_depth).unwrap();

            // Property: result contains at most 500 nodes
            prop_assert!(
                result.nodes.len() <= EGO_GRAPH_MAX_NODES,
                "Expected at most {} nodes, got {}",
                EGO_GRAPH_MAX_NODES,
                result.nodes.len()
            );

            // Property: truncated == (total_reachable > 500)
            prop_assert_eq!(
                result.truncated,
                result.total_reachable > EGO_GRAPH_MAX_NODES,
                "truncated={} but total_reachable={} (cap={})",
                result.truncated,
                result.total_reachable,
                EGO_GRAPH_MAX_NODES
            );
        }
    }

    // **Validates: Requirements 6.2**

    proptest! {
        /// **Property 6: Ego-graph priority ordering**
        ///
        /// For any truncated result, all nodes at depth D appear before depth D+1;
        /// within same depth, ordered by caller_count DESC.
        #[test]
        fn prop_ego_graph_priority_ordering(
            depth_1_count in 250_u32..400,
            depth_2_count in 250_u32..400,
            // Extra callers per depth-1 node (varying caller_counts)
            extra_callers in proptest::collection::vec(0_u32..15, 250..400),
        ) {
            let conn = setup_db();

            // Insert center node
            insert_node(&conn, "center::f", "Function", "center.rs", 1);

            // Clamp depth_1_count to the length of extra_callers vec
            let d1_count = depth_1_count.min(extra_callers.len() as u32);

            // Insert depth-1 nodes connected to center
            for i in 0..d1_count {
                let fqn = format!("d1_{}::f", i);
                insert_node(&conn, &fqn, "Function", &format!("d1_{}.rs", i), 1);
                insert_edge(&conn, "center::f", &fqn, "Calls", 1.0);
            }

            // Insert depth-2 nodes connected to depth-1 nodes (round-robin)
            for i in 0..depth_2_count {
                let fqn = format!("d2_{}::f", i);
                let parent_idx = i % d1_count;
                let parent_fqn = format!("d1_{}::f", parent_idx);
                insert_node(&conn, &fqn, "Function", &format!("d2_{}.rs", i), 1);
                insert_edge(&conn, &parent_fqn, &fqn, "Calls", 1.0);
            }

            // Add extra caller nodes to give depth-1 nodes varying caller_counts
            let mut caller_id = 0_u32;
            for i in 0..d1_count {
                let target_fqn = format!("d1_{}::f", i);
                let num_extra = extra_callers[i as usize];
                for _ in 0..num_extra {
                    let caller_fqn = format!("xc_{}::f", caller_id);
                    insert_node(
                        &conn,
                        &caller_fqn,
                        "Function",
                        &format!("xc_{}.rs", caller_id),
                        1,
                    );
                    insert_edge(&conn, &caller_fqn, &target_fqn, "Calls", 1.0);
                    caller_id += 1;
                }
            }

            // Run ego_graph with depth 3 to reach depth-2 nodes
            let result = ego_graph(&conn, "center::f", 3).unwrap();

            // Only verify ordering when result is truncated (large enough graph)
            if result.truncated {
                // Property: for consecutive nodes, either depth increases or
                // (depth is same AND caller_count is non-increasing)
                for window in result.nodes.windows(2) {
                    let prev = &window[0];
                    let curr = &window[1];

                    prop_assert!(
                        curr.depth > prev.depth
                            || (curr.depth == prev.depth
                                && curr.caller_count <= prev.caller_count),
                        "Ordering violated: node '{}' at depth={} caller_count={} \
                         follows node '{}' at depth={} caller_count={}",
                        curr.node.fqn,
                        curr.depth,
                        curr.caller_count,
                        prev.node.fqn,
                        prev.depth,
                        prev.caller_count,
                    );
                }
            }
        }
    }
}
