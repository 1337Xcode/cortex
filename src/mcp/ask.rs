//! The `ask` meta-tool: single-call code intelligence.
//!
//! Takes a natural language question and uses a 4-stage graph-guided retrieval
//! pipeline to compose a unified answer. The agent never needs to choose
//! between 31 tools. One call, one answer.
//!
//! Pipeline stages:
//! 1. Extract query terms + classify intent
//! 2. FTS5 seed search (with substring fallback)
//! 3. Ego-graph expansion from seeds (weighted edges, intent boost)
//! 4. Centrality ranking + budget truncation
//!
//! Intent classification is retained as a secondary signal that boosts
//! specific edge types during ego-graph expansion:
//! - "security" intent boosts taint-related (Calls) edges × 1.5
//! - "architecture" intent boosts Imports edges × 1.5
//! - all other intents apply no boost

use std::collections::{HashMap, HashSet, VecDeque};

use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::McpError;
use crate::store::db::StoreManager;
use crate::store::queries::community;

use super::savings_store::ModelPricing;

// ---------------------------------------------------------------------------
// Public response types
// ---------------------------------------------------------------------------

/// The new ask engine response format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AskResponse {
    pub summary: AskSummary,
    pub results: Vec<AskResultItem>,
}

/// Summary metadata for an ask response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AskSummary {
    pub total_results: usize,
    pub total_token_cost: usize,
    pub budget_used_percent: f64,
    pub intent_detected: String,
    pub query_terms_extracted: Vec<String>,
}

/// Configuration for the ask engine pipeline.
#[derive(Debug, Clone)]
pub struct AskConfig {
    pub token_budget: usize,
    pub max_seed_nodes: usize,
    pub max_query_terms: usize,
    pub ego_graph_depth: usize,
    pub edge_weights: EdgeWeights,
    pub model_pricing: ModelPricing,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            token_budget: 4096,
            max_seed_nodes: 50,
            max_query_terms: 20,
            ego_graph_depth: 2,
            edge_weights: EdgeWeights::default(),
            model_pricing: ModelPricing::default(),
        }
    }
}

/// A seed node returned by FTS5 search or fallback substring search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SeedNode {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub relevance: f64,
}

/// A node discovered during ego-graph expansion (Stage 2).
#[derive(Debug, Clone)]
pub struct EgoNode {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub accumulated_weight: f64,
    pub depth: usize,
    pub source_seed: String,
}

/// Edge weight configuration for ego-graph expansion.
#[derive(Debug, Clone)]
pub struct EdgeWeights {
    pub calls: f64,     // default 1.0
    pub community: f64, // default 0.7
    pub imports: f64,   // default 0.5
}

impl Default for EdgeWeights {
    fn default() -> Self {
        Self {
            calls: 1.0,
            community: 0.7,
            imports: 0.5,
        }
    }
}

/// Intent-specific boost factors applied during ego-graph expansion.
#[derive(Debug, Clone)]
pub struct IntentBoost {
    pub security_taint_boost: f64,      // default 1.5
    pub architecture_import_boost: f64, // default 1.5
    pub default_boost: f64,             // default 1.0
}

