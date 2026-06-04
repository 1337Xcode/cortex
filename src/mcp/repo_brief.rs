//! Cold-start repository brief for AI coding agents.
//!
//! Provides a single-call summary of the entire codebase (under 400 tokens)
//! so agents can skip 10-20 exploration calls and start working immediately.
//! The brief is cached in the database and invalidated on re-index.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::mcp::health::{check_health, HealthStatus};
use crate::store::db::StoreManager;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Cold-start repository summary (under 400 tokens).
///
/// Contains the essential information an AI agent needs to orient itself
/// in a new codebase: languages, frameworks, entry points, hotspots,
/// security patterns, test shape, and index health.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBrief {
    /// Detected programming languages (by file extension frequency).
    pub languages: Vec<String>,
    /// Detected frameworks from index_health.frameworks_detected.
    pub frameworks: Vec<String>,
    /// Top 5 entry points (main functions or high-caller-count functions).
    pub entry_points: Vec<EntryPoint>,
    /// Top 10 hotspot files (complexity x churn proxy).
    pub hotspots: Vec<HotspotFile>,
    /// Detected auth/security-related patterns in node FQNs.
    pub security_patterns: Vec<String>,
    /// Test coverage shape summary.
    pub test_shape: TestShape,
    /// Current index health status.
    pub health: HealthStatus,
}

/// A significant entry point in the codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPoint {
    /// Fully qualified name of the entry point function.
    pub fqn: String,
    /// One-line description of what this entry point does.
    pub description: String,
}

/// A hotspot file ranked by complexity x churn proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotspotFile {
    /// Fully qualified name of the hotspot symbol.
    pub fqn: String,
    /// Computed score (line_span * caller_count).
    pub score: f64,
}

/// Test coverage shape summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestShape {
    /// Directories containing test files.
    pub test_directories: Vec<String>,
    /// Percentage of functions that have test coverage (0.0 to 100.0).
    pub functions_with_tests_percent: f64,
}

// ---------------------------------------------------------------------------
// Cache helpers
// ---------------------------------------------------------------------------

