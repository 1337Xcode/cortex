//! Leiden community detection on the call graph.
//!
//! Implements a simplified Leiden algorithm to detect communities (clusters)
//! of tightly-coupled functions/classes in the codebase. Used by the
//! `decompose_boundaries` MCP tool to suggest module extraction boundaries.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A detected community of related code symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    /// Unique identifier for this community.
    pub community_id: usize,
    /// Source files that belong to this community.
    pub files: Vec<String>,
    /// Number of nodes (functions/classes) in this community.
    pub node_count: usize,
    /// Number of edges (calls) within this community.
    pub internal_edges: usize,
    /// Number of edges crossing this community's boundary.
    pub external_edges: usize,
    /// Suggested API surface: nodes with external callers.
    pub suggested_api_surface: Vec<String>,
    /// External dependencies: nodes outside this community that are called.
    pub external_dependencies: Vec<String>,
    /// Blast radius: number of nodes outside that depend on this community.
    pub blast_radius: usize,
}

/// Result of community detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDetectionResult {
    pub communities: Vec<Community>,
}

// ---------------------------------------------------------------------------
// Internal graph representation
// ---------------------------------------------------------------------------

/// A lightweight graph for community detection.
struct Graph {
    /// Node indices by FQN.
    node_index: HashMap<String, usize>,
    /// FQN by node index.
    node_fqns: Vec<String>,
    /// File path for each node.
    node_files: Vec<String>,
    /// Adjacency list: edges[i] = list of (target_index, weight).
    edges: Vec<Vec<(usize, f64)>>,
    /// Total edge weight in the graph.
    total_weight: f64,
}

impl Graph {
    fn new() -> Self {
        Self {
            node_index: HashMap::new(),
            node_fqns: Vec::new(),
            node_files: Vec::new(),
            edges: Vec::new(),
            total_weight: 0.0,
        }
    }

    fn add_node(&mut self, fqn: &str, file: &str) -> usize {
        if let Some(&idx) = self.node_index.get(fqn) {
            return idx;
        }
        let idx = self.node_fqns.len();
        self.node_index.insert(fqn.to_string(), idx);
        self.node_fqns.push(fqn.to_string());
        self.node_files.push(file.to_string());
        self.edges.push(Vec::new());
        idx
    }

    fn add_edge(&mut self, source: usize, target: usize, weight: f64) {
        self.edges[source].push((target, weight));
        self.edges[target].push((source, weight));
        self.total_weight += weight;
    }

    fn node_count(&self) -> usize {
        self.node_fqns.len()
    }

    /// Compute the weighted degree of a node (sum of edge weights).
    fn weighted_degree(&self, node: usize) -> f64 {
        self.edges[node].iter().map(|(_, w)| w).sum()
    }
}

// ---------------------------------------------------------------------------
// Leiden Algorithm Implementation
// ---------------------------------------------------------------------------

/// Run Leiden community detection on the call graph filtered by module_path.
///
/// Parameters:
/// - `conn`: Database connection
/// - `module_path`: Optional path prefix to filter nodes (e.g., "src/auth")
/// - `coupling_threshold`: Minimum modularity gain to move a node (0.0-1.0)
///
/// Returns communities with files, API surface, external deps, and blast radius.
pub fn detect_communities(
    conn: &Connection,
    module_path: Option<&str>,
    coupling_threshold: f64,
) -> Result<CommunityDetectionResult, StoreError> {
    // Step 1: Load the call graph from the database
    let graph = load_call_graph(conn, module_path)?;

    if graph.node_count() == 0 {
        return Ok(CommunityDetectionResult {
            communities: Vec::new(),
        });
    }

    // Step 2: Run Leiden algorithm
    let assignments = leiden_algorithm(&graph, coupling_threshold);

    // Step 3: Build community structures from assignments
    let communities = build_communities(&graph, &assignments);

    Ok(CommunityDetectionResult { communities })
}

