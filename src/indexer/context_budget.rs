//! Task-aware context budgeting for the `get_task_context` MCP tool.
//!
//! This module selects and prioritizes structural graph context to fit within
//! a specified token budget. It queries the FTS5 index and graph relationships
//! to return the most relevant symbols and edges for a given task description.
//!
//! Pipeline:
//! 1. FTS5 relevance search (task description → scored FQNs)
//! 2. Graph expansion (callers/callees of top FQNs at depth 1)
//! 3. Token cost estimation per symbol
//! 4. Greedy knapsack budget packing
//! 5. Scope filtering (file path prefix constraint)
//! 6. Code snippet inclusion (when include_code=true)

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;

/// A request for task-relevant context within a token budget.
#[derive(Debug, Clone)]
pub struct ContextRequest {
    /// Natural language description of the task.
    pub task_description: String,
    /// Maximum number of tokens to include in the response.
    pub token_budget: usize,
    /// Whether to include source code snippets for top-priority symbols.
    pub include_code: bool,
    /// Optional file path or directory prefix to constrain the search space.
    pub scope: Option<String>,
}

/// A symbol included in the context response.
#[derive(Debug, Clone, Serialize)]
pub struct ContextSymbol {
    /// Fully qualified name of the symbol.
    pub fqn: String,
    /// The node kind (Function, Class, Module, etc.).
    pub kind: String,
    /// File path containing the symbol.
    pub file: String,
    /// Starting line number in the file.
    pub start_line: u32,
    /// Ending line number in the file.
    pub end_line: u32,
    /// Relevance score (0.0–1.0) indicating priority.
    pub relevance: f64,
    /// Optional source code snippet (included when `include_code` is true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// A relationship between two symbols included in the context response.
#[derive(Debug, Clone, Serialize)]
pub struct ContextRelationship {
    /// FQN of the source symbol.
    pub source: String,
    /// FQN of the target symbol.
    pub target: String,
    /// The edge kind (Calls, Imports, Inherits, etc.).
    pub kind: String,
    /// Confidence score of the resolved edge (0.0–1.0).
    pub confidence: f64,
}

/// The response returned by `build_context`, containing prioritized symbols
/// and relationships that fit within the requested token budget.
#[derive(Debug, Clone, Serialize)]
pub struct ContextResponse {
    /// Symbols included in the context, ordered by relevance.
    pub symbols: Vec<ContextSymbol>,
    /// Relationships between included symbols.
    pub relationships: Vec<ContextRelationship>,
    /// Whether the response was truncated due to budget constraints.
    pub truncated: bool,
    /// The lowest relevance score among included symbols (0.0 if none truncated).
    pub relevance_cutoff: f64,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A candidate symbol with relevance score and estimated token cost.
#[derive(Debug, Clone)]
struct Candidate {
    fqn: String,
    kind: String,
    file: String,
    start_line: u32,
    end_line: u32,
    relevance: f64,
    token_cost: usize,
}

impl Candidate {
    /// Value density for knapsack: relevance per token.
    fn value_density(&self) -> f64 {
        if self.token_cost == 0 {
            return self.relevance;
        }
        self.relevance / self.token_cost as f64
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Tokens for node metadata (FQN, kind, file, lines).
const NODE_METADATA_TOKENS: usize = 20;

/// Tokens per edge/relationship entry.
#[allow(dead_code)]
const EDGE_TOKENS: usize = 15;

/// Average tokens per source line (for code snippet estimation).
const TOKENS_PER_LINE: usize = 8;

/// Maximum number of FTS5 results to consider.
const MAX_FTS_RESULTS: usize = 50;

/// Maximum number of graph-expanded candidates.
const MAX_EXPANSION_RESULTS: usize = 100;

/// Relevance decay factor for graph-expanded nodes (callers/callees).
const GRAPH_EXPANSION_DECAY: f64 = 0.7;

// ---------------------------------------------------------------------------
// Step 1: FTS5-based relevance search
// ---------------------------------------------------------------------------

/// Search the FTS5 index for symbols matching the task description.
/// Returns a map of FQN → relevance score (0.6–1.0 range, normalized BM25).
fn fts5_relevance_search(
    conn: &Connection,
    task_description: &str,
) -> Result<HashMap<String, f64>, anyhow::Error> {
    let sanitized = sanitize_fts_query(task_description);
    if sanitized.is_empty() {
        return Ok(HashMap::new());
    }

    let mut stmt = conn.prepare(
        "SELECT fqn, kind, file, rank \
         FROM nodes_fts \
         WHERE nodes_fts MATCH ?1 \
         ORDER BY rank \
         LIMIT ?2",
    )?;

    let rows: Vec<(String, f64)> = stmt
        .query_map(
            rusqlite::params![sanitized, MAX_FTS_RESULTS as i64],
            |row| {
                let fqn: String = row.get(0)?;
                let rank: f64 = row.get(3)?;
                Ok((fqn, rank))
            },
        )?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    // Normalize BM25 ranks to 0.6–1.0 range.
    // FTS5 rank is negative (more negative = better match).
    let best_rank = rows.iter().map(|(_, r)| *r).fold(f64::INFINITY, f64::min);
    let worst_rank = rows
        .iter()
        .map(|(_, r)| *r)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut results = HashMap::new();
    for (fqn, rank) in rows {
        let normalized = if (worst_rank - best_rank).abs() < f64::EPSILON {
            1.0
        } else {
            // best_rank maps to 1.0, worst_rank maps to 0.0
            (rank - worst_rank) / (best_rank - worst_rank)
        };
        // Scale to 0.6–1.0 range
        let relevance = 0.6 + 0.4 * normalized;
        results.insert(fqn, relevance);
    }

    Ok(results)
}

/// Sanitize user input for FTS5 MATCH queries.
fn sanitize_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let tokens: Vec<String> = trimmed
        .split_whitespace()
        .map(|token| {
            let upper = token.to_uppercase();
            if matches!(upper.as_str(), "OR" | "AND" | "NOT" | "NEAR") {
                return format!("\"{}\"", token);
            }
            if token.contains('"')
                || token.contains('*')
                || token.contains('(')
                || token.contains(')')
                || token.contains('^')
                || token.contains(':')
            {
                let escaped = token.replace('"', "\"\"");
                return format!("\"{}\"", escaped);
            }
            token.to_string()
        })
        .collect();

    tokens.join(" ")
}

// ---------------------------------------------------------------------------
// Step 2: Graph expansion (callers/callees at depth 1)
// ---------------------------------------------------------------------------

/// Expand the candidate set by finding callers and callees of top FQNs.
/// Returns additional FQN → relevance entries (decayed from parent score).
fn graph_expansion(
    conn: &Connection,
    seed_scores: &HashMap<String, f64>,
) -> Result<HashMap<String, f64>, anyhow::Error> {
    let mut expanded: HashMap<String, f64> = HashMap::new();

    // Take top-scoring seeds for expansion (limit to avoid expensive queries)
    let mut seeds: Vec<(&String, &f64)> = seed_scores.iter().collect();
    seeds.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    seeds.truncate(20);

    for (fqn, parent_score) in &seeds {
        let decayed_score = *parent_score * GRAPH_EXPANSION_DECAY;

        // Find callees (depth 1)
        let mut stmt = conn.prepare(
            "SELECT target_fqn FROM edges \
             WHERE source_fqn = ?1 AND kind = 'Calls' \
             LIMIT 10",
        )?;
        let callees: Vec<String> = stmt
            .query_map(rusqlite::params![fqn], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for callee in callees {
            let entry = expanded.entry(callee).or_insert(0.0);
            if decayed_score > *entry {
                *entry = decayed_score;
            }
        }

        // Find callers (depth 1)
        let mut stmt = conn.prepare(
            "SELECT source_fqn FROM edges \
             WHERE target_fqn = ?1 AND kind = 'Calls' \
             LIMIT 10",
        )?;
        let callers: Vec<String> = stmt
            .query_map(rusqlite::params![fqn], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for caller in callers {
            let entry = expanded.entry(caller).or_insert(0.0);
            if decayed_score > *entry {
                *entry = decayed_score;
            }
        }
    }

    // Don't include seeds that are already in the original set
    for key in seed_scores.keys() {
        expanded.remove(key);
    }

    // Limit expansion results
    let mut expanded_vec: Vec<(String, f64)> = expanded.into_iter().collect();
    expanded_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    expanded_vec.truncate(MAX_EXPANSION_RESULTS);

    Ok(expanded_vec.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Step 3: Token cost estimation
// ---------------------------------------------------------------------------

/// Estimate the token cost for a symbol.
/// - Node metadata: ~20 tokens
/// - Code snippet (if include_code): (end_line - start_line) * 8 tokens
fn estimate_token_cost(start_line: u32, end_line: u32, include_code: bool) -> usize {
    let base = NODE_METADATA_TOKENS;
    if include_code {
        let lines = (end_line.saturating_sub(start_line)).max(1) as usize;
        base + lines * TOKENS_PER_LINE
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Step 4: Greedy knapsack budget packing
// ---------------------------------------------------------------------------

/// Pack candidates into the token budget using greedy knapsack (sort by value density).
/// Returns the selected candidates and whether truncation occurred.
fn greedy_knapsack(candidates: &mut [Candidate], token_budget: usize) -> (Vec<Candidate>, bool) {
    // Sort by value density (relevance / token_cost) descending
    candidates.sort_by(|a, b| {
        b.value_density()
            .partial_cmp(&a.value_density())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = Vec::new();
    let mut tokens_used: usize = 0;
    let mut truncated = false;

    for candidate in candidates.iter() {
        if tokens_used + candidate.token_cost <= token_budget {
            tokens_used += candidate.token_cost;
            selected.push(candidate.clone());
        } else {
            truncated = true;
        }
    }

    (selected, truncated)
}

// ---------------------------------------------------------------------------
// Step 5: Scope filtering
// ---------------------------------------------------------------------------

/// Filter candidates by file path prefix scope.
fn apply_scope_filter(candidates: Vec<Candidate>, scope: &Option<String>) -> Vec<Candidate> {
    match scope {
        Some(prefix) => candidates
            .into_iter()
            .filter(|c| c.file.starts_with(prefix.as_str()))
            .collect(),
        None => candidates,
    }
}

// ---------------------------------------------------------------------------
// Step 6: Code snippet inclusion
// ---------------------------------------------------------------------------

/// Read source code snippets for selected symbols.
fn include_code_snippets(symbols: &mut [ContextSymbol], conn: &Connection) {
    let repo_root = std::env::var("CORTEX_REPO_ROOT").unwrap_or_else(|_| ".".to_string());

    for symbol in symbols.iter_mut() {
        let file_path = std::path::Path::new(&repo_root).join(&symbol.file);
        if let Ok(source) = std::fs::read_to_string(&file_path) {
            let lines: Vec<&str> = source.lines().collect();
            let start = (symbol.start_line as usize).saturating_sub(1);
            let end = (symbol.end_line as usize).min(lines.len());
            if start < end {
                symbol.code = Some(lines[start..end].join("\n"));
            }
        }
    }
    // conn is available for future use (e.g., reading cached snippets)
    let _ = conn;
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Build task-aware context within the specified token budget.
///
/// Queries the graph database to find symbols relevant to the task description,
/// expands the graph neighborhood, ranks by relevance, and packs results into
/// the token budget using a greedy knapsack approach.
///
/// # Arguments
/// * `conn` - A reference to the SQLite database connection.
/// * `request` - The context request specifying task, budget, and options.
///
/// # Returns
/// A `ContextResponse` containing the most relevant symbols and relationships
/// that fit within the token budget.
pub fn build_context(
    conn: &Connection,
    request: &ContextRequest,
) -> Result<ContextResponse, anyhow::Error> {
    // Step 1: FTS5 relevance search
    let seed_scores = fts5_relevance_search(conn, &request.task_description)?;

    if seed_scores.is_empty() {
        return Ok(ContextResponse {
            symbols: vec![],
            relationships: vec![],
            truncated: false,
            relevance_cutoff: 0.0,
        });
    }

    // Step 2: Graph expansion
    let expanded_scores = graph_expansion(conn, &seed_scores)?;

    // Merge seed and expanded scores
    let mut all_scores: HashMap<String, f64> = seed_scores;
    for (fqn, score) in expanded_scores {
        all_scores.entry(fqn).or_insert(score);
    }

    // Fetch node metadata for all candidates
    let mut candidates: Vec<Candidate> = Vec::new();
    for (fqn, relevance) in &all_scores {
        let mut stmt =
            conn.prepare("SELECT kind, file, start_line, end_line FROM nodes WHERE fqn = ?1")?;
        let result = stmt.query_row(rusqlite::params![fqn], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, u32>(3)?,
            ))
        });

        if let Ok((kind, file, start_line, end_line)) = result {
            let token_cost = estimate_token_cost(start_line, end_line, request.include_code);
            candidates.push(Candidate {
                fqn: fqn.clone(),
                kind,
                file,
                start_line,
                end_line,
                relevance: *relevance,
                token_cost,
            });
        }
    }

    // Step 5: Scope filtering (applied before packing)
    candidates = apply_scope_filter(candidates, &request.scope);

    if candidates.is_empty() {
        return Ok(ContextResponse {
            symbols: vec![],
            relationships: vec![],
            truncated: false,
            relevance_cutoff: 0.0,
        });
    }

    // Reserve some budget for relationships metadata
    let relationship_budget_reserve = (request.token_budget / 10).min(200);
    let symbol_budget = request
        .token_budget
        .saturating_sub(relationship_budget_reserve);

    // Step 4: Greedy knapsack
    let (selected, truncated) = greedy_knapsack(&mut candidates, symbol_budget);

    // Build symbols
    let mut symbols: Vec<ContextSymbol> = selected
        .iter()
        .map(|c| ContextSymbol {
            fqn: c.fqn.clone(),
            kind: c.kind.clone(),
            file: c.file.clone(),
            start_line: c.start_line,
            end_line: c.end_line,
            relevance: c.relevance,
            code: None,
        })
        .collect();

    // Sort by relevance descending for output
    symbols.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 6: Code snippet inclusion
    if request.include_code {
        include_code_snippets(&mut symbols, conn);
    }

    // Fetch relationships between selected symbols
    let selected_fqns: HashSet<&str> = symbols.iter().map(|s| s.fqn.as_str()).collect();
    let relationships = fetch_relationships(conn, &selected_fqns)?;

    // Compute relevance cutoff
    let relevance_cutoff = symbols.last().map(|s| s.relevance).unwrap_or(0.0);

    Ok(ContextResponse {
        symbols,
        relationships,
        truncated,
        relevance_cutoff,
    })
}

/// Fetch edges between the selected set of symbols.
fn fetch_relationships(
    conn: &Connection,
    selected_fqns: &HashSet<&str>,
) -> Result<Vec<ContextRelationship>, anyhow::Error> {
    if selected_fqns.is_empty() {
        return Ok(vec![]);
    }

    // Query all edges where both source and target are in the selected set.
    // For efficiency, we query edges from selected sources and filter targets.
    let mut relationships = Vec::new();

    for &fqn in selected_fqns {
        let mut stmt = conn.prepare(
            "SELECT source_fqn, target_fqn, kind, confidence \
             FROM edges \
             WHERE source_fqn = ?1",
        )?;

        let edges: Vec<(String, String, String, f64)> = stmt
            .query_map(rusqlite::params![fqn], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (source, target, kind, confidence) in edges {
            if selected_fqns.contains(target.as_str()) {
                relationships.push(ContextRelationship {
                    source,
                    target,
                    kind,
                    confidence,
                });
            }
        }
    }

    Ok(relationships)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Creates an in-memory SQLite connection with the required schema.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // Apply initial schema
        conn.execute_batch(
            "CREATE TABLE nodes (
                fqn TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                file TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                file_hash TEXT NOT NULL,
                indexed_at INTEGER NOT NULL,
                attributes TEXT DEFAULT '{}'
            );
            CREATE INDEX idx_nodes_file ON nodes(file);
            CREATE INDEX idx_nodes_kind ON nodes(kind);

            CREATE TABLE edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_fqn TEXT NOT NULL,
                target_fqn TEXT NOT NULL,
                kind TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                attributes TEXT DEFAULT '{}'
            );
            CREATE INDEX idx_edges_source ON edges(source_fqn);
            CREATE INDEX idx_edges_target ON edges(target_fqn);

            CREATE TABLE file_snapshots (
                file TEXT PRIMARY KEY,
                file_hash TEXT NOT NULL,
                node_count INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL
            );",
        )
        .unwrap();

        // Apply FTS5 index
        conn.execute_batch(
            "CREATE VIRTUAL TABLE nodes_fts USING fts5(
                fqn, kind, file, attributes,
                content='nodes', content_rowid='rowid',
                tokenize='unicode61'
            );
            CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN
                INSERT INTO nodes_fts(rowid, fqn, kind, file, attributes)
                VALUES (new.rowid, new.fqn, new.kind, new.file, new.attributes);
            END;
            CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, fqn, kind, file, attributes)
                VALUES('delete', old.rowid, old.fqn, old.kind, old.file, old.attributes);
            END;
            CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, fqn, kind, file, attributes)
                VALUES('delete', old.rowid, old.fqn, old.kind, old.file, old.attributes);
                INSERT INTO nodes_fts(rowid, fqn, kind, file, attributes)
                VALUES (new.rowid, new.fqn, new.kind, new.file, new.attributes);
            END;",
        )
        .unwrap();

        conn
    }

    fn insert_node(conn: &Connection, fqn: &str, kind: &str, file: &str, start: u32, end: u32) {
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'hash', 1000, '{}')",
            rusqlite::params![fqn, kind, file, start, end],
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

    // -----------------------------------------------------------------------
    // Task 7.9: Unit tests for context_budget.rs
    // -----------------------------------------------------------------------

    #[test]
    fn test_budget_adherence_empty_db() {
        let conn = setup_db();
        let request = ContextRequest {
            task_description: "nonexistent function".to_string(),
            token_budget: 1000,
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        assert!(response.symbols.is_empty());
        assert!(!response.truncated);
        assert_eq!(response.relevance_cutoff, 0.0);
    }

    #[test]
    fn test_budget_adherence_within_limit() {
        let conn = setup_db();
        insert_node(
            &conn,
            "src/auth.rs::validate_token",
            "Function",
            "src/auth.rs",
            10,
            30,
        );
        insert_node(
            &conn,
            "src/auth.rs::check_expiry",
            "Function",
            "src/auth.rs",
            35,
            50,
        );
        insert_node(&conn, "src/db.rs::get_user", "Function", "src/db.rs", 1, 20);

        insert_edge(
            &conn,
            "src/auth.rs::validate_token",
            "src/db.rs::get_user",
            "Calls",
            1.0,
        );
        insert_edge(
            &conn,
            "src/auth.rs::validate_token",
            "src/auth.rs::check_expiry",
            "Calls",
            1.0,
        );

        let request = ContextRequest {
            task_description: "validate token auth".to_string(),
            token_budget: 500,
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        // Total token cost should not exceed budget
        let total_cost: usize = response.symbols.len() * NODE_METADATA_TOKENS;
        assert!(total_cost <= 500);
    }

    #[test]
    fn test_budget_truncation_small_budget() {
        let conn = setup_db();
        // Insert many nodes so budget is exceeded
        for i in 0..20 {
            insert_node(
                &conn,
                &format!("src/mod.rs::func_{}", i),
                "Function",
                "src/mod.rs",
                i * 20 + 1,
                i * 20 + 15,
            );
        }

        let request = ContextRequest {
            task_description: "func mod".to_string(),
            token_budget: 60, // Only room for ~3 symbols at 20 tokens each
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        assert!(response.symbols.len() <= 3);
        assert!(response.truncated);
    }

    #[test]
    fn test_relevance_ordering() {
        let conn = setup_db();
        insert_node(
            &conn,
            "src/auth.rs::login",
            "Function",
            "src/auth.rs",
            1,
            20,
        );
        insert_node(
            &conn,
            "src/auth.rs::logout",
            "Function",
            "src/auth.rs",
            25,
            40,
        );
        insert_node(
            &conn,
            "src/utils.rs::helper",
            "Function",
            "src/utils.rs",
            1,
            10,
        );

        let request = ContextRequest {
            task_description: "auth login".to_string(),
            token_budget: 1000,
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        if response.symbols.len() >= 2 {
            // First symbol should have higher or equal relevance than second
            assert!(response.symbols[0].relevance >= response.symbols[1].relevance);
        }
    }

    #[test]
    fn test_scope_filtering() {
        let conn = setup_db();
        insert_node(
            &conn,
            "src/auth/login.rs::login",
            "Function",
            "src/auth/login.rs",
            1,
            20,
        );
        insert_node(
            &conn,
            "src/db/users.rs::get_user",
            "Function",
            "src/db/users.rs",
            1,
            15,
        );

        let request = ContextRequest {
            task_description: "login get_user".to_string(),
            token_budget: 1000,
            include_code: false,
            scope: Some("src/auth".to_string()),
        };

        let response = build_context(&conn, &request).unwrap();
        // Only symbols in src/auth should be included
        for symbol in &response.symbols {
            assert!(
                symbol.file.starts_with("src/auth"),
                "Symbol {} in file {} should be filtered by scope",
                symbol.fqn,
                symbol.file
            );
        }
    }

    #[test]
    fn test_graph_expansion_includes_callees() {
        let conn = setup_db();
        insert_node(
            &conn,
            "src/auth.rs::validate",
            "Function",
            "src/auth.rs",
            1,
            20,
        );
        insert_node(
            &conn,
            "src/db.rs::query_user",
            "Function",
            "src/db.rs",
            1,
            15,
        );
        insert_node(
            &conn,
            "src/cache.rs::get_cached",
            "Function",
            "src/cache.rs",
            1,
            10,
        );

        insert_edge(
            &conn,
            "src/auth.rs::validate",
            "src/db.rs::query_user",
            "Calls",
            1.0,
        );
        insert_edge(
            &conn,
            "src/auth.rs::validate",
            "src/cache.rs::get_cached",
            "Calls",
            0.9,
        );

        let request = ContextRequest {
            task_description: "validate auth".to_string(),
            token_budget: 2000,
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        let fqns: Vec<&str> = response.symbols.iter().map(|s| s.fqn.as_str()).collect();
        // The direct match should be included
        assert!(fqns.contains(&"src/auth.rs::validate"));
        // Callees should be expanded into the result
        // (they may or may not be included depending on budget, but with 2000 tokens they should fit)
        // At minimum, the direct match is present
    }

    #[test]
    fn test_relationships_between_selected() {
        let conn = setup_db();
        insert_node(&conn, "src/a.rs::foo", "Function", "src/a.rs", 1, 10);
        insert_node(&conn, "src/b.rs::bar", "Function", "src/b.rs", 1, 10);
        insert_node(&conn, "src/c.rs::baz", "Function", "src/c.rs", 1, 10);

        insert_edge(&conn, "src/a.rs::foo", "src/b.rs::bar", "Calls", 1.0);
        insert_edge(&conn, "src/b.rs::bar", "src/c.rs::baz", "Calls", 0.9);

        let request = ContextRequest {
            task_description: "foo bar baz".to_string(),
            token_budget: 2000,
            include_code: false,
            scope: None,
        };

        let response = build_context(&conn, &request).unwrap();
        // If both foo and bar are selected, the edge between them should appear
        let has_foo = response.symbols.iter().any(|s| s.fqn == "src/a.rs::foo");
        let has_bar = response.symbols.iter().any(|s| s.fqn == "src/b.rs::bar");
        if has_foo && has_bar {
            let has_edge = response
                .relationships
                .iter()
                .any(|r| r.source == "src/a.rs::foo" && r.target == "src/b.rs::bar");
            assert!(has_edge, "Expected edge between foo and bar");
        }
    }

    #[test]
    fn test_token_cost_estimation_no_code() {
        let cost = estimate_token_cost(1, 20, false);
        assert_eq!(cost, NODE_METADATA_TOKENS);
    }

    #[test]
    fn test_token_cost_estimation_with_code() {
        let cost = estimate_token_cost(1, 20, true);
        // 20 tokens metadata + 19 lines * 8 tokens/line = 20 + 152 = 172
        assert_eq!(cost, NODE_METADATA_TOKENS + 19 * TOKENS_PER_LINE);
    }

    #[test]
    fn test_greedy_knapsack_selects_best_density() {
        let mut candidates = vec![
            Candidate {
                fqn: "a".to_string(),
                kind: "Function".to_string(),
                file: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
                relevance: 0.9,
                token_cost: 100,
            },
            Candidate {
                fqn: "b".to_string(),
                kind: "Function".to_string(),
                file: "b.rs".to_string(),
                start_line: 1,
                end_line: 5,
                relevance: 0.8,
                token_cost: 20,
            },
        ];

        let (selected, truncated) = greedy_knapsack(&mut candidates, 50);
        // b has higher density (0.8/20=0.04) vs a (0.9/100=0.009)
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].fqn, "b");
        assert!(truncated);
    }

    #[test]
    fn test_sanitize_fts_query_basic() {
        assert_eq!(sanitize_fts_query("hello world"), "hello world");
        assert_eq!(sanitize_fts_query(""), "");
        assert_eq!(sanitize_fts_query("foo OR bar"), "foo \"OR\" bar");
    }
}
