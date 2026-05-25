//! Dead code detection query module.
//!
//! Provides a multi-stage filtering pipeline to identify unused code symbols
//! (functions, methods, classes) that have no inbound Calls or Implements edges,
//! excluding test files, generated code, and annotated nodes.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::error::StoreError;
use crate::store::types::NodeKind;

/// Configuration for dead code detection filtering.
pub struct DeadCodeConfig {
    /// Node kinds eligible for dead code detection.
    pub allowed_kinds: HashSet<NodeKind>,
    /// File path patterns that exclude nodes (test files, generated code).
    pub excluded_path_patterns: Vec<regex::Regex>,
    /// Attribute annotations that exclude nodes.
    pub excluded_annotations: Vec<String>,
    /// Maximum results to return.
    pub limit: usize,
}

impl Default for DeadCodeConfig {
    fn default() -> Self {
        let mut allowed_kinds = HashSet::new();
        allowed_kinds.insert(NodeKind::Function);
        allowed_kinds.insert(NodeKind::Method);
        allowed_kinds.insert(NodeKind::Class);

        Self {
            allowed_kinds,
            excluded_path_patterns: Vec::new(),
            excluded_annotations: vec![
                "test".to_string(),
                "bench".to_string(),
                "example".to_string(),
            ],
            limit: 100,
        }
    }
}

/// Result of dead code detection.
pub struct DeadCodeResult {
    /// Candidate dead code symbols.
    pub candidates: Vec<DeadCodeCandidate>,
    /// Total number of nodes scanned before filtering.
    pub total_scanned: usize,
    /// Descriptions of filters that were applied.
    pub filters_applied: Vec<String>,
}

/// A single dead code candidate.
pub struct DeadCodeCandidate {
    /// Fully qualified name of the symbol.
    pub fqn: String,
    /// Kind of the symbol (Function, Method, or Class).
    pub kind: NodeKind,
    /// File path where the symbol is defined.
    pub file: String,
    /// Starting line number.
    pub start_line: u32,
    /// Ending line number.
    pub end_line: u32,
}

/// Raw row from the SQL query before Rust-side filtering.
struct RawCandidate {
    fqn: String,
    kind: String,
    file: String,
    start_line: u32,
    end_line: u32,
    attributes: String,
}

/// Main entry point for dead code detection.
///
/// Queries the graph database for nodes with zero inbound Calls and Implements
/// edges, then applies in-memory filtering for test/generated paths and
/// annotated nodes.
pub fn find_dead_code(
    conn: &Connection,
    config: &DeadCodeConfig,
) -> Result<DeadCodeResult, StoreError> {
    // Stage 1: SQL query to find nodes with allowed kinds and zero inbound
    // Calls/Implements edges.
    let mut stmt = conn
        .prepare(
            "SELECT n.fqn, n.kind, n.file, n.start_line, n.end_line, n.attributes \
             FROM nodes n \
             WHERE n.kind IN ('Function', 'Method', 'Class') \
               AND n.fqn NOT IN ( \
                   SELECT target_fqn FROM edges WHERE kind = 'Calls' \
               ) \
               AND n.fqn NOT IN ( \
                   SELECT target_fqn FROM edges WHERE kind = 'Implements' \
               ) \
             ORDER BY n.file, n.start_line",
        )
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare find_dead_code: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(RawCandidate {
                fqn: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                attributes: row.get(5)?,
            })
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to execute find_dead_code: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to collect find_dead_code results: {}", e),
        })?;

    let total_scanned = rows.len();

    // Stage 2: In-memory filtering
    let mut filters_applied = Vec::new();

    let mut candidates: Vec<DeadCodeCandidate> = Vec::new();

    for row in &rows {
        // Filter 1: Verify kind is in allowed_kinds
        let kind = parse_node_kind(&row.kind);
        if !config.allowed_kinds.contains(&kind) {
            continue;
        }

        // Filter 2: Exclude test file paths
        if is_test_path(&row.file) {
            continue;
        }

        // Filter 3: Exclude generated file paths
        if is_generated_path(&row.file) {
            continue;
        }

        // Filter 4: Exclude by custom excluded_path_patterns (regex)
        if config
            .excluded_path_patterns
            .iter()
            .any(|re| re.is_match(&row.file))
        {
            continue;
        }

        // Filter 5: Exclude annotated nodes (test/bench/example)
        if has_excluded_annotation(&row.attributes, &config.excluded_annotations) {
            continue;
        }

        candidates.push(DeadCodeCandidate {
            fqn: row.fqn.clone(),
            kind,
            file: row.file.clone(),
            start_line: row.start_line,
            end_line: row.end_line,
        });
    }

    filters_applied.push("kind_filter: Function, Method, Class".to_string());
    filters_applied.push("exclude_test_paths".to_string());
    filters_applied.push("exclude_generated_paths".to_string());
    if !config.excluded_path_patterns.is_empty() {
        filters_applied.push(format!(
            "exclude_custom_patterns: {} patterns",
            config.excluded_path_patterns.len()
        ));
    }
    filters_applied.push(format!(
        "exclude_annotations: {:?}",
        config.excluded_annotations
    ));
    filters_applied.push(format!("limit: {}", config.limit));

    // Stage 3: Apply limit
    candidates.truncate(config.limit);

    Ok(DeadCodeResult {
        candidates,
        total_scanned,
        filters_applied,
    })
}