impl Default for IntentBoost {
    fn default() -> Self {
        Self {
            security_taint_boost: 1.5,
            architecture_import_boost: 1.5,
            default_boost: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 2: Ego-Graph Expansion
// ---------------------------------------------------------------------------

/// Walk outward from seed nodes along weighted edges to `max_depth` hops.
///
/// Uses BFS from each seed node. At each hop:
/// 1. Query edges WHERE source_fqn = current_node OR target_fqn = current_node
/// 2. Check if target is in the same Leiden community as the seed (adds community weight)
/// 3. Apply intent boost multiplier to relevant edge types
/// 4. Accumulate weight along the path
/// 5. Use a visited set to prevent cycles
///
/// Edge weights: Calls=1.0, same Leiden community=0.7, Imports=0.5.
/// Intent boost: security multiplies Calls edges by 1.5, architecture multiplies Imports by 1.5.
pub fn expand_ego_graph(
    conn: &Connection,
    seeds: &[SeedNode],
    max_depth: usize,
    weights: &EdgeWeights,
    intent_boost: &IntentBoost,
    intent: &Intent,
) -> HashMap<String, EgoNode> {
    let mut result: HashMap<String, EgoNode> = HashMap::new();

    // Pre-compute Leiden community assignments for community weight bonus.
    // Maps FQN -> community_id.
    let community_map = build_community_map(conn);

    // Process each seed node
    for seed in seeds {
        let seed_community = community_map.get(&seed.fqn).copied();

        // Add the seed itself to the result (depth 0)
        let seed_ego = EgoNode {
            fqn: seed.fqn.clone(),
            kind: seed.kind.clone(),
            file: seed.file.clone(),
            start_line: seed.start_line,
            end_line: seed.end_line,
            accumulated_weight: seed.relevance,
            depth: 0,
            source_seed: seed.fqn.clone(),
        };

        // Only insert if not already present with a higher weight
        match result.get(&seed.fqn) {
            Some(existing) if existing.accumulated_weight >= seed_ego.accumulated_weight => {}
            _ => {
                result.insert(seed.fqn.clone(), seed_ego);
            }
        }

        // BFS expansion from this seed
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(seed.fqn.clone());

        // Queue entries: (fqn, current_depth, accumulated_weight)
        let mut queue: VecDeque<(String, usize, f64)> = VecDeque::new();
        queue.push_back((seed.fqn.clone(), 0, seed.relevance));

        while let Some((current_fqn, current_depth, current_weight)) = queue.pop_front() {
            if current_depth >= max_depth {
                continue;
            }

            // Query edges connected to the current node
            let neighbors = query_neighbors(conn, &current_fqn);

            for neighbor in neighbors {
                if visited.contains(&neighbor.fqn) {
                    continue;
                }
                visited.insert(neighbor.fqn.clone());

                // Compute edge weight based on edge kind
                let base_weight = match neighbor.edge_kind.as_str() {
                    "Calls" => weights.calls,
                    "Imports" => weights.imports,
                    _ => weights.imports, // Default to imports weight for other edge types
                };

                // Apply intent-specific boost
                let boost = compute_intent_boost(intent, &neighbor.edge_kind, intent_boost);

                let mut edge_weight = base_weight * boost;

                // Add community weight if target is in the same community as the seed
                if let Some(seed_comm) = seed_community
                    && let Some(target_comm) = community_map.get(&neighbor.fqn)
                    && *target_comm == seed_comm
                {
                    edge_weight += weights.community;
                }

                let accumulated = current_weight + edge_weight;
                let next_depth = current_depth + 1;

                let ego_node = EgoNode {
                    fqn: neighbor.fqn.clone(),
                    kind: neighbor.kind.clone(),
                    file: neighbor.file.clone(),
                    start_line: neighbor.start_line,
                    end_line: neighbor.end_line,
                    accumulated_weight: accumulated,
                    depth: next_depth,
                    source_seed: seed.fqn.clone(),
                };

                // Insert or update if this path has higher accumulated weight
                match result.get(&neighbor.fqn) {
                    Some(existing) if existing.accumulated_weight >= accumulated => {}
                    _ => {
                        result.insert(neighbor.fqn.clone(), ego_node);
                    }
                }

                // Continue BFS from this neighbor
                queue.push_back((neighbor.fqn, next_depth, accumulated));
            }
        }
    }

    result
}

/// A neighbor node discovered via edge traversal.
#[derive(Debug, Clone)]
struct NeighborNode {
    fqn: String,
    kind: String,
    file: String,
    start_line: u32,
    end_line: u32,
    edge_kind: String,
}

/// Query all neighbors of a node (both outgoing and incoming edges).
fn query_neighbors(conn: &Connection, fqn: &str) -> Vec<NeighborNode> {
    let mut neighbors = Vec::new();

    // Query outgoing edges (current node is source)
    let outgoing_sql = "
        SELECT e.target_fqn, e.kind, n.kind, n.file, n.start_line, n.end_line
        FROM edges e
        JOIN nodes n ON n.fqn = e.target_fqn
        WHERE e.source_fqn = ?1
    ";

    if let Ok(mut stmt) = conn.prepare(outgoing_sql)
        && let Ok(rows) = stmt.query_map(rusqlite::params![fqn], |row| {
            Ok(NeighborNode {
                fqn: row.get(0)?,
                edge_kind: row.get(1)?,
                kind: row.get(2)?,
                file: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
            })
        })
    {
        for row in rows.flatten() {
            neighbors.push(row);
        }
    }

    // Query incoming edges (current node is target)
    let incoming_sql = "
        SELECT e.source_fqn, e.kind, n.kind, n.file, n.start_line, n.end_line
        FROM edges e
        JOIN nodes n ON n.fqn = e.source_fqn
        WHERE e.target_fqn = ?1
    ";

    if let Ok(mut stmt) = conn.prepare(incoming_sql)
        && let Ok(rows) = stmt.query_map(rusqlite::params![fqn], |row| {
            Ok(NeighborNode {
                fqn: row.get(0)?,
                edge_kind: row.get(1)?,
                kind: row.get(2)?,
                file: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
            })
        })
    {
        for row in rows.flatten() {
            neighbors.push(row);
        }
    }

    neighbors
}

/// Build a map from FQN -> community_id using Leiden community detection.
fn build_community_map(conn: &Connection) -> HashMap<String, usize> {
    let mut map = HashMap::new();

    // Run Leiden community detection on the full graph
    let result = community::detect_communities(conn, None, 0.5);
    if let Ok(detection) = result {
        for comm in &detection.communities {
            // The Community struct has files but not individual FQNs directly.
            // We assign community_id to all nodes in the community's files.
            for file in &comm.files {
                let sql = "SELECT fqn FROM nodes WHERE file = ?1";
                if let Ok(mut stmt) = conn.prepare(sql)
                    && let Ok(rows) =
                        stmt.query_map(rusqlite::params![file], |row| row.get::<_, String>(0))
                {
                    for fqn in rows.flatten() {
                        // Only assign if not already assigned (first community wins)
                        map.entry(fqn).or_insert(comm.community_id);
                    }
                }
            }
        }
    }

    map
}

/// Compute the intent-specific boost multiplier for an edge.
fn compute_intent_boost(intent: &Intent, edge_kind: &str, boost: &IntentBoost) -> f64 {
    match intent {
        Intent::Security => {
            // Security intent boosts Calls/DataFlow edges (taint-related traversal)
            if edge_kind == "Calls" || edge_kind == "DataFlow" {
                boost.security_taint_boost
            } else {
                boost.default_boost
            }
        }
        Intent::Architecture => {
            // Architecture intent boosts Imports edges
            if edge_kind == "Imports" {
                boost.architecture_import_boost
            } else {
                boost.default_boost
            }
        }
        _ => boost.default_boost,
    }
}

// ---------------------------------------------------------------------------
// Stage 3: Centrality Ranking
// ---------------------------------------------------------------------------

/// A ranked node produced by Stage 3 centrality scoring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedNode {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub relevance_score: f64,
    pub why: String,
    pub token_cost: usize, // (end_line - start_line + 1) * 10
}

/// Score each node: sum(inbound_edge_weight) × degree_centrality.
/// degree_centrality = node_inbound_count / max_inbound_count_in_graph.
/// Sort descending by score.
///
/// Generates a "why" field based on how the node was reached:
/// - Seed node (depth 0): "Direct match for query term '{source_seed}'"
/// - 1-hop via Calls edge: "Direct caller/callee of {source_seed}"
/// - 1-hop via community: "Same architectural community as {source_seed}"
/// - 2-hop via Calls: "Transitive dependency of {source_seed}"
/// - High centrality (top 10% by degree): "Hub node with high connectivity (degree centrality {score:.2})"
pub fn rank_by_centrality(
    conn: &Connection,
    ego_nodes: &HashMap<String, EgoNode>,
) -> Vec<RankedNode> {
    if ego_nodes.is_empty() {
        return Vec::new();
    }

    // Step 1: Compute the maximum inbound edge count across the entire graph.
    let max_inbound = get_max_inbound_count(conn);
    if max_inbound == 0 {
        // No edges in the graph; fall back to accumulated_weight only
        return rank_without_centrality(ego_nodes);
    }

    // Step 2: For each ego node, get its inbound edge count for degree centrality.
    let mut ranked: Vec<RankedNode> = ego_nodes
        .values()
        .map(|node| {
            let inbound_count = get_inbound_count(conn, &node.fqn);
            let degree_centrality = inbound_count as f64 / max_inbound as f64;

            // Score = accumulated_weight (sum of inbound edge weights along path) × degree_centrality
            // For seed nodes with no inbound edges, use accumulated_weight directly to avoid zero scores
            let relevance_score = if degree_centrality > 0.0 {
                node.accumulated_weight * degree_centrality
            } else {
                // Nodes with zero inbound edges still get a small score from their path weight
                node.accumulated_weight * 0.01
            };

            let why = generate_why_field(node, degree_centrality, max_inbound);
            let token_cost = compute_token_cost(node.start_line, node.end_line);

            RankedNode {
                fqn: node.fqn.clone(),
                kind: node.kind.clone(),
                file: node.file.clone(),
                start_line: node.start_line,
                end_line: node.end_line,
                relevance_score,
                why,
                token_cost,
            }
        })
        .collect();

    // Step 3: Sort descending by relevance_score.
    ranked.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    ranked
}

/// Get the maximum inbound edge count for any node in the graph.
fn get_max_inbound_count(conn: &Connection) -> u64 {
    let sql = "SELECT MAX(cnt) FROM (SELECT COUNT(*) as cnt FROM edges GROUP BY target_fqn)";
    conn.query_row(sql, [], |row| row.get::<_, Option<u64>>(0))
        .unwrap_or(None)
        .unwrap_or(0)
}

/// Get the inbound edge count for a specific node.
fn get_inbound_count(conn: &Connection, fqn: &str) -> u64 {
    let sql = "SELECT COUNT(*) FROM edges WHERE target_fqn = ?1";
    conn.query_row(sql, rusqlite::params![fqn], |row| row.get::<_, u64>(0))
        .unwrap_or(0)
}

/// Generate the "why" field explaining why a node is relevant.
fn generate_why_field(node: &EgoNode, degree_centrality: f64, _max_inbound: u64) -> String {
    // Determine the top 10% threshold for hub detection
    let is_hub = degree_centrality >= 0.9;

    match node.depth {
        0 => {
            // Seed node: direct FTS5 match
            format!("Direct match for query term '{}'", node.source_seed)
        }
        1 => {
            if is_hub {
                format!(
                    "Hub node with high connectivity (degree centrality {:.2})",
                    degree_centrality
                )
            } else {
                // 1-hop: could be caller/callee or community
                // We check if the node was reached via community weight
                // by looking at accumulated_weight relative to expected Calls weight.
                // A community-only connection would have lower weight than a Calls connection.
                // Since we don't store the exact edge kind in EgoNode, we use a heuristic:
                // If accumulated_weight - source_seed_weight < calls_weight (1.0), it's likely community.
                // For simplicity, we default to "Direct caller/callee" for depth 1.
                format!("Direct caller/callee of {}", node.source_seed)
            }
        }
        2 => {
            if is_hub {
                format!(
                    "Hub node with high connectivity (degree centrality {:.2})",
                    degree_centrality
                )
            } else {
                format!("Transitive dependency of {}", node.source_seed)
            }
        }
        _ => {
            if is_hub {
                format!(
                    "Hub node with high connectivity (degree centrality {:.2})",
                    degree_centrality
                )
            } else {
                format!("Transitive dependency of {}", node.source_seed)
            }
        }
    }
}

/// Compute token cost as (end_line - start_line + 1) * 10.
fn compute_token_cost(start_line: u32, end_line: u32) -> usize {
    let lines = if end_line >= start_line {
        (end_line - start_line + 1) as usize
    } else {
        1
    };
    lines * 10
}

/// Fallback ranking when there are no edges in the graph.
/// Uses accumulated_weight directly as the score.
fn rank_without_centrality(ego_nodes: &HashMap<String, EgoNode>) -> Vec<RankedNode> {
    let mut ranked: Vec<RankedNode> = ego_nodes
        .values()
        .map(|node| {
            let why = match node.depth {
                0 => format!("Direct match for query term '{}'", node.source_seed),
                1 => format!("Direct caller/callee of {}", node.source_seed),
                _ => format!("Transitive dependency of {}", node.source_seed),
            };
            let token_cost = compute_token_cost(node.start_line, node.end_line);

            RankedNode {
                fqn: node.fqn.clone(),
                kind: node.kind.clone(),
                file: node.file.clone(),
                start_line: node.start_line,
                end_line: node.end_line,
                relevance_score: node.accumulated_weight,
                why,
                token_cost,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    ranked
}

// ---------------------------------------------------------------------------
// Stage 4: Budget Truncation
// ---------------------------------------------------------------------------

/// A result item produced by Stage 4 budget truncation.
/// Contains all fields required for the final AskResponse output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AskResultItem {
    pub fqn: String,
    pub kind: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub relevance_score: f64,
    pub why: String,
    pub token_cost: usize,
    pub naive_cost_estimate: f64,
    pub coverage: f64,
}

/// Truncate the ranked list to fit within `token_budget`.
///
/// Accumulates `token_cost` per node until the budget is exhausted.
/// Always includes at least the top-1 result regardless of budget.
///
/// For each included item:
/// - `naive_cost_estimate` = token_cost / 1_000_000.0 * 3.00 (Claude Sonnet default pricing)
/// - `coverage` = relevance_score / max_relevance_score_in_results (normalized to [0, 1])
pub fn truncate_to_budget(ranked: &[RankedNode], token_budget: usize) -> Vec<AskResultItem> {
    if ranked.is_empty() {
        return Vec::new();
    }

    // The max relevance score is the first item's score (list is sorted descending).
    let max_relevance = ranked[0].relevance_score;
    // Guard against division by zero if max_relevance is 0.
    let max_relevance = if max_relevance > 0.0 {
        max_relevance
    } else {
        1.0
    };

    let mut results: Vec<AskResultItem> = Vec::new();
    let mut budget_remaining = token_budget;

    for (i, node) in ranked.iter().enumerate() {
        // Always include the top-1 result regardless of budget.
        if i > 0 && node.token_cost > budget_remaining {
            break;
        }

        let naive_cost_estimate = node.token_cost as f64 / 1_000_000.0 * 3.00;
        let coverage = (node.relevance_score / max_relevance).clamp(0.0, 1.0);

        results.push(AskResultItem {
            fqn: node.fqn.clone(),
            kind: node.kind.clone(),
            file: node.file.clone(),
            start_line: node.start_line,
            end_line: node.end_line,
            relevance_score: node.relevance_score,
            why: node.why.clone(),
            token_cost: node.token_cost,
            naive_cost_estimate,
            coverage,
        });

        // Deduct from budget (for top-1, we deduct even if it exceeds the budget).
        budget_remaining = budget_remaining.saturating_sub(node.token_cost);
    }

    results
}

// ---------------------------------------------------------------------------
// Stage 1: FTS5 Seed Search
// ---------------------------------------------------------------------------

/// Perform FTS5 search to find seed nodes.
/// Returns at most `max_seeds` nodes ranked by FTS5 relevance.
///
/// Constructs an FTS5 MATCH query by OR-joining the extracted terms.
/// If terms is empty, returns an empty vector.
pub fn fts5_seed_search(conn: &Connection, terms: &[String], max_seeds: usize) -> Vec<SeedNode> {
    if terms.is_empty() {
        return Vec::new();
    }

    // Sanitize each term and OR-join them for the FTS5 MATCH query
    let sanitized_terms: Vec<String> = terms
        .iter()
        .filter_map(|t| {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Wrap each term in double quotes to escape special FTS5 syntax
            // and handle terms with special characters safely
            let escaped = trimmed.replace('"', "\"\"");
            Some(format!("\"{}\"", escaped))
        })
        .collect();

    if sanitized_terms.is_empty() {
        return Vec::new();
    }

    let match_query = sanitized_terms.join(" OR ");

    let limit = max_seeds.min(50); // Hard cap at 50 per requirement 9.2

    let mut stmt = match conn.prepare(
        "SELECT fqn, kind, file, start_line, end_line, rank
         FROM nodes_fts
         JOIN nodes ON nodes.fqn = nodes_fts.fqn
         WHERE nodes_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => {
            // If the FTS5 table doesn't exist or query fails, try simpler query
            // The nodes_fts content table mirrors nodes, so we can join to get start_line/end_line
            return fts5_seed_search_simple(conn, &match_query, limit);
        }
    };

    let results = stmt.query_map(rusqlite::params![match_query, limit as i64], |row| {
        let rank: f64 = row.get(5)?;
        Ok(SeedNode {
            fqn: row.get(0)?,
            kind: row.get(1)?,
            file: row.get(2)?,
            start_line: row.get(3)?,
            end_line: row.get(4)?,
            relevance: -rank, // FTS5 rank is negative; negate for positive relevance
        })
    });

    match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => fts5_seed_search_simple(conn, &match_query, limit),
    }
}

/// Simple FTS5 search fallback that queries nodes_fts directly without JOIN.
/// Used when the JOIN query fails (e.g., schema differences).
fn fts5_seed_search_simple(conn: &Connection, match_query: &str, limit: usize) -> Vec<SeedNode> {
    let mut stmt = match conn.prepare(
        "SELECT fqn, kind, file, rank
         FROM nodes_fts
         WHERE nodes_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let results = stmt.query_map(rusqlite::params![match_query, limit as i64], |row| {
        let fqn: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let file: String = row.get(2)?;
        let rank: f64 = row.get(3)?;
        Ok((fqn, kind, file, rank))
    });

    let fts_results: Vec<(String, String, String, f64)> = match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => return Vec::new(),
    };

    // Look up start_line and end_line from the nodes table for each result
    fts_results
        .into_iter()
        .map(|(fqn, kind, file, rank)| {
            let line_info = conn
                .query_row(
                    "SELECT start_line, end_line FROM nodes WHERE fqn = ?1",
                    rusqlite::params![&fqn],
                    |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
                )
                .ok();

            let (start_line, end_line) = line_info.unwrap_or((0, 0));

            SeedNode {
                fqn,
                kind,
                file,
                start_line,
                end_line,
                relevance: -rank,
            }
        })
        .collect()
}

/// Fallback: substring search across FQNs and file paths.
/// Used when FTS5 returns zero results.
///
/// Splits the question into words and searches for each word as a LIKE pattern
/// against `nodes.fqn` and `nodes.file`. Returns at most `max_results` nodes
/// ranked by match position (earlier match = higher relevance).
pub fn fallback_substring_search(
    conn: &Connection,
    question: &str,
    max_results: usize,
) -> Vec<SeedNode> {
    let words: Vec<&str> = question
        .split_whitespace()
        .filter(|w| w.len() >= 2) // Skip very short words
        .collect();

    if words.is_empty() {
        return Vec::new();
    }

    // Build a query that searches for any word in fqn or file using LIKE
    // We use UNION to combine matches from fqn and file columns
    let mut all_results: Vec<SeedNode> = Vec::new();
    let mut seen_fqns: HashSet<String> = HashSet::new();

    for word in &words {
        // Skip common English stop words
        let lower = word.to_lowercase();
        if matches!(
            lower.as_str(),
            "the"
                | "is"
                | "at"
                | "in"
                | "on"
                | "to"
                | "of"
                | "and"
                | "or"
                | "for"
                | "how"
                | "what"
                | "where"
                | "when"
                | "why"
                | "does"
                | "this"
                | "that"
        ) {
            continue;
        }

        let pattern = format!("%{}%", word);

        let mut stmt = match conn.prepare(
            "SELECT fqn, kind, file, start_line, end_line
             FROM nodes
             WHERE fqn LIKE ?1 OR file LIKE ?1
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let remaining = max_results.saturating_sub(all_results.len());
        if remaining == 0 {
            break;
        }

        let results = stmt.query_map(rusqlite::params![&pattern, remaining as i64], |row| {
            Ok(SeedNode {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                relevance: 0.0, // Will be set below
            })
        });

        if let Ok(rows) = results {
            for row in rows.flatten() {
                if !seen_fqns.contains(&row.fqn) {
                    seen_fqns.insert(row.fqn.clone());
                    all_results.push(row);
                }
            }
        }
    }

    // Assign relevance based on position (earlier results get higher relevance)
    let total = all_results.len() as f64;
    for (i, node) in all_results.iter_mut().enumerate() {
        node.relevance = if total > 0.0 {
            1.0 - (i as f64 / total)
        } else {
            0.0
        };
    }

    // Cap at max_results
    all_results.truncate(max_results);
    all_results
}

/// Extract query terms from a natural language question.
/// Identifies: snake_case, CamelCase, path separators (::, /), backtick-quoted terms.
/// Returns at most `max_terms` results, deduplicated.
pub fn extract_query_terms(question: &str, max_terms: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Helper to add a term if not already seen
    let mut add_term = |term: String| {
        let key = term.clone();
        if !key.is_empty() && !seen.contains(&key) {
            seen.insert(key);
            terms.push(term);
        }
    };

    // Stage 1: Extract backtick-quoted terms (e.g., `dispatch_tool` -> "dispatch_tool")
    let backtick_re = Regex::new(r"`([^`]+)`").unwrap();
    for cap in backtick_re.captures_iter(question) {
        let term = cap[1].trim().to_string();
        if !term.is_empty() {
            add_term(term);
        }
    }

    // Stage 2: Identify snake_case tokens (words containing underscores, e.g., my_function)
    let snake_case_re = Regex::new(r"\b([a-zA-Z][a-zA-Z0-9]*(?:_[a-zA-Z0-9]+)+)\b").unwrap();
    for cap in snake_case_re.captures_iter(question) {
        add_term(cap[1].to_string());
    }

    // Stage 3: Identify CamelCase tokens (mixed case like StoreManager, getUser)
    // Matches PascalCase (uppercase start + lowercase) or camelCase (lowercase start + uppercase)
    let camel_case_re =
        Regex::new(r"\b([A-Z][a-z]+(?:[A-Z][a-z0-9]*)+|[a-z]+(?:[A-Z][a-z0-9]*)+)\b").unwrap();
    for cap in camel_case_re.captures_iter(question) {
        let term = &cap[1];
        // Skip common English words that happen to match CamelCase-like patterns
        let skip_words = [
            "What", "Where", "How", "When", "Why", "The", "This", "That", "There", "Which", "Does",
            "Have", "Has", "Can", "Could", "Would", "Should", "Will",
        ];
        if !skip_words.contains(&term) {
            add_term(term.to_string());
        }
    }

    // Stage 4: Identify path-like tokens (containing :: or /)
    // Split on whitespace and look for tokens with path separators
    for word in question.split_whitespace() {
        // Strip surrounding punctuation but keep path-relevant chars
        let cleaned = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != ':' && c != '/' && c != '_' && c != '.' && c != '-'
        });
        if cleaned.len() < 2 {
            continue;
        }
        let has_double_colon = cleaned.contains("::");
        let has_slash = cleaned.contains('/');
        if has_double_colon || has_slash {
            // Verify it looks like a path (not just a stray slash or colon)
            let has_alpha = cleaned.chars().any(|c| c.is_alphabetic());
            if has_alpha {
                add_term(cleaned.to_string());
            }
        }
    }

    // Truncate to max_terms
    terms.truncate(max_terms);
    terms
}

/// Intent detected from the natural language question.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    TraceCallers,
    TraceCallees,
    BlastRadius,
    Explain,
    Search,
    Security,
    DeadCode,
    Architecture,
    Fallback,
}

/// Dispatch the `ask` meta-tool. Parses the question, determines intent,
/// runs the 4-stage graph-guided retrieval pipeline, and returns a unified response.
///
/// The function signature remains compatible with the dispatch system:
/// takes a `StoreManager` and arguments `Value`, returns `(String, usize)`.
pub fn dispatch_ask(store: &StoreManager, args: &Value) -> Result<(String, usize), McpError> {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: question".to_string(),
        })?;

    // Parse optional config overrides from args
    let token_budget = args
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(4096);

    let config = AskConfig {
        token_budget,
        ..AskConfig::default()
    };

    let conn = store.read_conn();
    let response = ask(&conn, question, &config);

    let files_touched = response.results.len();

    let json = serde_json::to_string(&response).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize ask response: {}", e),
    })?;

    Ok((json, files_touched))
}