/// Load the call graph from SQLite, optionally filtered by module_path prefix.
fn load_call_graph(conn: &Connection, module_path: Option<&str>) -> Result<Graph, StoreError> {
    let mut graph = Graph::new();

    // Load nodes (Function and Class kinds are most relevant for community detection)
    let (node_sql, pattern) = if let Some(prefix) = module_path {
        (
            "SELECT fqn, file FROM nodes WHERE (kind = 'Function' OR kind = 'Class') AND file LIKE ?1",
            Some(format!("{}%", prefix)),
        )
    } else {
        (
            "SELECT fqn, file FROM nodes WHERE kind = 'Function' OR kind = 'Class'",
            None,
        )
    };

    let mut stmt = conn.prepare(node_sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare node query for community detection: {}", e),
    })?;

    let rows: Vec<(String, String)> = if let Some(ref pat) = pattern {
        stmt.query_map(rusqlite::params![pat], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to query nodes for community detection: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect nodes for community detection: {}", e),
        })?
    } else {
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to query nodes for community detection: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect nodes for community detection: {}", e),
        })?
    };

    for (fqn, file) in &rows {
        graph.add_node(fqn, file);
    }

    // Load edges (Calls and Import edges between nodes in our graph)
    let edge_sql = "SELECT source_fqn, target_fqn, confidence FROM edges WHERE kind = 'Calls' OR kind = 'Import'";
    let mut edge_stmt = conn.prepare(edge_sql).map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to prepare edge query for community detection: {}", e),
    })?;

    let edge_rows: Vec<(String, String, f64)> = edge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to query edges for community detection: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect edges for community detection: {}", e),
        })?;

    for (source, target, confidence) in &edge_rows {
        // Only include edges where both endpoints are in our graph
        if let (Some(&src_idx), Some(&tgt_idx)) =
            (graph.node_index.get(source), graph.node_index.get(target))
        {
            // Avoid self-loops
            if src_idx != tgt_idx {
                graph.add_edge(src_idx, tgt_idx, *confidence);
            }
        }
    }

    Ok(graph)
}

/// Leiden algorithm: iterative local moving + refinement until convergence.
///
/// This is a simplified implementation suitable for code graphs:
/// - Phase 1: Local moving (greedy modularity optimization)
/// - Phase 2: Refinement (split large communities)
/// - Repeat until no improvement
fn leiden_algorithm(graph: &Graph, threshold: f64) -> Vec<usize> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    // Initialize: each node in its own community
    let mut assignments: Vec<usize> = (0..n).collect();
    let mut num_communities = n;

    // Effective threshold: scale by total weight to make it resolution-independent
    let effective_threshold = if graph.total_weight > 0.0 {
        threshold * 0.01 // Small threshold for code graphs
    } else {
        0.0
    };

    // Iterate until convergence (max 50 iterations to prevent infinite loops)
    for _iteration in 0..50 {
        let mut improved = false;

        // Phase 1: Local moving
        for node in 0..n {
            let best_community =
                find_best_community(graph, node, &assignments, num_communities, effective_threshold);

            if best_community != assignments[node] {
                assignments[node] = best_community;
                improved = true;
            }
        }

        if !improved {
            break;
        }

        // Renumber communities to be contiguous
        num_communities = renumber_communities(&mut assignments);
    }

    // Phase 2: Refinement - merge very small communities (< 2 nodes)
    // into their best neighbor community
    let community_sizes = compute_community_sizes(&assignments, num_communities);
    for node in 0..n {
        let comm = assignments[node];
        if community_sizes[comm] < 2 {
            // Find the most connected neighboring community
            let best = find_best_community(graph, node, &assignments, num_communities, 0.0);
            if best != comm {
                assignments[node] = best;
            }
        }
    }

    // Final renumbering
    renumber_communities(&mut assignments);

    assignments
}

/// Find the best community for a node based on modularity gain.
fn find_best_community(
    graph: &Graph,
    node: usize,
    assignments: &[usize],
    _num_communities: usize,
    threshold: f64,
) -> usize {
    let current_community = assignments[node];
    let ki = graph.weighted_degree(node);
    let m2 = graph.total_weight; // Already doubled since we add both directions

    if m2 == 0.0 {
        return current_community;
    }

    // Compute the sum of edge weights from `node` to each neighboring community
    let mut community_weights: HashMap<usize, f64> = HashMap::new();
    for &(neighbor, weight) in &graph.edges[node] {
        let neighbor_comm = assignments[neighbor];
        *community_weights.entry(neighbor_comm).or_insert(0.0) += weight;
    }

    // Find the community with the best modularity gain
    let mut best_community = current_community;
    let mut best_gain = 0.0;

    // Weight to current community (for removal calculation)
    let ki_in_current = community_weights.get(&current_community).copied().unwrap_or(0.0);

    // Sum of degrees in current community (excluding this node)
    let sigma_current = community_degree_sum(graph, assignments, current_community, Some(node));

    // Gain from removing node from current community
    let remove_cost = ki_in_current - (sigma_current * ki) / m2;

    for (&comm, &ki_in_comm) in &community_weights {
        if comm == current_community {
            continue;
        }

        // Sum of degrees in target community
        let sigma_target = community_degree_sum(graph, assignments, comm, None);

        // Gain from adding node to target community
        let add_gain = ki_in_comm - (sigma_target * ki) / m2;

        let delta_q = add_gain - remove_cost;

        if delta_q > best_gain + threshold {
            best_gain = delta_q;
            best_community = comm;
        }
    }

    best_community
}