/// Check if a file path matches test patterns.
/// Matches: contains "test", "_test", "spec", or has a path segment equal to "tests".
fn is_test_path(file: &str) -> bool {
    let lower = file.to_lowercase();

    // Check path segment equals "tests"
    if has_path_segment(&lower, "tests") {
        return true;
    }

    // Check contains "spec" (as a path-relevant pattern)
    if lower.contains("spec") {
        return true;
    }

    // Check contains "_test" (e.g., my_test.py, test_helper.rs)
    if lower.contains("_test")
        || lower.contains("test_")
        || lower.contains("/test/")
        || lower.contains("\\test\\")
    {
        return true;
    }

    // Check if filename starts with "test" or contains "test." pattern
    let filename = file_name_from_path(&lower);
    if filename.starts_with("test") {
        return true;
    }

    false
}

/// Check if a file path matches generated code patterns.
/// Matches: contains "generated", "auto_generated", "proto", ".pb.", or has path segment "gen".
fn is_generated_path(file: &str) -> bool {
    let lower = file.to_lowercase();

    // Check path segment equals "gen"
    if has_path_segment(&lower, "gen") {
        return true;
    }

    // Check contains patterns
    if lower.contains("generated")
        || lower.contains("auto_generated")
        || lower.contains("proto")
        || lower.contains(".pb.")
    {
        return true;
    }

    false
}

/// Check if a path has a specific segment (directory name).
/// Handles both forward and backward slashes.
fn has_path_segment(path: &str, segment: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.split('/').any(|s| s == segment)
}

/// Extract the filename from a path.
fn file_name_from_path(path: &str) -> &str {
    (if path.contains('\\') {
        path.rsplit('\\').next().unwrap_or(path)
    } else {
        path.rsplit('/').next().unwrap_or(path)
    }) as _
}

/// Check if the attributes JSON contains an annotation matching any excluded annotation.
fn has_excluded_annotation(attributes_json: &str, excluded: &[String]) -> bool {
    let attrs: serde_json::Value =
        serde_json::from_str(attributes_json).unwrap_or(serde_json::Value::Null);

    // Check various possible attribute structures for annotations
    // Common patterns: {"annotations": ["test"]}, {"decorators": ["test"]},
    // or direct keys like {"test": true}

    // Check "annotations" array
    if let Some(annotations) = attrs.get("annotations")
        && annotation_matches(annotations, excluded)
    {
        return true;
    }

    // Check "decorators" array
    if let Some(decorators) = attrs.get("decorators")
        && annotation_matches(decorators, excluded)
    {
        return true;
    }

    // Check "attributes" nested field
    if let Some(inner_attrs) = attrs.get("attributes")
        && annotation_matches(inner_attrs, excluded)
    {
        return true;
    }

    // Check top-level keys for annotation-like markers
    if let Some(obj) = attrs.as_object() {
        for key in obj.keys() {
            let key_lower = key.to_lowercase();
            if excluded.iter().any(|ex| key_lower.contains(ex)) {
                return true;
            }
        }
    }

    // Check if the entire JSON string contains annotation markers
    // (handles cases like {"macro": "#[test]"} or {"annotation": "@Test"})
    let attrs_lower = attributes_json.to_lowercase();
    for annotation in excluded {
        // Look for the annotation as a distinct token in the attributes
        if attrs_lower.contains(&format!("\"{}\"", annotation))
            || attrs_lower.contains(&format!("#[{}]", annotation))
            || attrs_lower.contains(&format!("@{}", annotation))
        {
            return true;
        }
    }

    false
}