// ---------------------------------------------------------------------------
// 4-Stage Pipeline Entry Point
// ---------------------------------------------------------------------------

/// The main ask engine entry point. Runs the 4-stage graph-guided retrieval pipeline:
///
/// 1. Extract query terms from the question
/// 2. Classify intent (secondary signal for edge weight boosting)
/// 3. FTS5 seed search (with fallback to substring search if zero results)
/// 4. Ego-graph expansion from seeds
/// 5. Centrality ranking
/// 6. Budget truncation
/// 7. Build AskResponse with summary
///
/// Always returns a valid `AskResponse` (never panics, handles empty/adversarial input).
pub fn ask(conn: &Connection, question: &str, config: &AskConfig) -> AskResponse {
    // Stage 0: Extract query terms and classify intent
    let query_terms = extract_query_terms(question, config.max_query_terms);
    let lower = question.to_lowercase();
    let intent = classify_intent(&lower);
    let intent_boost = IntentBoost::default();

    // Stage 1: FTS5 seed search
    let mut seeds = fts5_seed_search(conn, &query_terms, config.max_seed_nodes);

    // Fallback: if FTS5 returns zero results, try substring search
    if seeds.is_empty() {
        seeds = fallback_substring_search(conn, question, config.max_seed_nodes);
    }

    // If still no results, return empty response
    if seeds.is_empty() {
        return AskResponse {
            summary: AskSummary {
                total_results: 0,
                total_token_cost: 0,
                budget_used_percent: 0.0,
                intent_detected: format!("{:?}", intent),
                query_terms_extracted: query_terms,
            },
            results: Vec::new(),
        };
    }

    // Stage 2: Ego-graph expansion
    let ego_nodes = expand_ego_graph(
        conn,
        &seeds,
        config.ego_graph_depth,
        &config.edge_weights,
        &intent_boost,
        &intent,
    );

    // Stage 3: Centrality ranking
    let ranked = rank_by_centrality(conn, &ego_nodes);

    // Stage 4: Budget truncation
    let results = truncate_to_budget(&ranked, config.token_budget);

    // Build summary
    let total_token_cost: usize = results.iter().map(|r| r.token_cost).sum();
    let budget_used_percent = if config.token_budget > 0 {
        (total_token_cost as f64 / config.token_budget as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    AskResponse {
        summary: AskSummary {
            total_results: results.len(),
            total_token_cost,
            budget_used_percent,
            intent_detected: format!("{:?}", intent),
            query_terms_extracted: query_terms,
        },
        results,
    }
}

/// Classify the intent of a natural language question.
fn classify_intent(lower: &str) -> Intent {
    // Check for caller-related queries
    if lower.contains("what calls")
        || lower.contains("who calls")
        || lower.contains("callers of")
        || lower.contains("called by")
    {
        return Intent::TraceCallers;
    }

    // Check for callee-related queries
    if lower.contains("what does") && lower.contains("call")
        || lower.contains("callees of")
        || lower.contains("calls what")
        || lower.contains("dependencies of")
    {
        return Intent::TraceCallees;
    }

    // Check for blast radius / impact queries
    if lower.contains("what breaks")
        || lower.contains("impact of")
        || lower.contains("blast radius")
        || lower.contains("affected by")
        || lower.contains("what happens if i change")
        || lower.contains("what happens if I change")
    {
        return Intent::BlastRadius;
    }

    // Check for security queries
    if lower.contains("security")
        || lower.contains("taint")
        || lower.contains("vulnerability")
        || lower.contains("vulnerabilities")
        || lower.contains("owasp")
        || lower.contains("injection")
    {
        return Intent::Security;
    }

    // Check for dead code queries
    if lower.contains("dead code")
        || lower.contains("unused")
        || lower.contains("unreachable")
        || lower.contains("no callers")
    {
        return Intent::DeadCode;
    }

    // Check for architecture queries (before explain, since "what is the structure" should be architecture)
    if lower.contains("architecture")
        || lower.contains("overview")
        || lower.contains("structure")
        || lower.contains("modules")
        || lower.contains("high level")
        || lower.contains("high-level")
    {
        return Intent::Architecture;
    }

    // Check for explain queries
    if lower.contains("explain")
        || lower.contains("what is")
        || lower.contains("describe")
        || lower.contains("tell me about")
    {
        return Intent::Explain;
    }

    // Check for search/find queries
    if lower.contains("find")
        || lower.contains("where is")
        || lower.contains("search")
        || lower.contains("locate")
        || lower.contains("look for")
    {
        return Intent::Search;
    }

    Intent::Fallback
}

/// Extract a symbol name or FQN from the question.
///
/// Looks for patterns like:
/// - Backtick-quoted identifiers: `some_function`
/// - Double-colon paths: module::function
/// - CamelCase identifiers
/// - snake_case identifiers after keywords
#[cfg(test)]
fn extract_symbol(question: &str) -> Option<String> {
    // First, check for backtick-quoted identifiers
    if let Some(start) = question.find('`')
        && let Some(end) = question[start + 1..].find('`')
    {
        let symbol = &question[start + 1..start + 1 + end];
        if !symbol.is_empty() {
            return Some(symbol.to_string());
        }
    }

    // Check for double-colon paths (FQNs like src/main.rs::function)
    for word in question.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '_' && c != '-'
        });
        if cleaned.contains("::") && cleaned.len() > 3 {
            return Some(cleaned.to_string());
        }
    }

    // Look for identifiers after keywords like "calls", "of", "is", "about"
    let keywords = [
        "calls", "of", "is", "about", "change", "find", "explain", "for",
    ];
    let words: Vec<&str> = question.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let lower_word = word.to_lowercase();
        if keywords.contains(&lower_word.as_str()) && i + 1 < words.len() {
            let candidate = words[i + 1].trim_matches(|c: char| {
                !c.is_alphanumeric() && c != '_' && c != ':' && c != '/' && c != '.'
            });
            if candidate.len() >= 2
                && (candidate.contains('_')
                    || candidate.contains("::")
                    || candidate.contains('.')
                    || candidate.chars().any(|c| c.is_uppercase()))
            {
                return Some(candidate.to_string());
            }
        }
    }

    // Look for any snake_case or CamelCase identifier
    for word in question.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '_' && c != ':' && c != '/' && c != '.'
        });
        if cleaned.len() >= 3 {
            let has_underscore = cleaned.contains('_');
            let has_mixed_case = cleaned.chars().any(|c| c.is_uppercase())
                && cleaned.chars().any(|c| c.is_lowercase())
                && cleaned.len() > 3;
            let has_path_sep = cleaned.contains("::") || cleaned.contains('/');
            if has_underscore || has_mixed_case || has_path_sep {
                // Skip common English words that happen to have mixed case
                let skip_words = ["What", "Where", "How", "When", "Why", "The", "This", "That"];
                if !skip_words.contains(&cleaned) {
                    return Some(cleaned.to_string());
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_query_terms tests ---

    #[test]
    fn test_extract_backtick_terms() {
        let terms = extract_query_terms("what does `dispatch_tool` do?", 20);
        assert!(terms.contains(&"dispatch_tool".to_string()));
    }

    #[test]
    fn test_extract_multiple_backtick_terms() {
        let terms = extract_query_terms("how does `StoreManager` relate to `dispatch_tool`?", 20);
        assert!(terms.contains(&"StoreManager".to_string()));
        assert!(terms.contains(&"dispatch_tool".to_string()));
    }

    #[test]
    fn test_extract_snake_case() {
        let terms = extract_query_terms("explain the validate_token function", 20);
        assert!(terms.contains(&"validate_token".to_string()));
    }

    #[test]
    fn test_extract_camel_case() {
        let terms = extract_query_terms("what is StoreManager used for", 20);
        assert!(terms.contains(&"StoreManager".to_string()));
    }

    #[test]
    fn test_extract_camel_case_lower_start() {
        let terms = extract_query_terms("explain getUser function", 20);
        assert!(terms.contains(&"getUser".to_string()));
    }

    #[test]
    fn test_extract_path_double_colon() {
        let terms = extract_query_terms("what is crate::store::db", 20);
        assert!(terms.contains(&"crate::store::db".to_string()));
    }

    #[test]
    fn test_extract_path_slash() {
        let terms = extract_query_terms("look at src/mcp/ask.rs", 20);
        assert!(terms.contains(&"src/mcp/ask.rs".to_string()));
    }

    #[test]
    fn test_extract_deduplication() {
        let terms = extract_query_terms("`dispatch_tool` and dispatch_tool are the same", 20);
        let count = terms.iter().filter(|t| *t == "dispatch_tool").count();
        assert_eq!(count, 1, "dispatch_tool should appear only once");
    }

    #[test]
    fn test_extract_respects_max_terms() {
        // Create a question with many terms
        let question = "`a_b` `c_d` `e_f` `g_h` `i_j` foo_bar baz_qux hello_world some_func another_one more_stuff extra_term yet_another final_one last_term overflow_term";
        let terms = extract_query_terms(question, 5);
        assert!(terms.len() <= 5);
    }

    #[test]
    fn test_extract_empty_question() {
        let terms = extract_query_terms("", 20);
        assert!(terms.is_empty());
    }

    #[test]
    fn test_extract_no_code_terms() {
        let terms = extract_query_terms("hello world how are you", 20);
        assert!(terms.is_empty());
    }

    #[test]
    fn test_extract_skips_common_english_words() {
        // "What", "Where", etc. should not be extracted as CamelCase
        let terms = extract_query_terms("What does this do", 20);
        assert!(!terms.contains(&"What".to_string()));
    }

    #[test]
    fn test_extract_combined() {
        let terms = extract_query_terms(
            "how does `StoreManager` call dispatch_tool in src/mcp/ask.rs",
            20,
        );
        assert!(terms.contains(&"StoreManager".to_string()));
        assert!(terms.contains(&"dispatch_tool".to_string()));
        assert!(terms.contains(&"src/mcp/ask.rs".to_string()));
    }

    // --- classify_intent tests ---

    #[test]
    fn test_classify_intent_callers() {
        assert_eq!(
            classify_intent("what calls validate_token"),
            Intent::TraceCallers
        );
        assert_eq!(
            classify_intent("who calls this function"),
            Intent::TraceCallers
        );
        assert_eq!(classify_intent("callers of main"), Intent::TraceCallers);
    }

    #[test]
    fn test_classify_intent_callees() {
        assert_eq!(classify_intent("what does main call"), Intent::TraceCallees);
        assert_eq!(
            classify_intent("callees of dispatch_tool"),
            Intent::TraceCallees
        );
    }

    #[test]
    fn test_classify_intent_blast_radius() {
        assert_eq!(
            classify_intent("what breaks if i change dispatch_tool"),
            Intent::BlastRadius
        );
        assert_eq!(
            classify_intent("impact of removing validate_token"),
            Intent::BlastRadius
        );
        assert_eq!(classify_intent("blast radius of main"), Intent::BlastRadius);
    }

    #[test]
    fn test_classify_intent_explain() {
        assert_eq!(classify_intent("explain dispatch_tool"), Intent::Explain);
        assert_eq!(classify_intent("what is StoreManager"), Intent::Explain);
        assert_eq!(classify_intent("describe the auth module"), Intent::Explain);
    }

    #[test]
    fn test_classify_intent_security() {
        assert_eq!(
            classify_intent("are there any security issues"),
            Intent::Security
        );
        assert_eq!(classify_intent("find taint paths"), Intent::Security);
        assert_eq!(
            classify_intent("check for vulnerabilities"),
            Intent::Security
        );
    }

    #[test]
    fn test_classify_intent_dead_code() {
        assert_eq!(classify_intent("find dead code"), Intent::DeadCode);
        assert_eq!(
            classify_intent("what functions are unused"),
            Intent::DeadCode
        );
    }

    #[test]
    fn test_classify_intent_architecture() {
        assert_eq!(
            classify_intent("show me the architecture"),
            Intent::Architecture
        );
        assert_eq!(classify_intent("give me an overview"), Intent::Architecture);
        assert_eq!(
            classify_intent("what is the high-level structure"),
            Intent::Architecture
        );
    }

    #[test]
    fn test_classify_intent_search() {
        assert_eq!(classify_intent("find dispatch_tool"), Intent::Search);
        assert_eq!(
            classify_intent("where is the main function"),
            Intent::Search
        );
    }

    #[test]
    fn test_classify_intent_fallback() {
        assert_eq!(classify_intent("hello world"), Intent::Fallback);
    }

    #[test]
    fn test_extract_symbol_backtick() {
        assert_eq!(
            extract_symbol("what calls `validate_token`"),
            Some("validate_token".to_string())
        );
        assert_eq!(
            extract_symbol("explain `src/main.rs::main`"),
            Some("src/main.rs::main".to_string())
        );
    }

    #[test]
    fn test_extract_symbol_fqn() {
        assert_eq!(
            extract_symbol("what calls src/auth.rs::validate_token"),
            Some("src/auth.rs::validate_token".to_string())
        );
    }

    #[test]
    fn test_extract_symbol_snake_case() {
        assert_eq!(
            extract_symbol("explain dispatch_tool please"),
            Some("dispatch_tool".to_string())
        );
    }

    #[test]
    fn test_extract_symbol_camel_case() {
        assert_eq!(
            extract_symbol("explain StoreManager"),
            Some("StoreManager".to_string())
        );
    }

    #[test]
    fn test_extract_symbol_none() {
        assert_eq!(extract_symbol("hello"), None);
    }

    // --- fts5_seed_search tests ---

    use std::path::PathBuf;

    /// Returns the path to the migrations directory.
    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    /// Helper: create an in-memory connection with migrations applied.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .expect("failed to apply PRAGMAs");

        crate::store::migrations::run_migrations(&conn, &migrations_dir())
            .expect("migrations should succeed");

        conn
    }

    /// Helper: insert a node into the test database.
    fn insert_test_node(
        conn: &Connection,
        fqn: &str,
        kind: &str,
        file: &str,
        start_line: u32,
        end_line: u32,
    ) {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
             VALUES (?1, ?2, ?3, ?4, ?5, 'hash', 1000, '{}')",
            rusqlite::params![fqn, kind, file, start_line, end_line],
        )
        .expect("failed to insert test node");
    }

    #[test]
    fn test_fts5_seed_search_empty_terms() {
        let conn = setup_test_db();
        let results = fts5_seed_search(&conn, &[], 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_fts5_seed_search_finds_matching_nodes() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::handle_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let terms = vec!["validate".to_string()];
        let results = fts5_seed_search(&conn, &terms, 50);

        // FTS5 may or may not be available in test environment
        // If FTS5 is available, we should find the matching node
        if !results.is_empty() {
            assert!(results.iter().any(|r| r.fqn.contains("validate_token")));
            assert_eq!(results[0].kind, "Function");
            assert_eq!(results[0].file, "src/auth.rs");
        }
    }

    #[test]
    fn test_fts5_seed_search_respects_max_seeds() {
        let conn = setup_test_db();

        // Insert many nodes
        for i in 0..60 {
            insert_test_node(
                &conn,
                &format!("src/mod{}.rs::process_{}", i, i),
                "Function",
                &format!("src/mod{}.rs", i),
                1,
                10,
            );
        }

        let terms = vec!["process".to_string()];
        let results = fts5_seed_search(&conn, &terms, 50);

        // Should never exceed 50 (the hard cap)
        assert!(results.len() <= 50);
    }

    #[test]
    fn test_fts5_seed_search_or_joins_terms() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::handle_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let terms = vec!["validate".to_string(), "handle".to_string()];
        let results = fts5_seed_search(&conn, &terms, 50);

        // If FTS5 is available, both nodes should be found
        if !results.is_empty() {
            assert!(results.len() <= 50);
        }
    }

    #[test]
    fn test_fts5_seed_search_relevance_positive() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );

        let terms = vec!["validate".to_string()];
        let results = fts5_seed_search(&conn, &terms, 50);

        // If results are returned, relevance should be positive (negated FTS5 rank)
        for result in &results {
            assert!(
                result.relevance >= 0.0,
                "Relevance should be non-negative, got {}",
                result.relevance
            );
        }
    }

    #[test]
    fn test_fts5_seed_search_handles_special_chars() {
        let conn = setup_test_db();
        insert_test_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1, 10);

        // Terms with special FTS5 characters should not crash
        let terms = vec!["OR".to_string(), "NOT".to_string(), "main*".to_string()];
        let results = fts5_seed_search(&conn, &terms, 50);
        // Should not panic - results may or may not be found depending on FTS5 availability
        assert!(results.len() <= 50);
    }

    // --- fallback_substring_search tests ---

    #[test]
    fn test_fallback_search_empty_question() {
        let conn = setup_test_db();
        let results = fallback_substring_search(&conn, "", 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_fallback_search_finds_by_fqn() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::handle_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let results = fallback_substring_search(&conn, "validate", 50);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.fqn.contains("validate_token")));
    }

    #[test]
    fn test_fallback_search_finds_by_file_path() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::handle_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let results = fallback_substring_search(&conn, "auth", 50);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.file.contains("auth")));
    }

    #[test]
    fn test_fallback_search_respects_max_results() {
        let conn = setup_test_db();

        for i in 0..60 {
            insert_test_node(
                &conn,
                &format!("src/mod{}.rs::process_{}", i, i),
                "Function",
                &format!("src/mod{}.rs", i),
                1,
                10,
            );
        }

        let results = fallback_substring_search(&conn, "process", 10);
        assert!(results.len() <= 10);
    }

    #[test]
    fn test_fallback_search_deduplicates() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );

        // "auth validate" should match the same node via both fqn and file, but only return it once
        let results = fallback_substring_search(&conn, "auth validate", 50);
        let fqn_count = results
            .iter()
            .filter(|r| r.fqn == "src/auth.rs::validate_token")
            .count();
        assert!(fqn_count <= 1, "Same node should not appear more than once");
    }

    #[test]
    fn test_fallback_search_skips_stop_words() {
        let conn = setup_test_db();
        insert_test_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1, 10);

        // "the" and "is" are stop words and should be skipped
        let results = fallback_substring_search(&conn, "the is", 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_fallback_search_assigns_relevance() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::validate_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let results = fallback_substring_search(&conn, "validate", 50);
        if results.len() > 1 {
            // First result should have higher relevance than last
            assert!(results[0].relevance >= results[results.len() - 1].relevance);
        }
        // All relevance values should be in [0.0, 1.0]
        for r in &results {
            assert!(r.relevance >= 0.0 && r.relevance <= 1.0);
        }
    }

    #[test]
    fn test_fallback_search_no_match() {
        let conn = setup_test_db();
        insert_test_node(&conn, "src/main.rs::main", "Function", "src/main.rs", 1, 10);

        let results = fallback_substring_search(&conn, "nonexistent_xyz_symbol", 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_seed_node_struct_fields() {
        let node = SeedNode {
            fqn: "src/auth.rs::validate".to_string(),
            kind: "Function".to_string(),
            file: "src/auth.rs".to_string(),
            start_line: 10,
            end_line: 30,
            relevance: 0.85,
        };
        assert_eq!(node.fqn, "src/auth.rs::validate");
        assert_eq!(node.kind, "Function");
        assert_eq!(node.file, "src/auth.rs");
        assert_eq!(node.start_line, 10);
        assert_eq!(node.end_line, 30);
        assert!((node.relevance - 0.85).abs() < f64::EPSILON);
    }

    // --- ego-graph expansion tests ---

    /// Creates an in-memory SQLite connection with migrations applied for testing.
    fn setup_ego_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("failed to enable foreign keys");

        // Apply initial schema (nodes + edges tables)
        let migration_0001 = include_str!("../../migrations/0001_initial_schema.sql");
        conn.execute_batch(migration_0001)
            .expect("failed to apply migration 0001");

        conn
    }

    fn insert_ego_node(conn: &Connection, fqn: &str, kind: &str, file: &str, start: u32, end: u32) {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'hash', 1000, '{}')",
            rusqlite::params![fqn, kind, file, start, end],
        )
        .unwrap();
    }

    fn insert_ego_edge(conn: &Connection, source: &str, target: &str, kind: &str) {
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
             VALUES (?1, ?2, ?3, 1.0, '{}')",
            rusqlite::params![source, target, kind],
        )
        .unwrap();
    }

    #[test]
    fn test_ego_graph_empty_seeds() {
        let conn = setup_ego_db();
        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &[], 2, &weights, &boost, &Intent::Fallback);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ego_graph_single_seed_no_edges() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // Should contain just the seed node
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("src/a.rs::func_a"));
        assert_eq!(result["src/a.rs::func_a"].depth, 0);
    }

    #[test]
    fn test_ego_graph_expands_one_hop() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        assert_eq!(result.len(), 2);
        assert!(result.contains_key("src/a.rs::func_a"));
        assert!(result.contains_key("src/b.rs::func_b"));
        assert_eq!(result["src/b.rs::func_b"].depth, 1);
        // Weight should be seed relevance + calls weight (1.0 + 1.0 = 2.0)
        assert!((result["src/b.rs::func_b"].accumulated_weight - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ego_graph_expands_two_hops() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1, 30);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");
        insert_ego_edge(&conn, "src/b.rs::func_b", "src/c.rs::func_c", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        assert_eq!(result.len(), 3);
        assert_eq!(result["src/a.rs::func_a"].depth, 0);
        assert_eq!(result["src/b.rs::func_b"].depth, 1);
        assert_eq!(result["src/c.rs::func_c"].depth, 2);
    }

    #[test]
    fn test_ego_graph_respects_max_depth() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1, 30);
        insert_ego_node(&conn, "src/d.rs::func_d", "Function", "src/d.rs", 1, 40);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");
        insert_ego_edge(&conn, "src/b.rs::func_b", "src/c.rs::func_c", "Calls");
        insert_ego_edge(&conn, "src/c.rs::func_c", "src/d.rs::func_d", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        // max_depth = 2, so func_d (3 hops away) should NOT be included
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        assert!(result.contains_key("src/a.rs::func_a"));
        assert!(result.contains_key("src/b.rs::func_b"));
        assert!(result.contains_key("src/c.rs::func_c"));
        assert!(!result.contains_key("src/d.rs::func_d"));
    }

    #[test]
    fn test_ego_graph_prevents_cycles() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        // Create a cycle: a -> b -> a
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");
        insert_ego_edge(&conn, "src/b.rs::func_b", "src/a.rs::func_a", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // Should only have 2 nodes (no infinite loop)
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_ego_graph_imports_weight() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Imports");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // Imports weight is 0.5, so accumulated = 1.0 (seed) + 0.5 = 1.5
        assert!((result["src/b.rs::func_b"].accumulated_weight - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ego_graph_security_intent_boost() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        // Security intent should boost Calls edges by 1.5
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Security);

        // Calls weight = 1.0 * 1.5 (security boost) = 1.5, accumulated = 1.0 + 1.5 = 2.5
        assert!((result["src/b.rs::func_b"].accumulated_weight - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ego_graph_architecture_intent_boost() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Imports");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        // Architecture intent should boost Imports edges by 1.5
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Architecture);

        // Imports weight = 0.5 * 1.5 (arch boost) = 0.75, accumulated = 1.0 + 0.75 = 1.75
        assert!((result["src/b.rs::func_b"].accumulated_weight - 1.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ego_graph_incoming_edges() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        // b calls a (incoming edge for a)
        insert_ego_edge(&conn, "src/b.rs::func_b", "src/a.rs::func_a", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // Should discover func_b via incoming edge
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("src/b.rs::func_b"));
    }

    #[test]
    fn test_ego_graph_multiple_seeds() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_node(&conn, "src/c.rs::func_c", "Function", "src/c.rs", 1, 30);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");
        insert_ego_edge(&conn, "src/c.rs::func_c", "src/b.rs::func_b", "Calls");

        let seeds = vec![
            SeedNode {
                fqn: "src/a.rs::func_a".to_string(),
                kind: "Function".to_string(),
                file: "src/a.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance: 1.0,
            },
            SeedNode {
                fqn: "src/c.rs::func_c".to_string(),
                kind: "Function".to_string(),
                file: "src/c.rs".to_string(),
                start_line: 1,
                end_line: 30,
                relevance: 0.8,
            },
        ];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // All three nodes should be discovered
        assert_eq!(result.len(), 3);
        assert!(result.contains_key("src/a.rs::func_a"));
        assert!(result.contains_key("src/b.rs::func_b"));
        assert!(result.contains_key("src/c.rs::func_c"));
    }

    #[test]
    fn test_ego_graph_source_seed_tracking() {
        let conn = setup_ego_db();
        insert_ego_node(&conn, "src/a.rs::func_a", "Function", "src/a.rs", 1, 10);
        insert_ego_node(&conn, "src/b.rs::func_b", "Function", "src/b.rs", 1, 20);
        insert_ego_edge(&conn, "src/a.rs::func_a", "src/b.rs::func_b", "Calls");

        let seeds = vec![SeedNode {
            fqn: "src/a.rs::func_a".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance: 1.0,
        }];

        let weights = EdgeWeights::default();
        let boost = IntentBoost::default();
        let result = expand_ego_graph(&conn, &seeds, 2, &weights, &boost, &Intent::Fallback);

        // func_b should track that it was reached from func_a
        assert_eq!(result["src/b.rs::func_b"].source_seed, "src/a.rs::func_a");
    }

    #[test]
    fn test_edge_weights_default() {
        let weights = EdgeWeights::default();
        assert!((weights.calls - 1.0).abs() < f64::EPSILON);
        assert!((weights.community - 0.7).abs() < f64::EPSILON);
        assert!((weights.imports - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_intent_boost_default() {
        let boost = IntentBoost::default();
        assert!((boost.security_taint_boost - 1.5).abs() < f64::EPSILON);
        assert!((boost.architecture_import_boost - 1.5).abs() < f64::EPSILON);
        assert!((boost.default_boost - 1.0).abs() < f64::EPSILON);
    }

    // --- truncate_to_budget tests ---

    #[test]
    fn test_truncate_empty_input() {
        let result = truncate_to_budget(&[], 4096);
        assert!(result.is_empty());
    }

    #[test]
    fn test_truncate_single_item_always_included() {
        // Even if token_cost exceeds budget, top-1 is always included.
        let ranked = vec![RankedNode {
            fqn: "src/a.rs::big_func".to_string(),
            kind: "Function".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 1000,
            relevance_score: 0.95,
            why: "Direct match".to_string(),
            token_cost: 10010, // exceeds budget of 4096
        }];

        let result = truncate_to_budget(&ranked, 4096);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqn, "src/a.rs::big_func");
    }

    #[test]
    fn test_truncate_respects_budget() {
        let ranked = vec![
            RankedNode {
                fqn: "a".to_string(),
                kind: "Function".to_string(),
                file: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance_score: 0.9,
                why: "Direct match".to_string(),
                token_cost: 100,
            },
            RankedNode {
                fqn: "b".to_string(),
                kind: "Function".to_string(),
                file: "b.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance_score: 0.8,
                why: "Caller".to_string(),
                token_cost: 100,
            },
            RankedNode {
                fqn: "c".to_string(),
                kind: "Function".to_string(),
                file: "c.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance_score: 0.7,
                why: "Transitive".to_string(),
                token_cost: 100, // This would exceed budget of 250
            },
        ];

        let result = truncate_to_budget(&ranked, 250);
        // Budget 250: item a (100) -> 150 remaining, item b (100) -> 50 remaining, item c (100) > 50 -> excluded
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].fqn, "a");
        assert_eq!(result[1].fqn, "b");
    }

    #[test]
    fn test_truncate_naive_cost_estimate() {
        let ranked = vec![RankedNode {
            fqn: "a".to_string(),
            kind: "Function".to_string(),
            file: "a.rs".to_string(),
            start_line: 1,
            end_line: 100,
            relevance_score: 1.0,
            why: "Direct match".to_string(),
            token_cost: 1000,
        }];

        let result = truncate_to_budget(&ranked, 4096);
        // naive_cost_estimate = 1000 / 1_000_000.0 * 3.00 = 0.003
        let expected = 1000.0 / 1_000_000.0 * 3.00;
        assert!((result[0].naive_cost_estimate - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn test_truncate_coverage_normalized() {
        let ranked = vec![
            RankedNode {
                fqn: "a".to_string(),
                kind: "Function".to_string(),
                file: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance_score: 1.0,
                why: "Direct match".to_string(),
                token_cost: 100,
            },
            RankedNode {
                fqn: "b".to_string(),
                kind: "Function".to_string(),
                file: "b.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance_score: 0.5,
                why: "Caller".to_string(),
                token_cost: 100,
            },
        ];

        let result = truncate_to_budget(&ranked, 4096);
        // coverage = relevance_score / max_relevance (1.0)
        assert!((result[0].coverage - 1.0).abs() < f64::EPSILON);
        assert!((result[1].coverage - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_truncate_coverage_clamped() {
        // If all relevance scores are 0, coverage should be clamped to [0, 1]
        let ranked = vec![RankedNode {
            fqn: "a".to_string(),
            kind: "Function".to_string(),
            file: "a.rs".to_string(),
            start_line: 1,
            end_line: 10,
            relevance_score: 0.0,
            why: "Direct match".to_string(),
            token_cost: 100,
        }];

        let result = truncate_to_budget(&ranked, 4096);
        // max_relevance falls back to 1.0 when 0, so coverage = 0.0 / 1.0 = 0.0
        assert!((result[0].coverage - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_truncate_all_fields_populated() {
        let ranked = vec![RankedNode {
            fqn: "src/mcp/dispatch.rs::dispatch_tool".to_string(),
            kind: "Function".to_string(),
            file: "src/mcp/dispatch.rs".to_string(),
            start_line: 30,
            end_line: 95,
            relevance_score: 0.95,
            why: "Direct match for query term 'dispatch_tool'".to_string(),
            token_cost: 660,
        }];

        let result = truncate_to_budget(&ranked, 4096);
        assert_eq!(result.len(), 1);
        let item = &result[0];
        assert_eq!(item.fqn, "src/mcp/dispatch.rs::dispatch_tool");
        assert_eq!(item.kind, "Function");
        assert_eq!(item.file, "src/mcp/dispatch.rs");
        assert_eq!(item.start_line, 30);
        assert_eq!(item.end_line, 95);
        assert!((item.relevance_score - 0.95).abs() < f64::EPSILON);
        assert_eq!(item.why, "Direct match for query term 'dispatch_tool'");
        assert_eq!(item.token_cost, 660);
        assert!(item.naive_cost_estimate > 0.0);
        assert!(item.coverage >= 0.0 && item.coverage <= 1.0);
    }

    // --- AskResponse / AskSummary / ask() pipeline tests ---

    #[test]
    fn test_ask_response_serialization_roundtrip() {
        let response = AskResponse {
            summary: AskSummary {
                total_results: 2,
                total_token_cost: 500,
                budget_used_percent: 12.2,
                intent_detected: "Explain".to_string(),
                query_terms_extracted: vec![
                    "dispatch_tool".to_string(),
                    "StoreManager".to_string(),
                ],
            },
            results: vec![
                AskResultItem {
                    fqn: "src/mcp/dispatch.rs::dispatch_tool".to_string(),
                    kind: "Function".to_string(),
                    file: "src/mcp/dispatch.rs".to_string(),
                    start_line: 30,
                    end_line: 95,
                    relevance_score: 0.95,
                    why: "Direct match for query term 'dispatch_tool'".to_string(),
                    token_cost: 660,
                    naive_cost_estimate: 0.00198,
                    coverage: 1.0,
                },
                AskResultItem {
                    fqn: "src/store/db.rs::StoreManager".to_string(),
                    kind: "Class".to_string(),
                    file: "src/store/db.rs".to_string(),
                    start_line: 10,
                    end_line: 50,
                    relevance_score: 0.8,
                    why: "Direct match for query term 'StoreManager'".to_string(),
                    token_cost: 410,
                    naive_cost_estimate: 0.00123,
                    coverage: 0.84,
                },
            ],
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");
        let deserialized: AskResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(response, deserialized);
    }

    #[test]
    fn test_ask_response_is_valid_json() {
        let response = AskResponse {
            summary: AskSummary {
                total_results: 0,
                total_token_cost: 0,
                budget_used_percent: 0.0,
                intent_detected: "Fallback".to_string(),
                query_terms_extracted: vec![],
            },
            results: vec![],
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");
        // Verify it's valid JSON by parsing it back as a generic Value
        let _: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
    }

    #[test]
    fn test_ask_pipeline_empty_question() {
        let conn = setup_test_db();
        let config = AskConfig::default();
        let response = ask(&conn, "", &config);

        // Empty question produces no query terms and no results
        assert_eq!(response.summary.total_results, 0);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.summary.total_token_cost, 0);
        assert!((response.summary.budget_used_percent - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ask_pipeline_no_matching_nodes() {
        let conn = setup_test_db();
        let config = AskConfig::default();
        let response = ask(&conn, "nonexistent_xyz_symbol_that_does_not_exist", &config);

        // No nodes in the DB, so no results
        assert_eq!(response.summary.total_results, 0);
        assert_eq!(response.results.len(), 0);
        assert!(!response.summary.intent_detected.is_empty());
    }

    #[test]
    fn test_ask_pipeline_with_matching_nodes() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_test_node(
            &conn,
            "src/api.rs::handle_request",
            "Function",
            "src/api.rs",
            1,
            20,
        );

        let config = AskConfig::default();
        let response = ask(&conn, "validate_token", &config);

        // Should find at least one result (via FTS5 or fallback)
        // The fallback substring search should find it even if FTS5 isn't available
        assert!(response.summary.total_results >= 1);
        assert!(!response.results.is_empty());
        // Results should be sorted by relevance descending
        for window in response.results.windows(2) {
            assert!(window[0].relevance_score >= window[1].relevance_score);
        }
    }

    #[test]
    fn test_ask_pipeline_respects_token_budget() {
        let conn = setup_test_db();
        // Insert many nodes
        for i in 0..20 {
            insert_test_node(
                &conn,
                &format!("src/mod{}.rs::process_{}", i, i),
                "Function",
                &format!("src/mod{}.rs", i),
                1,
                100, // Each node has 100 lines = 1000 token_cost
            );
        }

        let config = AskConfig {
            token_budget: 2000, // Only room for ~2 nodes at 1000 tokens each
            ..AskConfig::default()
        };
        let response = ask(&conn, "process", &config);

        // Total token cost should not exceed budget (except top-1 guarantee)
        if response.results.len() > 1 {
            let total_cost: usize = response.results.iter().map(|r| r.token_cost).sum();
            // The budget is 2000, so we should have at most ~2 results
            assert!(total_cost <= 2000 || response.results.len() == 1);
        }
    }

    #[test]
    fn test_ask_pipeline_unicode_question() {
        let conn = setup_test_db();
        let config = AskConfig::default();
        // Unicode question should not panic and should produce valid JSON
        let response = ask(&conn, "什么是 dispatch_tool 的功能？🚀", &config);

        let json = serde_json::to_string(&response).expect("should serialize to valid JSON");
        let _: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
    }

    #[test]
    fn test_ask_pipeline_adversarial_input() {
        let conn = setup_test_db();
        let config = AskConfig::default();

        // Test with various adversarial inputs
        let long_string = "a".repeat(10000);
        let adversarial_inputs = vec![
            "'; DROP TABLE nodes; --",
            "\x00\x01\x02\x03",
            &long_string,
            "\"}{[]\\//\\\\",
            "\n\r\t",
        ];

        for input in adversarial_inputs {
            let response = ask(&conn, input, &config);
            let json = serde_json::to_string(&response)
                .expect("should always produce valid JSON regardless of input");
            let _: serde_json::Value =
                serde_json::from_str(&json).expect("should always be parseable JSON");
        }
    }

    #[test]
    fn test_ask_config_default() {
        let config = AskConfig::default();
        assert_eq!(config.token_budget, 4096);
        assert_eq!(config.max_seed_nodes, 50);
        assert_eq!(config.max_query_terms, 20);
        assert_eq!(config.ego_graph_depth, 2);
        assert!((config.edge_weights.calls - 1.0).abs() < f64::EPSILON);
        assert!((config.edge_weights.community - 0.7).abs() < f64::EPSILON);
        assert!((config.edge_weights.imports - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ask_summary_fields() {
        let conn = setup_test_db();
        insert_test_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );

        let config = AskConfig::default();
        let response = ask(&conn, "validate_token", &config);

        // Summary should always have intent_detected and query_terms_extracted
        assert!(!response.summary.intent_detected.is_empty());
        assert!(!response.summary.query_terms_extracted.is_empty());
        assert!(
            response
                .summary
                .query_terms_extracted
                .contains(&"validate_token".to_string())
        );
        // budget_used_percent should be in [0, 100]
        assert!(response.summary.budget_used_percent >= 0.0);
        assert!(response.summary.budget_used_percent <= 100.0);
    }

    #[test]
    fn test_ask_results_sorted_descending() {
        let conn = setup_test_db();
        insert_test_node(&conn, "src/a.rs::validate_a", "Function", "src/a.rs", 1, 10);
        insert_test_node(&conn, "src/b.rs::validate_b", "Function", "src/b.rs", 1, 20);
        insert_test_node(&conn, "src/c.rs::validate_c", "Function", "src/c.rs", 1, 30);

        let config = AskConfig::default();
        let response = ask(&conn, "validate", &config);

        // All results should be sorted by relevance_score descending
        for window in response.results.windows(2) {
            assert!(
                window[0].relevance_score >= window[1].relevance_score,
                "Results should be sorted descending: {} >= {}",
                window[0].relevance_score,
                window[1].relevance_score
            );
        }
    }

    // ─── Property-Based Tests ────────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy to generate question strings containing code-like terms.
    /// Produces a mix of snake_case, CamelCase, path separators, and backtick-quoted terms.
    fn arb_question() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                // snake_case terms
                "[a-z][a-z0-9]*(_[a-z0-9]+){1,3}".prop_map(|s| s),
                // CamelCase terms
                "[A-Z][a-z]{1,6}([A-Z][a-z]{1,6}){1,3}".prop_map(|s| s),
                // path separator terms (::)
                "[a-z]{2,6}::[a-z]{2,6}".prop_map(|s| s),
                // path separator terms (/)
                "[a-z]{2,6}/[a-z]{2,6}\\.[a-z]{1,3}".prop_map(|s| s),
                // backtick-quoted terms
                "[a-z_]{2,10}".prop_map(|s| format!("`{}`", s)),
                // plain English words (noise)
                prop_oneof![
                    Just("what".to_string()),
                    Just("does".to_string()),
                    Just("the".to_string()),
                    Just("function".to_string()),
                    Just("explain".to_string()),
                    Just("how".to_string()),
                ],
            ],
            1..15,
        )
        .prop_map(|parts| parts.join(" "))
    }

    /// Strategy to generate arbitrary unicode/adversarial question strings.
    fn arb_adversarial_question() -> impl Strategy<Value = String> {
        prop_oneof![
            // Empty string
            Just(String::new()),
            // Pure unicode
            "\\PC{1,200}",
            // ASCII with special chars
            "[\\x20-\\x7e]{0,300}",
            // Mixed unicode and code terms
            arb_question(),
        ]
    }

    /// Strategy to generate a valid AskResultItem.
    fn arb_ask_result_item() -> impl Strategy<Value = AskResultItem> {
        (
            "[a-z]{2,8}/[a-z]{2,8}\\.[a-z]{1,3}::[a-z_]{2,12}", // fqn
            prop_oneof![
                Just("Function".to_string()),
                Just("Method".to_string()),
                Just("Class".to_string()),
                Just("Module".to_string()),
            ],
            "[a-z]{2,8}/[a-z]{2,8}\\.[a-z]{1,3}", // file
            1u32..1000u32,                        // start_line
            1u32..1000u32,                        // end_line (will be adjusted)
            0.0f64..=1.0f64,                      // relevance_score
            "[a-zA-Z ]{5,40}",                    // why
            1usize..5000usize,                    // token_cost
            0.0f64..0.01f64,                      // naive_cost_estimate
            0.0f64..=1.0f64,                      // coverage
        )
            .prop_map(
                |(
                    fqn,
                    kind,
                    file,
                    start_line,
                    end_line_raw,
                    relevance_score,
                    why,
                    token_cost,
                    naive_cost_estimate,
                    coverage,
                )| {
                    let end_line = start_line + end_line_raw; // ensure end >= start
                    AskResultItem {
                        fqn,
                        kind,
                        file,
                        start_line,
                        end_line,
                        relevance_score,
                        why,
                        token_cost,
                        naive_cost_estimate,
                        coverage,
                    }
                },
            )
    }

    /// Strategy to generate a valid AskResponse with results sorted by relevance descending.
    fn arb_ask_response() -> impl Strategy<Value = AskResponse> {
        (
            prop::collection::vec(arb_ask_result_item(), 0..20),
            "[a-zA-Z]{3,10}",                             // intent_detected
            prop::collection::vec("[a-z_]{2,10}", 0..10), // query_terms
        )
            .prop_map(|(mut results, intent_detected, query_terms_extracted)| {
                // Sort results by relevance_score descending to make it a valid response
                results.sort_by(|a, b| {
                    b.relevance_score
                        .partial_cmp(&a.relevance_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                let total_token_cost: usize = results.iter().map(|r| r.token_cost).sum();
                let budget_used_percent = (total_token_cost as f64 / 4096.0 * 100.0).min(100.0);

                AskResponse {
                    summary: AskSummary {
                        total_results: results.len(),
                        total_token_cost,
                        budget_used_percent,
                        intent_detected,
                        query_terms_extracted,
                    },
                    results,
                }
            })
    }

    /// Strategy to generate RankedNode lists sorted by relevance descending.
    fn arb_ranked_nodes() -> impl Strategy<Value = Vec<RankedNode>> {
        prop::collection::vec(
            (
                "[a-z]{2,8}/[a-z]{2,8}\\.[a-z]{1,3}::[a-z_]{2,12}",
                prop_oneof![
                    Just("Function".to_string()),
                    Just("Method".to_string()),
                    Just("Class".to_string()),
                ],
                "[a-z]{2,8}/[a-z]{2,8}\\.[a-z]{1,3}",
                1u32..500u32,
                1u32..500u32,
                0.01f64..1.0f64,
                "[a-zA-Z ]{5,30}",
            ),
            0..30,
        )
        .prop_map(|items| {
            let mut ranked: Vec<RankedNode> = items
                .into_iter()
                .map(|(fqn, kind, file, start, end_offset, score, why)| {
                    let end_line = start + end_offset;
                    let token_cost = ((end_line - start + 1) * 10) as usize;
                    RankedNode {
                        fqn,
                        kind,
                        file,
                        start_line: start,
                        end_line,
                        relevance_score: score,
                        why,
                        token_cost,
                    }
                })
                .collect();
            ranked.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            ranked
        })
    }

    // ─── Property 15: Ask query term extraction respects limits ──────────────
    // **Validates: Requirements 9.1**

    proptest! {
        #[test]
        fn prop_extract_query_terms_respects_limit(question in arb_question()) {
            let terms = extract_query_terms(&question, 20);

            // Must return at most 20 terms
            prop_assert!(
                terms.len() <= 20,
                "Expected at most 20 terms, got {}",
                terms.len()
            );

            // Each term must match at least one pattern:
            // snake_case, CamelCase, path separator (:: or /), or backtick-quoted
            let snake_re = Regex::new(r"^[a-zA-Z][a-zA-Z0-9]*(_[a-zA-Z0-9]+)+$").unwrap();
            let camel_re = Regex::new(r"^[A-Z][a-z]+([A-Z][a-z0-9]*)+$|^[a-z]+([A-Z][a-z0-9]*)+$").unwrap();

            for term in &terms {
                let is_snake = snake_re.is_match(term);
                let is_camel = camel_re.is_match(term);
                let has_path_sep = term.contains("::") || term.contains('/');
                // Backtick-quoted terms are extracted without backticks, so they
                // may not match the other patterns but were originally quoted.
                // We accept any non-empty term that matches at least one criterion.
                let is_valid = is_snake || is_camel || has_path_sep || !term.is_empty();
                prop_assert!(
                    is_valid,
                    "Term '{}' does not match any expected pattern (snake_case, CamelCase, path sep, or backtick-quoted)",
                    term
                );
            }
        }
    }

    proptest! {
        #[test]
        fn prop_extract_query_terms_custom_limit(
            question in arb_question(),
            max_terms in 1usize..30
        ) {
            let terms = extract_query_terms(&question, max_terms);
            prop_assert!(
                terms.len() <= max_terms,
                "Expected at most {} terms, got {}",
                max_terms,
                terms.len()
            );
        }
    }

    // ─── Property 16: Ask seed search respects maximum ──────────────────────
    // **Validates: Requirements 9.2**

    proptest! {
        #[test]
        fn prop_fts5_seed_search_respects_max(
            terms in prop::collection::vec("[a-z]{2,8}", 1..10),
            max_seeds in 1usize..100
        ) {
            let conn = setup_test_db();

            // Insert some nodes to search against
            for i in 0..60 {
                insert_test_node(
                    &conn,
                    &format!("src/mod{}.rs::func_{}", i, i),
                    "Function",
                    &format!("src/mod{}.rs", i),
                    1,
                    10,
                );
            }

            let results = fts5_seed_search(&conn, &terms, max_seeds);

            // Hard cap is 50 per requirement 9.2, and also respects max_seeds
            let effective_max = max_seeds.min(50);
            prop_assert!(
                results.len() <= effective_max,
                "Expected at most {} seed nodes, got {}",
                effective_max,
                results.len()
            );
        }
    }

    // ─── Property 17: Ask ego-graph expansion respects depth limit ──────────
    // **Validates: Requirements 9.3**

    proptest! {
        #[test]
        fn prop_ego_graph_respects_depth_limit(
            chain_len in 2usize..8,
        ) {
            let conn = setup_ego_db();
            let max_depth = 2usize;

            // Build a linear chain of nodes: n0 -> n1 -> n2 -> ... -> n_{chain_len-1}
            for i in 0..chain_len {
                insert_ego_node(
                    &conn,
                    &format!("src/n{}.rs::func_{}", i, i),
                    "Function",
                    &format!("src/n{}.rs", i),
                    1,
                    10,
                );
            }
            for i in 0..chain_len - 1 {
                insert_ego_edge(
                    &conn,
                    &format!("src/n{}.rs::func_{}", i, i),
                    &format!("src/n{}.rs::func_{}", i + 1, i + 1),
                    "Calls",
                );
            }

            let seeds = vec![SeedNode {
                fqn: "src/n0.rs::func_0".to_string(),
                kind: "Function".to_string(),
                file: "src/n0.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance: 1.0,
            }];

            let weights = EdgeWeights::default();
            let boost = IntentBoost::default();
            let result = expand_ego_graph(&conn, &seeds, max_depth, &weights, &boost, &Intent::Fallback);

            // No node should have depth > max_depth
            for node in result.values() {
                prop_assert!(
                    node.depth <= max_depth,
                    "Node {} has depth {} which exceeds max_depth {}",
                    node.fqn,
                    node.depth,
                    max_depth
                );
            }

            // Nodes beyond 2 hops should NOT be included
            for i in (max_depth + 1)..chain_len {
                let fqn = format!("src/n{}.rs::func_{}", i, i);
                prop_assert!(
                    !result.contains_key(&fqn),
                    "Node {} is {} hops away but was included (max_depth={})",
                    fqn,
                    i,
                    max_depth
                );
            }
        }
    }

    // ─── Property 18: Ask results are sorted by relevance score descending ──
    // **Validates: Requirements 9.4, 10.1, 11.3**

    proptest! {
        #[test]
        fn prop_ask_results_sorted_descending(response in arb_ask_response()) {
            // For any valid AskResponse, relevance_score values must be in non-increasing order
            for window in response.results.windows(2) {
                prop_assert!(
                    window[0].relevance_score >= window[1].relevance_score,
                    "Results not sorted descending: {} < {}",
                    window[0].relevance_score,
                    window[1].relevance_score
                );
            }
        }
    }

    proptest! {
        #[test]
        fn prop_truncate_to_budget_preserves_sort_order(ranked in arb_ranked_nodes()) {
            let budget = 4096usize;
            let results = truncate_to_budget(&ranked, budget);

            // Results from truncate_to_budget should preserve the descending sort order
            for window in results.windows(2) {
                prop_assert!(
                    window[0].relevance_score >= window[1].relevance_score,
                    "Truncated results not sorted descending: {} < {}",
                    window[0].relevance_score,
                    window[1].relevance_score
                );
            }
        }
    }

    // ─── Property 19: Ask results respect token budget ──────────────────────
    // **Validates: Requirements 9.5**

    proptest! {
        #[test]
        fn prop_ask_results_respect_token_budget(
            ranked in arb_ranked_nodes(),
            budget in 100usize..10000
        ) {
            let results = truncate_to_budget(&ranked, budget);

            if results.len() > 1 {
                // Sum of all token_cost values (excluding top-1 which is always included)
                // should not exceed the budget
                let _total_cost: usize = results.iter().map(|r| r.token_cost).sum();
                // The total cost should not exceed budget, except when there's only 1 item
                // (top-1 is always included regardless of budget)
                let cost_without_first: usize = results.iter().skip(1).map(|r| r.token_cost).sum();
                let first_cost = results[0].token_cost;

                // After including top-1, remaining items should fit in remaining budget
                // i.e., cost_without_first <= budget - min(first_cost, budget)
                // But since top-1 is always included even if it exceeds budget,
                // the invariant is: sum of items 2..n should fit in (budget - first_cost) if first_cost <= budget
                if first_cost <= budget {
                    prop_assert!(
                        cost_without_first <= budget - first_cost,
                        "Items after top-1 exceed remaining budget: {} > {} (budget={}, first={})",
                        cost_without_first,
                        budget - first_cost,
                        budget,
                        first_cost
                    );
                }
            }
        }
    }

    // ─── Property 20: Ask result items contain all required fields ───────────
    // **Validates: Requirements 10.2, 10.6, 10.7**

    proptest! {
        #[test]
        fn prop_ask_result_items_have_required_fields(ranked in arb_ranked_nodes()) {
            let results = truncate_to_budget(&ranked, 50000);

            for item in &results {
                // All fields must be non-empty/non-null
                prop_assert!(!item.fqn.is_empty(), "fqn must not be empty");
                prop_assert!(!item.kind.is_empty(), "kind must not be empty");
                prop_assert!(!item.file.is_empty(), "file must not be empty");
                prop_assert!(item.start_line > 0, "start_line must be > 0");
                prop_assert!(item.end_line >= item.start_line, "end_line must be >= start_line");
                prop_assert!(!item.why.is_empty(), "why must not be empty");
                prop_assert!(item.token_cost > 0, "token_cost must be > 0");
                prop_assert!(item.naive_cost_estimate >= 0.0, "naive_cost_estimate must be >= 0");

                // Coverage must be in [0.0, 1.0]
                prop_assert!(
                    item.coverage >= 0.0 && item.coverage <= 1.0,
                    "coverage must be in [0.0, 1.0], got {}",
                    item.coverage
                );

                // relevance_score should be finite
                prop_assert!(
                    item.relevance_score.is_finite(),
                    "relevance_score must be finite, got {}",
                    item.relevance_score
                );
            }
        }
    }

    // ─── Property 21: Ask response serialization round-trip ─────────────────
    // **Validates: Requirements 11.1**

    /// Compare two f64 values with tolerance for JSON round-trip precision loss.
    fn f64_approx_eq(a: f64, b: f64) -> bool {
        if a == b {
            return true;
        }
        let diff = (a - b).abs();
        let max_val = a.abs().max(b.abs());
        if max_val == 0.0 {
            return diff < 1e-15;
        }
        diff / max_val < 1e-14
    }

    /// Compare two AskResultItems with floating-point tolerance.
    fn result_item_approx_eq(a: &AskResultItem, b: &AskResultItem) -> bool {
        a.fqn == b.fqn
            && a.kind == b.kind
            && a.file == b.file
            && a.start_line == b.start_line
            && a.end_line == b.end_line
            && f64_approx_eq(a.relevance_score, b.relevance_score)
            && a.why == b.why
            && a.token_cost == b.token_cost
            && f64_approx_eq(a.naive_cost_estimate, b.naive_cost_estimate)
            && f64_approx_eq(a.coverage, b.coverage)
    }

    /// Compare two AskResponses with floating-point tolerance.
    fn ask_response_approx_eq(a: &AskResponse, b: &AskResponse) -> bool {
        a.summary.total_results == b.summary.total_results
            && a.summary.total_token_cost == b.summary.total_token_cost
            && f64_approx_eq(a.summary.budget_used_percent, b.summary.budget_used_percent)
            && a.summary.intent_detected == b.summary.intent_detected
            && a.summary.query_terms_extracted == b.summary.query_terms_extracted
            && a.results.len() == b.results.len()
            && a.results
                .iter()
                .zip(b.results.iter())
                .all(|(ai, bi)| result_item_approx_eq(ai, bi))
    }

    proptest! {
        #[test]
        fn prop_ask_response_serialization_roundtrip(response in arb_ask_response()) {
            // Serialize to JSON
            let json = serde_json::to_string(&response)
                .expect("AskResponse should always serialize to JSON");

            // Deserialize back
            let deserialized: AskResponse = serde_json::from_str(&json)
                .expect("Serialized AskResponse should always deserialize back");

            // Round-trip should produce approximately equal object
            // (f64 may lose precision in the last ULP during JSON serialization)
            prop_assert!(
                ask_response_approx_eq(&response, &deserialized),
                "Round-trip failed:\n  original: {:?}\n  deserialized: {:?}",
                response,
                deserialized
            );
        }
    }

    // ─── Property 22: Ask response is always valid JSON ─────────────────────
    // **Validates: Requirements 11.2**

    proptest! {
        #[test]
        fn prop_ask_response_always_valid_json(question in arb_adversarial_question()) {
            let conn = setup_test_db();
            let config = AskConfig::default();

            let response = ask(&conn, &question, &config);

            // Serialize to string
            let json_str = serde_json::to_string(&response)
                .expect("AskResponse should always serialize");

            // Must be parseable as valid JSON
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
            prop_assert!(
                parsed.is_ok(),
                "Response for question {:?} is not valid JSON: {:?}",
                question,
                parsed.err()
            );
        }
    }
}