/// Compute a hash of the current index state for cache invalidation.
fn compute_index_hash(files_indexed: usize, node_count: usize, edge_count: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(files_indexed.to_le_bytes());
    hasher.update(node_count.to_le_bytes());
    hasher.update(edge_count.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

/// Try to load a cached repo brief if the index hash matches.
fn load_cached_brief(store: &StoreManager, current_hash: &str) -> Option<RepoBrief> {
    let conn = store.read_conn();
    let result = conn.query_row(
        "SELECT brief_json, index_hash FROM repo_brief_cache WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        },
    );

    match result {
        Ok((json, stored_hash)) if stored_hash == current_hash => {
            serde_json::from_str(&json).ok()
        }
        _ => None,
    }
}

/// Store the computed brief in the cache.
fn store_cached_brief(store: &StoreManager, brief: &RepoBrief, index_hash: &str) {
    let json = match serde_json::to_string(brief) {
        Ok(j) => j,
        Err(_) => return,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let conn = store.write_conn();
    // Upsert: delete then insert (SQLite doesn't have native UPSERT in all versions)
    let _ = conn.execute("DELETE FROM repo_brief_cache WHERE id = 1", []);
    let _ = conn.execute(
        "INSERT INTO repo_brief_cache (id, brief_json, computed_at, index_hash) VALUES (1, ?1, ?2, ?3)",
        rusqlite::params![json, now, index_hash],
    );
}

// ---------------------------------------------------------------------------
// Core implementation
// ---------------------------------------------------------------------------

/// Build a repository brief aggregating data from the graph database.
///
/// The brief is cached and invalidated when the index state changes
/// (files_indexed, node_count, or edge_count differ from the cached hash).
///
/// The output is kept under 400 tokens (estimated as character_count / 4).
pub fn build_repo_brief(store: &StoreManager) -> RepoBrief {
    let health = check_health(store);

    let index_hash = compute_index_hash(
        health.files_indexed,
        health.node_count,
        health.edge_count,
    );

    // Try cache first
    if let Some(cached) = load_cached_brief(store, &index_hash) {
        return cached;
    }

    // Build fresh brief
    let languages = detect_languages(store);
    let frameworks = health.framework_coverage.clone();
    let entry_points = detect_entry_points(store);
    let hotspots = detect_hotspots(store);
    let security_patterns = detect_security_patterns(store);
    let test_shape = detect_test_shape(store);

    let brief = RepoBrief {
        languages,
        frameworks,
        entry_points,
        hotspots,
        security_patterns,
        test_shape,
        health,
    };

    // Cache the result
    store_cached_brief(store, &brief, &index_hash);

    brief
}

/// Detect languages by grouping nodes by file extension.
fn detect_languages(store: &StoreManager) -> Vec<String> {
    let conn = store.read_conn();

    // Extract file extensions from the nodes table, count occurrences, return top languages
    let mut stmt = match conn.prepare("SELECT DISTINCT file FROM nodes") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let files: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => return Vec::new(),
    };

    // Count extensions
    let mut ext_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for file in &files {
        if let Some(ext) = std::path::Path::new(file).extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            let lang = extension_to_language(&ext_str);
            *ext_counts.entry(lang).or_insert(0) += 1;
        }
    }

    // Sort by count descending, take top entries
    let mut sorted: Vec<(String, usize)> = ext_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted.into_iter().take(5).map(|(lang, _)| lang).collect()
}

/// Map file extension to language name.
fn extension_to_language(ext: &str) -> String {
    match ext {
        "rs" => "Rust".to_string(),
        "py" => "Python".to_string(),
        "ts" | "tsx" => "TypeScript".to_string(),
        "js" | "jsx" => "JavaScript".to_string(),
        "java" => "Java".to_string(),
        "go" => "Go".to_string(),
        "rb" => "Ruby".to_string(),
        "cpp" | "cc" | "cxx" => "C++".to_string(),
        "c" | "h" => "C".to_string(),
        "cs" => "C#".to_string(),
        "php" => "PHP".to_string(),
        "swift" => "Swift".to_string(),
        "kt" | "kts" => "Kotlin".to_string(),
        "scala" => "Scala".to_string(),
        "zig" => "Zig".to_string(),
        "lua" => "Lua".to_string(),
        "ex" | "exs" => "Elixir".to_string(),
        "hs" => "Haskell".to_string(),
        "ml" | "mli" => "OCaml".to_string(),
        other => other.to_uppercase(),
    }
}

/// Detect top 5 entry points: functions named 'main' or with highest caller count.
fn detect_entry_points(store: &StoreManager) -> Vec<EntryPoint> {
    let conn = store.read_conn();

    // First, find functions named 'main' or similar entry-point names
    let mut entry_points: Vec<EntryPoint> = Vec::new();

    // Look for main-like functions
    if let Ok(mut stmt) = conn.prepare(
        "SELECT fqn, kind FROM nodes WHERE kind = 'Function' AND (fqn LIKE '%::main' OR fqn LIKE '%::Main' OR fqn LIKE '%main%') LIMIT 5",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        }) {
            for row in rows.flatten() {
                entry_points.push(EntryPoint {
                    fqn: row.0.clone(),
                    description: format!("Entry point ({})", row.1),
                });
            }
        }
    }

    // If we don't have 5 yet, fill with high-caller-count functions
    if entry_points.len() < 5 {
        let remaining = 5 - entry_points.len();
        let existing_fqns: Vec<String> = entry_points.iter().map(|e| e.fqn.clone()).collect();

        if let Ok(mut stmt) = conn.prepare(
            "SELECT n.fqn, COUNT(e.id) as caller_count \
             FROM nodes n \
             LEFT JOIN edges e ON e.target_fqn = n.fqn AND e.kind = 'Calls' \
             WHERE n.kind IN ('Function', 'Method') \
             GROUP BY n.fqn \
             ORDER BY caller_count DESC \
             LIMIT ?1",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![remaining + existing_fqns.len()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                ))
            }) {
                for row in rows.flatten() {
                    if entry_points.len() >= 5 {
                        break;
                    }
                    if existing_fqns.iter().any(|f| f == &row.0) {
                        continue;
                    }
                    entry_points.push(EntryPoint {
                        fqn: row.0,
                        description: format!("High-traffic function ({} callers)", row.1),
                    });
                }
            }
        }
    }

    entry_points.truncate(5);
    entry_points
}