/// Check if a JSON value (array or string) contains any of the excluded annotations.
fn annotation_matches(value: &serde_json::Value, excluded: &[String]) -> bool {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let s_lower = s.to_lowercase();
                    if excluded.iter().any(|ex| s_lower.contains(ex)) {
                        return true;
                    }
                }
            }
        }
        serde_json::Value::String(s) => {
            let s_lower = s.to_lowercase();
            if excluded.iter().any(|ex| s_lower.contains(ex)) {
                return true;
            }
        }
        _ => {}
    }
    false
}

/// Parse a kind string into a NodeKind.
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Helper: create an in-memory SQLite database with the nodes and edges tables.
    /// Uses relaxed CHECK constraints to allow inserting arbitrary kinds for property testing.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        conn.execute_batch(
            "CREATE TABLE nodes (
                fqn TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                file TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                file_hash TEXT NOT NULL DEFAULT 'test_hash',
                indexed_at INTEGER NOT NULL DEFAULT 0,
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
            CREATE INDEX idx_edges_kind ON edges(kind);",
        )
        .expect("failed to create test schema");
        conn
    }

    /// All possible node kinds for property testing (including non-allowed ones).
    fn arb_node_kind() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("Function".to_string()),
            Just("Method".to_string()),
            Just("Class".to_string()),
            Just("Module".to_string()),
            Just("Route".to_string()),
            Just("Interface".to_string()),
            Just("Type".to_string()),
            Just("Enum".to_string()),
            Just("Constant".to_string()),
            Just("TypeAlias".to_string()),
            Just("Trait".to_string()),
            Just("Namespace".to_string()),
        ]
    }

    /// Generate file paths that may or may not match test/generated patterns.
    fn arb_file_path() -> impl Strategy<Value = String> {
        prop_oneof![
            // Normal paths (should NOT be excluded)
            Just("src/main.rs".to_string()),
            Just("src/lib.rs".to_string()),
            Just("src/utils/helpers.rs".to_string()),
            Just("app/models/user.py".to_string()),
            Just("pkg/server/handler.go".to_string()),
            // Test paths (should be excluded)
            Just("src/tests/unit.rs".to_string()),
            Just("tests/integration.rs".to_string()),
            Just("src/my_test.py".to_string()),
            Just("test_utils.rs".to_string()),
            Just("src/spec/models.rb".to_string()),
            // Generated paths (should be excluded)
            Just("src/generated/models.rs".to_string()),
            Just("proto/service.proto".to_string()),
            Just("src/service.pb.go".to_string()),
            Just("gen/types.ts".to_string()),
            Just("src/auto_generated/schema.rs".to_string()),
        ]
    }

    /// Generate attributes JSON that may or may not contain excluded annotations.
    fn arb_attributes() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("{}".to_string()),
            Just(r#"{"async": true}"#.to_string()),
            Just(r#"{"visibility": "public"}"#.to_string()),
            // Annotated (should be excluded)
            Just(r#"{"annotations": ["test"]}"#.to_string()),
            Just(r#"{"annotations": ["bench", "inline"]}"#.to_string()),
            Just(r#"{"decorators": ["example"]}"#.to_string()),
            Just(r##"{"macro": "#[test]"}"##.to_string()),
            Just(r#"{"annotations": ["benchmark"]}"#.to_string()),
        ]
    }

    /// A generated node for property testing.
    #[derive(Debug, Clone)]
    struct ArbitraryNode {
        fqn: String,
        kind: String,
        file: String,
        start_line: u32,
        end_line: u32,
        attributes: String,
    }

    #[allow(dead_code)]
    fn arb_node(index: usize) -> impl Strategy<Value = ArbitraryNode> {
        (
            arb_node_kind(),
            arb_file_path(),
            arb_attributes(),
            1u32..500u32,
        )
            .prop_map(move |(kind, file, attributes, start)| ArbitraryNode {
                fqn: format!("{}::symbol_{}", file, index),
                kind,
                file,
                start_line: start,
                end_line: start + 10,
                attributes,
            })
    }

    /// Generate a vector of arbitrary nodes.
    fn arb_nodes(max_count: usize) -> impl Strategy<Value = Vec<ArbitraryNode>> {
        proptest::collection::vec(
            (
                arb_node_kind(),
                arb_file_path(),
                arb_attributes(),
                1u32..500u32,
            ),
            1..=max_count,
        )
        .prop_map(|items| {
            items
                .into_iter()
                .enumerate()
                .map(|(i, (kind, file, attributes, start))| ArbitraryNode {
                    fqn: format!("{}::symbol_{}", file, i),
                    kind,
                    file,
                    start_line: start,
                    end_line: start + 10,
                    attributes,
                })
                .collect()
        })
    }

    /// Insert nodes into the test database.
    fn insert_nodes(conn: &Connection, nodes: &[ArbitraryNode]) {
        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO nodes (fqn, kind, file, start_line, end_line, attributes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .unwrap();
        for node in nodes {
            let _ = stmt.execute(rusqlite::params![
                node.fqn,
                node.kind,
                node.file,
                node.start_line,
                node.end_line,
                node.attributes,
            ]);
        }
    }

    /// Insert an Implements edge targeting a specific node.
    fn insert_implements_edge(conn: &Connection, source_fqn: &str, target_fqn: &str) {
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind) VALUES (?1, ?2, 'Implements')",
            rusqlite::params![source_fqn, target_fqn],
        )
        .unwrap();
    }

    /// Insert a Calls edge targeting a specific node.
    #[allow(dead_code)]
    fn insert_calls_edge(conn: &Connection, source_fqn: &str, target_fqn: &str) {
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind) VALUES (?1, ?2, 'Calls')",
            rusqlite::params![source_fqn, target_fqn],
        )
        .unwrap();
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    proptest! {
        /// **Validates: Requirements 1.1, 1.2**
        ///
        /// Property 1: Dead code results contain only allowed kinds.
        /// For any graph containing nodes of arbitrary kinds, the dead code detector
        /// SHALL only return nodes with kind in {Function, Method, Class}.
        #[test]
        fn prop_dead_code_results_contain_only_allowed_kinds(
            nodes in arb_nodes(20)
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            let config = DeadCodeConfig::default();
            let result = find_dead_code(&conn, &config).unwrap();

            for candidate in &result.candidates {
                prop_assert!(
                    candidate.kind == NodeKind::Function
                        || candidate.kind == NodeKind::Method
                        || candidate.kind == NodeKind::Class,
                    "Dead code candidate had disallowed kind: {:?} (fqn: {})",
                    candidate.kind,
                    candidate.fqn
                );
            }
        }

        /// **Validates: Requirements 1.3, 1.4**
        ///
        /// Property 2: Dead code excludes test and generated file paths.
        /// For any node whose file path matches a test or generated pattern,
        /// the dead code detector SHALL exclude that node from results.
        #[test]
        fn prop_dead_code_excludes_test_and_generated_paths(
            nodes in arb_nodes(20)
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            let config = DeadCodeConfig::default();
            let result = find_dead_code(&conn, &config).unwrap();

            for candidate in &result.candidates {
                prop_assert!(
                    !is_test_path(&candidate.file),
                    "Dead code result included a test path: {} (fqn: {})",
                    candidate.file,
                    candidate.fqn
                );
                prop_assert!(
                    !is_generated_path(&candidate.file),
                    "Dead code result included a generated path: {} (fqn: {})",
                    candidate.file,
                    candidate.fqn
                );
            }
        }

        /// **Validates: Requirements 1.5**
        ///
        /// Property 3: Dead code excludes nodes with Implements edges.
        /// For any node that has at least one inbound Implements edge,
        /// the dead code detector SHALL exclude that node from results.
        #[test]
        fn prop_dead_code_excludes_nodes_with_implements_edges(
            nodes in arb_nodes(15),
            implements_indices in proptest::collection::vec(0usize..15, 1..=5)
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            // Add Implements edges to some nodes
            let mut implemented_fqns: Vec<String> = Vec::new();
            for &idx in &implements_indices {
                if idx < nodes.len() {
                    let target = &nodes[idx].fqn;
                    insert_implements_edge(&conn, "some_interface::trait_impl", target);
                    implemented_fqns.push(target.clone());
                }
            }

            let config = DeadCodeConfig::default();
            let result = find_dead_code(&conn, &config).unwrap();

            for candidate in &result.candidates {
                prop_assert!(
                    !implemented_fqns.contains(&candidate.fqn),
                    "Dead code result included a node with Implements edge: {} ",
                    candidate.fqn
                );
            }
        }

        /// **Validates: Requirements 1.6**
        ///
        /// Property 4: Dead code excludes annotated nodes.
        /// For any node with kind Function or Method whose attributes JSON contains
        /// "test", "bench", or "example", the dead code detector SHALL exclude that node.
        #[test]
        fn prop_dead_code_excludes_annotated_nodes(
            nodes in arb_nodes(20)
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            let config = DeadCodeConfig::default();
            let result = find_dead_code(&conn, &config).unwrap();

            // For each candidate in results, verify it does NOT have excluded annotations
            for candidate in &result.candidates {
                // Find the original node data to check its attributes
                let original = nodes.iter().find(|n| n.fqn == candidate.fqn);
                if let Some(orig) = original {
                    prop_assert!(
                        !has_excluded_annotation(&orig.attributes, &config.excluded_annotations),
                        "Dead code result included an annotated node: {} with attributes: {}",
                        candidate.fqn,
                        orig.attributes
                    );
                }
            }
        }

        /// **Validates: Requirements 1.7**
        ///
        /// Property 5: Dead code respects limit parameter.
        /// For any positive integer limit value, the dead code detector SHALL return
        /// at most that number of results.
        #[test]
        fn prop_dead_code_respects_limit(
            nodes in arb_nodes(30),
            limit in 1usize..=50
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            let config = DeadCodeConfig { limit, ..Default::default() };

            let result = find_dead_code(&conn, &config).unwrap();

            prop_assert!(
                result.candidates.len() <= limit,
                "Dead code returned {} results but limit was {}",
                result.candidates.len(),
                limit
            );
        }
    }

    // ─── Default limit property (no explicit limit → max 100) ─────────────────

    proptest! {
        /// **Validates: Requirements 1.7**
        ///
        /// When no limit is specified (default config), results SHALL not exceed 100.
        #[test]
        fn prop_dead_code_default_limit_is_100(
            nodes in arb_nodes(30)
        ) {
            let conn = setup_test_db();
            insert_nodes(&conn, &nodes);

            let config = DeadCodeConfig::default();
            let result = find_dead_code(&conn, &config).unwrap();

            prop_assert!(
                result.candidates.len() <= 100,
                "Dead code returned {} results with default config (expected <= 100)",
                result.candidates.len()
            );
        }
    }

    // ─── Unit Tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_is_test_path() {
        assert!(is_test_path("src/tests/helper.rs"));
        assert!(is_test_path("src/my_test.py"));
        assert!(is_test_path("test_utils.rs"));
        assert!(is_test_path("src/spec/models.rb"));
        assert!(is_test_path("tests/integration.rs"));
        assert!(!is_test_path("src/main.rs"));
        assert!(!is_test_path("src/utils/contest.rs"));
    }

    #[test]
    fn test_is_generated_path() {
        assert!(is_generated_path("src/generated/models.rs"));
        assert!(is_generated_path("proto/service.proto"));
        assert!(is_generated_path("src/service.pb.go"));
        assert!(is_generated_path("gen/types.ts"));
        assert!(is_generated_path("src/auto_generated/schema.rs"));
        assert!(!is_generated_path("src/main.rs"));
        assert!(!is_generated_path("src/generic/utils.rs"));
    }

    #[test]
    fn test_has_excluded_annotation() {
        // annotations array
        assert!(has_excluded_annotation(
            r#"{"annotations": ["test", "inline"]}"#,
            &["test".to_string(), "bench".to_string()]
        ));

        // decorators array
        assert!(has_excluded_annotation(
            r#"{"decorators": ["benchmark"]}"#,
            &["bench".to_string()]
        ));

        // No match
        assert!(!has_excluded_annotation(
            r#"{"async": true}"#,
            &["test".to_string(), "bench".to_string()]
        ));

        // Rust-style attribute
        assert!(has_excluded_annotation(
            r##"{"macro": "#[test]"}"##,
            &["test".to_string()]
        ));

        // Empty attributes
        assert!(!has_excluded_annotation(r#"{}"#, &["test".to_string()]));
    }

    #[test]
    fn test_has_path_segment() {
        assert!(has_path_segment("src/tests/helper.rs", "tests"));
        assert!(has_path_segment("src\\tests\\helper.rs", "tests"));
        assert!(!has_path_segment("src/testing/helper.rs", "tests"));
        assert!(has_path_segment("gen/types.ts", "gen"));
        assert!(!has_path_segment("src/generic/utils.rs", "gen"));
    }

    #[test]
    fn test_default_config() {
        let config = DeadCodeConfig::default();
        assert_eq!(config.limit, 100);
        assert!(config.allowed_kinds.contains(&NodeKind::Function));
        assert!(config.allowed_kinds.contains(&NodeKind::Method));
        assert!(config.allowed_kinds.contains(&NodeKind::Class));
        assert_eq!(config.excluded_annotations.len(), 3);
    }
}