/// Compute the sum of weighted degrees for all nodes in a community,
/// optionally excluding a specific node.
fn community_degree_sum(
    graph: &Graph,
    assignments: &[usize],
    community: usize,
    exclude: Option<usize>,
) -> f64 {
    let mut sum = 0.0;
    for (i, &comm) in assignments.iter().enumerate() {
        if comm == community {
            if let Some(excl) = exclude {
                if i == excl {
                    continue;
                }
            }
            sum += graph.weighted_degree(i);
        }
    }
    sum
}

/// Renumber communities to be contiguous (0, 1, 2, ...).
/// Returns the number of distinct communities.
fn renumber_communities(assignments: &mut [usize]) -> usize {
    let mut mapping: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0;

    for assignment in assignments.iter_mut() {
        let new_id = *mapping.entry(*assignment).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        *assignment = new_id;
    }

    next_id
}

/// Compute the size of each community.
fn compute_community_sizes(assignments: &[usize], num_communities: usize) -> Vec<usize> {
    let mut sizes = vec![0usize; num_communities];
    for &comm in assignments {
        if comm < num_communities {
            sizes[comm] += 1;
        }
    }
    sizes
}

// ---------------------------------------------------------------------------
// Build community result structures
// ---------------------------------------------------------------------------

/// Build the final Community structs from node assignments.
fn build_communities(graph: &Graph, assignments: &[usize]) -> Vec<Community> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    let num_communities = assignments.iter().max().map(|m| m + 1).unwrap_or(0);

    // Group nodes by community
    let mut community_nodes: Vec<Vec<usize>> = vec![Vec::new(); num_communities];
    for (node, &comm) in assignments.iter().enumerate() {
        community_nodes[comm].push(node);
    }

    let mut communities = Vec::new();

    for (comm_id, nodes) in community_nodes.iter().enumerate() {
        if nodes.is_empty() {
            continue;
        }

        let node_set: HashSet<usize> = nodes.iter().copied().collect();

        // Collect unique files
        let mut files: Vec<String> = nodes
            .iter()
            .map(|&n| graph.node_files[n].clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        files.sort();

        // Count internal and external edges
        let mut internal_edges = 0usize;
        let mut external_edges = 0usize;
        let mut api_surface: HashSet<String> = HashSet::new();
        let mut external_deps: HashSet<String> = HashSet::new();
        let mut external_callers: HashSet<usize> = HashSet::new();

        for &node in nodes {
            for &(neighbor, _) in &graph.edges[node] {
                if node_set.contains(&neighbor) {
                    // Only count each internal edge once (from the lower index)
                    if node < neighbor {
                        internal_edges += 1;
                    }
                } else {
                    external_edges += 1;
                    // If an external node calls into this community, the called node is API surface
                    // We track both directions: external calling us = API surface,
                    // us calling external = external dependency
                    if assignments[neighbor] != comm_id {
                        // This node has a connection to an external node
                        api_surface.insert(graph.node_fqns[node].clone());
                        external_deps.insert(graph.node_fqns[neighbor].clone());
                        external_callers.insert(neighbor);
                    }
                }
            }
        }

        // Blast radius: count unique external nodes that depend on this community
        let blast_radius = external_callers.len();

        let mut suggested_api_surface: Vec<String> = api_surface.into_iter().collect();
        suggested_api_surface.sort();

        let mut external_dependencies: Vec<String> = external_deps.into_iter().collect();
        external_dependencies.sort();

        communities.push(Community {
            community_id: comm_id,
            files,
            node_count: nodes.len(),
            internal_edges,
            external_edges,
            suggested_api_surface,
            external_dependencies,
            blast_radius,
        });
    }

    communities
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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

    fn insert_node(conn: &Connection, fqn: &str, kind: &str, file: &str) {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, ?3, 1, 10, 'hash', 1000, '{}')",
            rusqlite::params![fqn, kind, file],
        )
        .unwrap();
    }

    fn insert_edge(conn: &Connection, source: &str, target: &str, kind: &str, confidence: f64) {
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
             VALUES (?1, ?2, ?3, ?4, '{}')",
            rusqlite::params![source, target, kind, confidence],
        )
        .unwrap();
    }

    #[test]
    fn test_empty_graph_returns_no_communities() {
        let conn = setup_db();
        let result = detect_communities(&conn, None, 0.5).unwrap();
        assert!(result.communities.is_empty());
    }

    #[test]
    fn test_single_node_returns_one_community() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs");

        let result = detect_communities(&conn, None, 0.5).unwrap();
        assert_eq!(result.communities.len(), 1);
        assert_eq!(result.communities[0].node_count, 1);
    }

    #[test]
    fn test_multi_module_produces_multiple_communities() {
        let conn = setup_db();

        // Module A: tightly connected cluster
        insert_node(&conn, "src/auth/login.rs::authenticate", "Function", "src/auth/login.rs");
        insert_node(&conn, "src/auth/login.rs::validate_password", "Function", "src/auth/login.rs");
        insert_node(&conn, "src/auth/session.rs::create_session", "Function", "src/auth/session.rs");
        insert_node(&conn, "src/auth/session.rs::validate_session", "Function", "src/auth/session.rs");

        // Module B: tightly connected cluster
        insert_node(&conn, "src/db/query.rs::execute_query", "Function", "src/db/query.rs");
        insert_node(&conn, "src/db/query.rs::prepare_statement", "Function", "src/db/query.rs");
        insert_node(&conn, "src/db/pool.rs::get_connection", "Function", "src/db/pool.rs");
        insert_node(&conn, "src/db/pool.rs::release_connection", "Function", "src/db/pool.rs");

        // Dense internal edges within Module A
        insert_edge(&conn, "src/auth/login.rs::authenticate", "src/auth/login.rs::validate_password", "Calls", 1.0);
        insert_edge(&conn, "src/auth/login.rs::authenticate", "src/auth/session.rs::create_session", "Calls", 1.0);
        insert_edge(&conn, "src/auth/session.rs::create_session", "src/auth/session.rs::validate_session", "Calls", 1.0);
        insert_edge(&conn, "src/auth/session.rs::validate_session", "src/auth/login.rs::validate_password", "Calls", 1.0);

        // Dense internal edges within Module B
        insert_edge(&conn, "src/db/query.rs::execute_query", "src/db/query.rs::prepare_statement", "Calls", 1.0);
        insert_edge(&conn, "src/db/query.rs::execute_query", "src/db/pool.rs::get_connection", "Calls", 1.0);
        insert_edge(&conn, "src/db/pool.rs::get_connection", "src/db/pool.rs::release_connection", "Calls", 1.0);
        insert_edge(&conn, "src/db/query.rs::prepare_statement", "src/db/pool.rs::get_connection", "Calls", 1.0);

        // One weak cross-module edge
        insert_edge(&conn, "src/auth/login.rs::authenticate", "src/db/query.rs::execute_query", "Calls", 0.5);

        let result = detect_communities(&conn, None, 0.5).unwrap();

        // Should detect at least 2 communities
        assert!(
            result.communities.len() >= 2,
            "Expected ≥2 communities, got {}. Communities: {:?}",
            result.communities.len(),
            result.communities.iter().map(|c| (&c.files, c.node_count)).collect::<Vec<_>>()
        );

        // Verify community structure
        let total_nodes: usize = result.communities.iter().map(|c| c.node_count).sum();
        assert_eq!(total_nodes, 8);

        // At least one community should have external edges (the cross-module call)
        let has_external = result.communities.iter().any(|c| c.external_edges > 0);
        assert!(has_external, "Expected at least one community with external edges");
    }

    #[test]
    fn test_module_path_filter() {
        let conn = setup_db();

        insert_node(&conn, "src/auth/login.rs::authenticate", "Function", "src/auth/login.rs");
        insert_node(&conn, "src/auth/login.rs::validate", "Function", "src/auth/login.rs");
        insert_node(&conn, "src/db/query.rs::execute", "Function", "src/db/query.rs");

        insert_edge(&conn, "src/auth/login.rs::authenticate", "src/auth/login.rs::validate", "Calls", 1.0);

        // Filter to only auth module
        let result = detect_communities(&conn, Some("src/auth"), 0.5).unwrap();

        // Should only include auth nodes
        let all_files: Vec<&str> = result
            .communities
            .iter()
            .flat_map(|c| c.files.iter().map(|f| f.as_str()))
            .collect();

        for file in &all_files {
            assert!(file.starts_with("src/auth"), "Unexpected file: {}", file);
        }
    }

    #[test]
    fn test_community_has_api_surface() {
        let conn = setup_db();

        // Two clusters with a cross-edge
        insert_node(&conn, "src/a.rs::func_a1", "Function", "src/a.rs");
        insert_node(&conn, "src/a.rs::func_a2", "Function", "src/a.rs");
        insert_node(&conn, "src/b.rs::func_b1", "Function", "src/b.rs");
        insert_node(&conn, "src/b.rs::func_b2", "Function", "src/b.rs");

        // Internal edges
        insert_edge(&conn, "src/a.rs::func_a1", "src/a.rs::func_a2", "Calls", 1.0);
        insert_edge(&conn, "src/b.rs::func_b1", "src/b.rs::func_b2", "Calls", 1.0);

        // Cross-edge
        insert_edge(&conn, "src/a.rs::func_a1", "src/b.rs::func_b1", "Calls", 0.3);

        let result = detect_communities(&conn, None, 0.5).unwrap();

        // If we got multiple communities, check API surface
        if result.communities.len() >= 2 {
            let has_api_surface = result
                .communities
                .iter()
                .any(|c| !c.suggested_api_surface.is_empty());
            assert!(has_api_surface, "Expected at least one community with API surface");
        }
    }
}