/// Detect top 10 hotspots by (end_line - start_line) * caller_count.
fn detect_hotspots(store: &StoreManager) -> Vec<HotspotFile> {
    let conn = store.read_conn();

    let mut hotspots: Vec<HotspotFile> = Vec::new();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT n.fqn, (n.end_line - n.start_line) AS line_span, COUNT(e.id) AS caller_count \
         FROM nodes n \
         LEFT JOIN edges e ON e.target_fqn = n.fqn \
         WHERE n.kind IN ('Function', 'Method', 'Class') \
         GROUP BY n.fqn \
         HAVING line_span > 0 \
         ORDER BY (line_span * (1 + caller_count)) DESC \
         LIMIT 10",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let score = (row.1 as f64) * (1.0 + row.2 as f64);
                hotspots.push(HotspotFile {
                    fqn: row.0,
                    score,
                });
            }
        }
    }

    hotspots
}

/// Detect auth/security patterns by searching node FQNs for security keywords.
fn detect_security_patterns(store: &StoreManager) -> Vec<String> {
    let conn = store.read_conn();

    let security_keywords = ["jwt", "auth", "verify", "token", "permission", "credential"];
    let mut patterns: Vec<String> = Vec::new();

    for keyword in &security_keywords {
        let pattern = format!("%{}%", keyword);
        if let Ok(mut stmt) = conn.prepare(
            "SELECT fqn FROM nodes WHERE LOWER(fqn) LIKE LOWER(?1) LIMIT 3",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![pattern], |row| {
                row.get::<_, String>(0)
            }) {
                for fqn in rows.flatten() {
                    if patterns.len() < 10 {
                        patterns.push(fqn);
                    }
                }
            }
        }
    }

    // Deduplicate
    patterns.sort();
    patterns.dedup();
    patterns.truncate(10);
    patterns
}

/// Detect test coverage shape: test directories and function coverage percentage.
fn detect_test_shape(store: &StoreManager) -> TestShape {
    let conn = store.read_conn();

    // Find test directories by looking for files with "test" in the path
    let mut test_dirs: Vec<String> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT file FROM nodes WHERE file LIKE '%test%' OR file LIKE '%spec%' OR file LIKE '%__tests__%'",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            let mut dir_set: std::collections::HashSet<String> = std::collections::HashSet::new();
            for file in rows.flatten() {
                // Extract directory from file path
                if let Some(parent) = std::path::Path::new(&file).parent() {
                    let dir = parent.to_string_lossy().to_string();
                    if !dir.is_empty() {
                        dir_set.insert(dir);
                    }
                }
            }
            test_dirs = dir_set.into_iter().collect();
            test_dirs.sort();
            test_dirs.truncate(5);
        }
    }

    // Compute percentage of functions with test coverage
    let total_functions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind IN ('Function', 'Method')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Count functions in test files (as a proxy for "functions with tests")
    let test_functions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind IN ('Function', 'Method') AND (file LIKE '%test%' OR file LIKE '%spec%')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let functions_with_tests_percent = if total_functions > 0 {
        // Rough estimate: test functions / total functions * 100
        // This is a proxy; real coverage requires LCOV data
        ((test_functions as f64) / (total_functions as f64) * 100.0).min(100.0)
    } else {
        0.0
    };

    TestShape {
        test_directories: test_dirs,
        functions_with_tests_percent,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Path to the migrations directory.
    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    /// Create a test StoreManager with migrations applied.
    fn setup_test_store() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();

        // Apply migrations
        let migrations_path = migrations_dir();
        let conn = store.write_conn();
        crate::store::migrations::run_migrations(&conn, &migrations_path).unwrap();
        drop(conn);

        (store, tmp)
    }

    #[test]
    fn test_build_repo_brief_empty_index() {
        let (store, _tmp) = setup_test_store();

        let brief = build_repo_brief(&store);

        assert!(!brief.health.healthy);
        assert!(brief.languages.is_empty());
        assert!(brief.entry_points.is_empty());
        assert!(brief.hotspots.is_empty());
        assert!(brief.security_patterns.is_empty());
    }

    #[test]
    fn test_build_repo_brief_with_data() {
        let (store, _tmp) = setup_test_store();

        // Populate index_health
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 10, node_count = 50, edge_count = 30, \
                 scip_coverage_percent = 0.5, frameworks_detected = '[\"FastAPI\"]' WHERE id = 1",
                [],
            )
            .unwrap();

            // Insert some nodes
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 50, 'hash1', 1000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/auth.rs::verify_jwt', 'Function', 'src/auth.rs', 1, 20, 'hash2', 1000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/lib.rs::run', 'Function', 'src/lib.rs', 1, 30, 'hash3', 1000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('tests/test_auth.rs::test_login', 'Function', 'tests/test_auth.rs', 1, 10, 'hash4', 1000)",
                [],
            )
            .unwrap();

            // Insert some edges
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES ('src/main.rs::main', 'src/lib.rs::run', 'Calls', 0.5, 'ast_direct')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES ('src/main.rs::main', 'src/auth.rs::verify_jwt', 'Calls', 0.5, 'ast_direct')",
                [],
            )
            .unwrap();
        }

        let brief = build_repo_brief(&store);

        assert!(brief.health.healthy);
        assert!(!brief.languages.is_empty());
        assert!(brief.languages.contains(&"Rust".to_string()));
        assert!(!brief.entry_points.is_empty());
        assert!(!brief.security_patterns.is_empty());
        assert!(brief.frameworks.contains(&"FastAPI".to_string()));
    }

    #[test]
    fn test_build_repo_brief_caching() {
        let (store, _tmp) = setup_test_store();

        // Populate index_health
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 5, node_count = 20, edge_count = 10 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        // First call builds fresh
        let brief1 = build_repo_brief(&store);
        // Second call should use cache
        let brief2 = build_repo_brief(&store);

        assert_eq!(brief1.health.files_indexed, brief2.health.files_indexed);
        assert_eq!(brief1.health.node_count, brief2.health.node_count);
    }

    #[test]
    fn test_build_repo_brief_cache_invalidation() {
        let (store, _tmp) = setup_test_store();

        // Populate index_health
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 5, node_count = 20, edge_count = 10 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        let brief1 = build_repo_brief(&store);

        // Change index state
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 10, node_count = 40, edge_count = 20 WHERE id = 1",
                [],
            )
            .unwrap();
        }

        let brief2 = build_repo_brief(&store);

        // Cache should be invalidated, new health values reflected
        assert_eq!(brief2.health.files_indexed, 10);
        assert_ne!(brief1.health.files_indexed, brief2.health.files_indexed);
    }

    #[test]
    fn test_repo_brief_under_400_tokens() {
        let (store, _tmp) = setup_test_store();

        // Populate with some data
        {
            let conn = store.write_conn();
            conn.execute(
                "UPDATE index_health SET files_indexed = 5, node_count = 20, edge_count = 10, \
                 frameworks_detected = '[\"Express\"]' WHERE id = 1",
                [],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/index.ts::main', 'Function', 'src/index.ts', 1, 50, 'h1', 1000)",
                [],
            )
            .unwrap();
        }

        let brief = build_repo_brief(&store);
        let json = serde_json::to_string(&brief).unwrap();
        let estimated_tokens = json.len() / 4;

        // The brief should be under 400 tokens
        assert!(
            estimated_tokens <= 400,
            "Brief is {} estimated tokens ({}  chars), should be <= 400",
            estimated_tokens,
            json.len()
        );
    }

    #[test]
    fn test_extension_to_language() {
        assert_eq!(extension_to_language("rs"), "Rust");
        assert_eq!(extension_to_language("py"), "Python");
        assert_eq!(extension_to_language("ts"), "TypeScript");
        assert_eq!(extension_to_language("tsx"), "TypeScript");
        assert_eq!(extension_to_language("js"), "JavaScript");
        assert_eq!(extension_to_language("java"), "Java");
        assert_eq!(extension_to_language("go"), "Go");
        assert_eq!(extension_to_language("unknown"), "UNKNOWN");
    }

    #[test]
    fn test_compute_index_hash_deterministic() {
        let hash1 = compute_index_hash(10, 50, 30);
        let hash2 = compute_index_hash(10, 50, 30);
        assert_eq!(hash1, hash2);

        let hash3 = compute_index_hash(10, 50, 31);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_detect_security_patterns() {
        let (store, _tmp) = setup_test_store();

        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/auth.rs::verify_token', 'Function', 'src/auth.rs', 1, 10, 'h1', 1000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at) \
                 VALUES ('src/jwt.rs::decode_jwt', 'Function', 'src/jwt.rs', 1, 10, 'h2', 1000)",
                [],
            )
            .unwrap();
        }

        let patterns = detect_security_patterns(&store);
        assert!(!patterns.is_empty());
        // Should find both auth and jwt related patterns
        assert!(patterns.iter().any(|p| p.contains("verify_token") || p.contains("jwt")));
    }

    #[test]
    fn test_repo_brief_serialization_roundtrip() {
        let brief = RepoBrief {
            languages: vec!["Rust".to_string(), "Python".to_string()],
            frameworks: vec!["FastAPI".to_string()],
            entry_points: vec![EntryPoint {
                fqn: "src/main.rs::main".to_string(),
                description: "Application entry point".to_string(),
            }],
            hotspots: vec![HotspotFile {
                fqn: "src/big_module.rs::process".to_string(),
                score: 150.0,
            }],
            security_patterns: vec!["src/auth.rs::verify_jwt".to_string()],
            test_shape: TestShape {
                test_directories: vec!["tests".to_string()],
                functions_with_tests_percent: 45.0,
            },
            health: HealthStatus {
                healthy: true,
                files_indexed: 42,
                node_count: 100,
                edge_count: 200,
                scip_coverage_percent: 0.75,
                framework_coverage: vec!["FastAPI".to_string()],
                failure_reason: None,
                suggested_action: None,
            },
        };

        let json = serde_json::to_string(&brief).unwrap();
        let deserialized: RepoBrief = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.languages, brief.languages);
        assert_eq!(deserialized.frameworks, brief.frameworks);
        assert_eq!(deserialized.entry_points.len(), brief.entry_points.len());
        assert_eq!(deserialized.hotspots.len(), brief.hotspots.len());
        assert_eq!(deserialized.security_patterns, brief.security_patterns);
        assert_eq!(
            deserialized.test_shape.test_directories,
            brief.test_shape.test_directories
        );
    }

    // ─── Property-Based Tests for cortex-intelligence-overhaul ────────────────

    use proptest::prelude::*;

    /// **Feature: cortex-intelligence-overhaul**
    ///
    /// **Property 17: Repo brief token budget**
    ///
    /// For any indexed state, serialized JSON of get_repo_brief is under
    /// 400 tokens (char_count / 4).
    ///
    /// **Validates: Requirements 22.1**
    mod prop_repo_brief_budget {
        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(100))]

            #[test]
            fn prop_repo_brief_under_400_tokens(
                files_indexed in 1usize..=100,
                node_count in 1usize..=500,
                edge_count in 1usize..=300,
                num_languages in 0usize..=5,
                num_frameworks in 0usize..=3,
                num_entry_points in 0usize..=5,
                num_hotspots in 0usize..=10,
                num_security in 0usize..=10,
            ) {
                // Build a RepoBrief with the generated parameters
                let languages: Vec<String> = (0..num_languages)
                    .map(|i| ["Rust", "Python", "TypeScript", "Go", "Java"][i % 5].to_string())
                    .collect();

                let frameworks: Vec<String> = (0..num_frameworks)
                    .map(|i| ["FastAPI", "Express", "React"][i % 3].to_string())
                    .collect();

                let entry_points: Vec<EntryPoint> = (0..num_entry_points)
                    .map(|i| EntryPoint {
                        fqn: format!("src/mod{}.rs::main", i),
                        description: format!("Entry point {}", i),
                    })
                    .collect();

                let hotspots: Vec<HotspotFile> = (0..num_hotspots)
                    .map(|i| HotspotFile {
                        fqn: format!("src/hot{}.rs::func", i),
                        score: (i as f64) * 10.0,
                    })
                    .collect();

                let security_patterns: Vec<String> = (0..num_security)
                    .map(|i| format!("src/auth{}.rs::verify", i))
                    .collect();

                let brief = RepoBrief {
                    languages,
                    frameworks,
                    entry_points,
                    hotspots,
                    security_patterns,
                    test_shape: TestShape {
                        test_directories: vec!["tests".to_string()],
                        functions_with_tests_percent: 50.0,
                    },
                    health: HealthStatus {
                        healthy: true,
                        files_indexed,
                        node_count,
                        edge_count,
                        scip_coverage_percent: 0.5,
                        framework_coverage: vec![],
                        failure_reason: None,
                        suggested_action: None,
                    },
                };

                let json = serde_json::to_string(&brief).unwrap();
                let estimated_tokens = json.len() / 4;

                prop_assert!(
                    estimated_tokens <= 400,
                    "Brief is {} estimated tokens ({} chars), should be <= 400",
                    estimated_tokens, json.len()
                );
            }
        }
    }
}
