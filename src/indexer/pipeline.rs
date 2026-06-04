//! Full repository indexing pipeline with SHA-256 delta detection and Rayon parallelism.
//!
//! Walks the directory tree, computes file hashes, parses changed files in parallel,
//! resolves cross-file FQNs, and applies deltas to the database serially.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::IndexError;
use crate::indexer::framework_detect;
use crate::indexer::http_routes;
use crate::indexer::languages;
use crate::indexer::parser::{self, SupportedLanguage};
use crate::indexer::resolver;
use crate::indexer::scip;
use crate::security::secrets;
use crate::store::db::StoreManager;
use crate::store::queries::delta::{GraphDelta, apply_delta, apply_deltas_batch};
use crate::store::types::{ExtractionResult, FileSnapshot};

/// Directories to always skip during walking.
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".cortex",
    ".cortex-data",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".serena",
    ".cursor",
    ".kiro",
    ".agent",
    "dist",
    "build",
    ".next",
    "out",
    "vendor",
    ".output",
    ".nuxt",
    ".svelte-kit",
    "coverage",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".eggs",
    "site-packages",
];

/// Files to always skip during indexing (lock files, generated manifests).
const EXCLUDED_FILES: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "Cargo.lock",
];

/// Statistics from a full repository index run.
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub files_scanned: usize,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_deleted: usize,
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub edges_added: usize,
    pub duration_ms: u64,
}

/// Index an entire repository, applying delta updates for changed files.
///
/// Pipeline steps:
/// 1. Walk directory tree with exclusion rules
/// 2. SHA-256 delta detection against file_snapshots
/// 3. Parallel parsing via Rayon
/// 4. Cross-file FQN resolution
/// 5. Serial delta application to SQLite
/// 6. Handle deleted files
pub fn index_repository(repo_root: &Path, store: &StoreManager) -> Result<IndexStats, IndexError> {
    let start = Instant::now();
    let mut stats = IndexStats::default();

    // Load ignore patterns
    let gitignore_patterns = load_ignore_patterns(repo_root, ".gitignore");
    let cortex_ignore_patterns = load_ignore_patterns(repo_root, ".cortexignore");

    // Step 1: Walk directory tree and collect candidate files
    let candidate_files = walk_directory(repo_root, &gitignore_patterns, &cortex_ignore_patterns);
    stats.files_scanned = candidate_files.len();

    // Step 2: SHA-256 delta detection
    let files_to_process = detect_changed_files(&candidate_files, repo_root, store)?;
    stats.files_skipped = stats.files_scanned - files_to_process.len();

    // Step 3: Parallel parsing via Rayon (using pre-computed hashes)
    let parse_results: Vec<(String, ExtractionResult, String)> = files_to_process
        .par_iter()
        .filter_map(|(file_path, file_hash)| {
            match parse_and_extract(file_path, repo_root, file_hash) {
                Ok(result) => Some(result),
                Err(e) => {
                    tracing::warn!(file = %file_path, error = %e, "parse_and_extract failed");
                    None
                }
            }
        })
        .collect();

    // Step 4: Cross-file FQN resolution (incremental: only resolve edges for changed files)
    let mut all_nodes: Vec<crate::store::types::Node> = Vec::new();
    let mut all_edges: Vec<crate::store::types::Edge> = Vec::new();

    for (_path, result, _hash) in &parse_results {
        all_nodes.extend(result.nodes.clone());
        all_edges.extend(result.edges.clone());
    }

    // Build the set of changed file paths for incremental resolution
    let changed_files: HashSet<String> = files_to_process
        .iter()
        .map(|(path, _hash)| path.clone())
        .collect();

    let fqn_index = resolver::build_fqn_index(&all_nodes);
    resolver::resolve_cross_file_edges_incremental(
        &all_nodes,
        &mut all_edges,
        &fqn_index,
        Some(&changed_files),
    );

    // Step 5: Apply deltas serially
    // Build a map from file path to resolved edges for that file
    let mut edge_map: std::collections::HashMap<String, Vec<crate::store::types::Edge>> =
        std::collections::HashMap::new();
    for edge in &all_edges {
        // Determine which file this edge belongs to based on source_fqn
        if let Some(file_prefix) = edge.source_fqn.split("::").next() {
            edge_map
                .entry(file_prefix.to_string())
                .or_default()
                .push(edge.clone());
        }
    }

    {
        let mut conn = store.write_conn();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Build all deltas first, then apply in a single transaction for performance.
        // One transaction for all files avoids per-file fsync overhead in SQLite.
        let mut deltas: Vec<GraphDelta> = Vec::with_capacity(parse_results.len());

        for (rel_path, result, file_hash) in &parse_results {
            // Get edges for this file (from resolved edges)
            let file_edges = edge_map.remove(rel_path).unwrap_or_default();

            // Before adding new nodes, remove old nodes for this file
            let old_nodes = get_nodes_for_file(&conn, rel_path);
            let new_fqns: HashSet<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
            let nodes_to_remove: Vec<String> = old_nodes
                .into_iter()
                .filter(|fqn| !new_fqns.contains(fqn.as_str()))
                .collect();

            // Remove old edges for this file before inserting new ones
            remove_edges_for_file(&conn, rel_path);

            deltas.push(GraphDelta {
                nodes_to_add: result.nodes.clone(),
                nodes_to_remove,
                edges_to_add: file_edges,
                file_snapshot: FileSnapshot {
                    file: rel_path.clone(),
                    file_hash: file_hash.clone(),
                    node_count: result.nodes.len() as u32,
                    indexed_at: now,
                },
            });
        }

        // Apply all deltas in a single transaction (dramatically faster than per-file commits)
        match apply_deltas_batch(&mut conn, &deltas) {
            Ok(delta_stats) => {
                stats.nodes_added += delta_stats.nodes_added;
                stats.nodes_removed += delta_stats.nodes_removed;
                stats.edges_added += delta_stats.edges_added;
                stats.files_indexed += deltas.len();
            }
            Err(e) => {
                tracing::warn!(
                    "batch delta application failed, falling back to per-file: {}",
                    e
                );
                // Fallback: apply deltas one by one so partial progress is preserved
                for delta in &deltas {
                    match apply_delta(&mut conn, delta) {
                        Ok(delta_stats) => {
                            stats.nodes_added += delta_stats.nodes_added;
                            stats.nodes_removed += delta_stats.nodes_removed;
                            stats.edges_added += delta_stats.edges_added;
                            stats.files_indexed += 1;
                        }
                        Err(_e) => {
                            tracing::warn!(
                                "failed to apply delta for {}: {}",
                                crate::telemetry::sanitize_path(Path::new(
                                    &delta.file_snapshot.file
                                )),
                                _e
                            );
                        }
                    }
                }
            }
        }
    }

    // Step 6: Generate embeddings for Function/Class nodes (when semantic feature is enabled)
    generate_embeddings_for_nodes(store, &all_nodes, repo_root);

    // Step 7: Handle deleted files
    let deleted_count = handle_deleted_files(repo_root, &candidate_files, store)?;
    stats.files_deleted = deleted_count;

    // Step 8: SCIP index ingestion (if an index file exists next to the repo root).
    // Runs after tree-sitter indexing so SCIP edges can supersede ast_direct edges
    // for the same (source_fqn, target_fqn) pairs.
    let scip_coverage = if let Some(scip_path) = scip::find_scip_index(repo_root) {
        scip::try_ingest_scip(&scip_path, store);
        scip::compute_scip_coverage(store)
    } else {
        scip::compute_scip_coverage(store)
    };

    // Step 9: Framework detection — scan dependency manifests and record detected
    // frameworks in index_health for health-gate reporting.
    let detected_frameworks = framework_detect::detect_frameworks(repo_root);
    let framework_names: Vec<String> = detected_frameworks
        .iter()
        .map(|f| f.name.as_str().to_string())
        .collect();
    let frameworks_json = serde_json::to_string(&framework_names).unwrap_or_else(|_| "[]".to_string());

    // Step 10: Update the index_health singleton row with current metrics.
    {
        let conn = store.write_conn();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let node_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap_or(0);
        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap_or(0);
        let files_indexed: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_snapshots", [], |row| row.get(0))
            .unwrap_or(0);

        let _ = conn.execute(
            "UPDATE index_health SET \
             files_indexed = ?1, \
             node_count = ?2, \
             edge_count = ?3, \
             scip_coverage_percent = ?4, \
             frameworks_detected = ?5, \
             last_index_at = ?6, \
             health_status = 'healthy' \
             WHERE id = 1",
            rusqlite::params![
                files_indexed,
                node_count,
                edge_count,
                scip_coverage,
                frameworks_json,
                now,
            ],
        );
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;
    Ok(stats)
}

/// Walk the directory tree applying exclusion rules.
///
/// Returns repo-root-relative paths (with forward slashes) for all candidate files.
fn walk_directory(
    repo_root: &Path,
    gitignore_patterns: &[String],
    cortex_ignore_patterns: &[String],
) -> Vec<String> {
    let mut files = Vec::new();

    for entry in WalkDir::new(repo_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e, repo_root))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Skip excluded files (lock files, etc.) before any further processing
        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if EXCLUDED_FILES.contains(&filename) {
            continue;
        }

        // Skip minified/bundled files (contain hashes like .D2o_JWn5.js or are .min.js)
        if filename.contains(".min.")
            || filename.ends_with(".min.js")
            || filename.ends_with(".min.css")
            || filename.ends_with(".bundle.js")
            || filename.ends_with(".chunk.js")
            || (filename.len() > 20 && filename.matches('.').count() >= 3)
        {
            continue;
        }

        // Only process files with recognized extensions
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if languages::language_for_extension(ext).is_none() {
            continue;
        }

        // Compute relative path
        let rel_path = match path.strip_prefix(repo_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        // Check ignore patterns
        if matches_any_pattern(&rel_path, gitignore_patterns)
            || matches_any_pattern(&rel_path, cortex_ignore_patterns)
        {
            continue;
        }

        files.push(rel_path);
    }

    files
}

/// Check if a walkdir entry is an excluded directory.
fn is_excluded_dir(entry: &walkdir::DirEntry, _repo_root: &Path) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    EXCLUDED_DIRS.iter().any(|&excluded| name == excluded)
}

/// Load ignore patterns from a file (one pattern per line).
fn load_ignore_patterns(repo_root: &Path, filename: &str) -> Vec<String> {
    let path = repo_root.join(filename);
    match fs::read_to_string(&path) {
        Ok(content) => content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Basic pattern matching for gitignore-style patterns.
///
/// Supports:
/// - Exact filename matches (e.g., "file.txt")
/// - Directory prefix matches (e.g., "build/")
/// - Wildcard extension matches (e.g., "*.log")
/// - Path prefix matches (e.g., "dist/")
fn matches_any_pattern(rel_path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if pattern.ends_with('/') {
            // Directory pattern: matches if path starts with this prefix
            let dir_prefix = &pattern[..pattern.len() - 1];
            if rel_path.starts_with(dir_prefix) || rel_path.contains(&format!("/{}/", dir_prefix)) {
                return true;
            }
        } else if pattern.starts_with("*.") {
            // Wildcard extension pattern
            let ext = &pattern[1..]; // e.g., ".log"
            if rel_path.ends_with(ext) {
                return true;
            }
        } else if pattern.contains('/') {
            // Path pattern
            if rel_path.starts_with(pattern) || rel_path == *pattern {
                return true;
            }
        } else {
            // Simple filename or directory name match
            let segments: Vec<&str> = rel_path.split('/').collect();
            if segments.iter().any(|s| *s == pattern) {
                return true;
            }
        }
    }
    false
}

/// Compute SHA-256 hash of file contents.
fn compute_file_hash(path: &Path) -> Result<String, IndexError> {
    let content = fs::read(path).map_err(|e| IndexError::FileReadFailed {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

/// Detect which files have changed by comparing SHA-256 hashes against file_snapshots.
/// Returns a Vec of (rel_path, file_hash) tuples for files that need processing.
fn detect_changed_files(
    candidate_files: &[String],
    repo_root: &Path,
    store: &StoreManager,
) -> Result<Vec<(String, String)>, IndexError> {
    let conn = store.read_conn();
    let mut changed = Vec::new();

    for rel_path in candidate_files {
        let abs_path = repo_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let file_hash = compute_file_hash(&abs_path)?;

        // Check against stored snapshot
        let stored_hash: Option<String> = conn
            .query_row(
                "SELECT file_hash FROM file_snapshots WHERE file = ?1",
                rusqlite::params![rel_path],
                |row| row.get(0),
            )
            .ok();

        match stored_hash {
            Some(ref h) if h == &file_hash => {
                // File unchanged, skip
            }
            _ => {
                // File is new or changed
                changed.push((rel_path.clone(), file_hash));
            }
        }
    }

    Ok(changed)
}

/// Parse and extract a single file, returning (relative_path, ExtractionResult, file_hash).
/// Accepts a pre-computed hash to avoid redundant SHA-256 computation.
///
/// Creates a Module node (FQN = file path) for each file before symbol extraction,
/// ensuring Import edges (which use the file path as source_fqn) satisfy FK constraints.
fn parse_and_extract(
    rel_path: &str,
    repo_root: &Path,
    precomputed_hash: &str,
) -> Result<(String, ExtractionResult, String), IndexError> {
    let abs_path = repo_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));

    // Sanitize the file path for any tracing output
    let sanitized_path = crate::telemetry::sanitize_path(&abs_path);

    // Try reading as UTF-8 string first; if that fails due to encoding,
    // read as bytes and sanitize to valid UTF-8.
    let source = match fs::read_to_string(&abs_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            // File contains invalid UTF-8 - read as bytes and sanitize
            let bytes = fs::read(&abs_path).map_err(|e| IndexError::FileReadFailed {
                path: rel_path.to_string(),
                reason: e.to_string(),
            })?;
            let (sanitized_source, _replacements) = crate::telemetry::sanitize_utf8(&bytes);
            tracing::debug!(
                file = %sanitized_path,
                original_byte_length = bytes.len(),
                "Source file contained invalid UTF-8, sanitized for processing"
            );
            sanitized_source
        }
        Err(e) => {
            return Err(IndexError::FileReadFailed {
                path: rel_path.to_string(),
                reason: e.to_string(),
            });
        }
    };

    let file_hash = precomputed_hash.to_string();

    // Try tree-sitter parsing first; if unsupported, try regex-based extraction
    let mut result = match parser::parse(&abs_path, &source) {
        Ok((language, tree)) => dispatch_extractor(language, &tree, rel_path, &source),
        Err(crate::error::ParseError::UnsupportedLanguage { .. }) => {
            // Try regex-based extraction for languages without tree-sitter grammars
            match dispatch_regex_extractor(rel_path, &source) {
                Some(extraction) => extraction,
                None => {
                    return Err(IndexError::ParseFailed {
                        path: rel_path.to_string(),
                        reason: "unsupported language".to_string(),
                    });
                }
            }
        }
        Err(e) => {
            return Err(IndexError::ParseFailed {
                path: rel_path.to_string(),
                reason: e.to_string(),
            });
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Create a Module node for this file (FQN = file path).
    // This ensures Import edges (which use file path as source_fqn) satisfy FK constraints.
    let line_count = source.lines().count().max(1) as u32;
    let module_node = crate::store::types::Node {
        fqn: rel_path.to_string(),
        kind: crate::store::types::NodeKind::Module,
        file: rel_path.to_string(),
        start_line: 1,
        end_line: line_count,
        file_hash: file_hash.clone(),
        indexed_at: now,
        attributes: serde_json::json!({}),
    };

    // Insert Module node at the beginning so it's created before edges reference it
    result.nodes.insert(0, module_node);

    // Set file_hash and indexed_at on all extracted nodes (skip index 0, the Module node)
    for node in result.nodes.iter_mut().skip(1) {
        node.file_hash = file_hash.clone();
        node.indexed_at = now;
    }

    // Secret scanning: detect secrets in source and tag containing nodes
    let secret_matches = secrets::detect_secrets(&source);
    if !secret_matches.is_empty() {
        for secret_match in &secret_matches {
            // Find the node that contains this secret (by line range)
            // Tag the containing function/class node, or the module node if no container
            let mut tagged = false;
            for node in result.nodes.iter_mut().skip(1) {
                if secret_match.line >= node.start_line && secret_match.line <= node.end_line {
                    // Merge contains_secret into the node's attributes
                    if let Some(attrs) = node.attributes.as_object_mut() {
                        attrs.insert(
                            "contains_secret".to_string(),
                            serde_json::json!(secret_match.secret_type.label()),
                        );
                        attrs.insert(
                            "secret_line".to_string(),
                            serde_json::json!(secret_match.line),
                        );
                    }
                    tagged = true;
                    break;
                }
            }
            // If no containing node found, tag the module node
            if !tagged
                && let Some(module_node) = result.nodes.first_mut()
                && let Some(attrs) = module_node.attributes.as_object_mut()
            {
                attrs.insert(
                    "contains_secret".to_string(),
                    serde_json::json!(secret_match.secret_type.label()),
                );
                attrs.insert(
                    "secret_line".to_string(),
                    serde_json::json!(secret_match.line),
                );
            }
        }
    }

    // HTTP route detection
    let lang_str = match parser::parse(&abs_path, &source) {
        Ok((lang, _)) => lang.as_str(),
        Err(_) => "",
    };
    if !lang_str.is_empty() {
        http_routes::detect_routes(
            &mut result.nodes,
            &mut result.edges,
            rel_path,
            &source,
            lang_str,
        );
    }

    Ok((rel_path.to_string(), result, file_hash))
}

/// Dispatch to the appropriate language extractor.
fn dispatch_extractor(
    language: SupportedLanguage,
    tree: &tree_sitter::Tree,
    file: &str,
    source: &str,
) -> ExtractionResult {
    match language {
        SupportedLanguage::Python => languages::python::extract(tree, file, source),
        SupportedLanguage::TypeScript => languages::typescript::extract(tree, file, source),
        SupportedLanguage::Tsx => languages::typescript::extract(tree, file, source),
        SupportedLanguage::JavaScript => languages::typescript::extract(tree, file, source),
        SupportedLanguage::Go => languages::go::extract(tree, file, source),
        SupportedLanguage::Rust => languages::rust_lang::extract(tree, file, source),
        SupportedLanguage::Java => languages::java::extract(tree, file, source),
        SupportedLanguage::CSharp => languages::csharp::extract(tree, file, source),
        SupportedLanguage::Cpp => languages::cpp::extract(tree, file, source),
        SupportedLanguage::Ruby => languages::ruby::extract(tree, file, source),
        SupportedLanguage::C => languages::c_lang::extract(tree, file, source),
        SupportedLanguage::Scala => languages::scala::extract(tree, file, source),
        SupportedLanguage::Swift => languages::swift::extract(tree, file, source),
        SupportedLanguage::Php => languages::php::extract(tree, file, source),
        SupportedLanguage::Sql => languages::sql::extract_sql(file, source),
        SupportedLanguage::Kotlin => languages::kotlin::extract_regex(file, source),
        SupportedLanguage::Dart => languages::dart::extract(tree, file, source),
        SupportedLanguage::Elixir => languages::elixir::extract(tree, file, source),
        SupportedLanguage::Haskell => languages::haskell::extract(tree, file, source),
        SupportedLanguage::Lua => languages::lua::extract(tree, file, source),
        SupportedLanguage::Zig => languages::zig::extract(tree, file, source),
        SupportedLanguage::Bash => languages::bash::extract(tree, file, source),
        SupportedLanguage::Perl => languages::perl::extract_regex(file, source),
        SupportedLanguage::R => languages::r_lang::extract(tree, file, source),
        SupportedLanguage::ObjectiveC => languages::objc::extract(tree, file, source),
        SupportedLanguage::OCaml => languages::ocaml::extract(tree, file, source),
        SupportedLanguage::Julia => languages::julia::extract(tree, file, source),
        SupportedLanguage::Terraform => languages::terraform::extract(tree, file, source),
        SupportedLanguage::Yaml => languages::yaml::extract(tree, file, source),
    }
}

/// Dispatch to regex-based extractors for languages without tree-sitter grammars.
/// Returns None if the file extension is not recognized as a regex-extractable language.
fn dispatch_regex_extractor(file: &str, source: &str) -> Option<ExtractionResult> {
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        #[allow(deprecated)]
        "scala" | "sc" => Some(languages::scala::extract_regex(file, source)),
        #[allow(deprecated)]
        "swift" => Some(languages::swift::extract_swift(file, source)),
        #[allow(deprecated)]
        "php" => Some(languages::php::extract_php(file, source)),
        "sql" => Some(languages::sql::extract_sql(file, source)),
        "kt" | "kts" => Some(languages::kotlin::extract_regex(file, source)),
        #[allow(deprecated)]
        "dart" => Some(languages::dart::extract_regex(file, source)),
        #[allow(deprecated)]
        "ex" | "exs" => Some(languages::elixir::extract_regex(file, source)),
        #[allow(deprecated)]
        "hs" | "lhs" => Some(languages::haskell::extract_regex(file, source)),
        #[allow(deprecated)]
        "lua" => Some(languages::lua::extract_regex(file, source)),
        #[allow(deprecated)]
        "zig" => Some(languages::zig::extract_regex(file, source)),
        "sh" | "bash" | "zsh" =>
        {
            #[allow(deprecated)]
            Some(languages::bash::extract_regex(file, source))
        }
        "pl" | "pm" => Some(languages::perl::extract_regex(file, source)),
        #[allow(deprecated)]
        "r" | "R" => Some(languages::r_lang::extract_regex(file, source)),
        #[allow(deprecated)]
        "m" => Some(languages::objc::extract_regex(file, source)),
        #[allow(deprecated)]
        "ml" | "mli" => Some(languages::ocaml::extract_regex(file, source)),
        #[allow(deprecated)]
        "jl" => Some(languages::julia::extract_regex(file, source)),
        #[allow(deprecated)]
        "tf" | "hcl" => Some(languages::terraform::extract_regex(file, source)),
        #[allow(deprecated)]
        "yml" | "yaml" => Some(languages::yaml::extract_regex(file, source)),
        _ => None,
    }
}

/// Get all node FQNs for a given file from the database.
fn get_nodes_for_file(
    conn: &std::sync::MutexGuard<'_, rusqlite::Connection>,
    file: &str,
) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT fqn FROM nodes WHERE file = ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map(rusqlite::params![file], |row| row.get(0)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Remove all edges where source_fqn starts with the given file prefix or equals the file path.
/// This handles both symbol edges (source_fqn = "file::symbol") and import edges (source_fqn = "file").
fn remove_edges_for_file(conn: &std::sync::MutexGuard<'_, rusqlite::Connection>, file: &str) {
    let pattern = format!("{}::%", file);
    let _ = conn.execute(
        "DELETE FROM edges WHERE source_fqn LIKE ?1 OR source_fqn = ?2",
        rusqlite::params![pattern, file],
    );
}

/// Handle deleted files: files in file_snapshots that are no longer on disk.
fn handle_deleted_files(
    repo_root: &Path,
    current_files: &[String],
    store: &StoreManager,
) -> Result<usize, IndexError> {
    let current_set: HashSet<&str> = current_files.iter().map(|s| s.as_str()).collect();

    // Get all tracked files from file_snapshots
    let tracked_files: Vec<String> = {
        let conn = store.read_conn();
        let mut stmt = conn
            .prepare("SELECT file FROM file_snapshots")
            .map_err(|e| IndexError::FileReadFailed {
                path: "file_snapshots".to_string(),
                reason: e.to_string(),
            })?;

        stmt.query_map([], |row| row.get(0))
            .map_err(|e| IndexError::FileReadFailed {
                path: "file_snapshots".to_string(),
                reason: e.to_string(),
            })?
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut deleted_count = 0;

    // Find files that are tracked but no longer on disk
    let deleted_files: Vec<&String> = tracked_files
        .iter()
        .filter(|f| {
            // Check if file is not in current set AND not on disk
            if current_set.contains(f.as_str()) {
                return false;
            }
            let abs_path = repo_root.join(f.replace('/', std::path::MAIN_SEPARATOR_STR));
            !abs_path.exists()
        })
        .collect();

    if deleted_files.is_empty() {
        return Ok(0);
    }

    let mut conn = store.write_conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    for file in &deleted_files {
        // Get nodes for this file
        let nodes_to_remove: Vec<String> = {
            let mut stmt = match conn.prepare("SELECT fqn FROM nodes WHERE file = ?1") {
                Ok(s) => s,
                Err(_) => continue,
            };
            stmt.query_map(rusqlite::params![file.as_str()], |row| row.get(0))
                .unwrap_or_else(|_| panic!("query failed"))
                .filter_map(|r| r.ok())
                .collect()
        };

        let delta = GraphDelta {
            nodes_to_add: vec![],
            nodes_to_remove,
            edges_to_add: vec![],
            file_snapshot: FileSnapshot {
                file: file.to_string(),
                file_hash: String::new(),
                node_count: 0,
                indexed_at: now,
            },
        };

        if apply_delta(&mut conn, &delta).is_ok() {
            deleted_count += 1;
        }

        // Remove the file snapshot entry
        let _ = conn.execute(
            "DELETE FROM file_snapshots WHERE file = ?1",
            rusqlite::params![file.as_str()],
        );
    }

    Ok(deleted_count)
}

/// Generate embeddings for Function/Class nodes and store them in the database.
///
/// This function is a no-op when:
/// - The semantic feature is not compiled in
/// - The ONNX model is not downloaded
///
/// When enabled, it generates 768-dim embeddings for each Function/Class node
/// using the node's FQN and first few lines of source code.
fn generate_embeddings_for_nodes(
    store: &StoreManager,
    nodes: &[crate::store::types::Node],
    repo_root: &Path,
) {
    // Determine data directory
    let data_dir = std::env::var("CORTEX_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let root = std::env::var("CORTEX_REPO_ROOT")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| repo_root.to_path_buf());
            root.join(".cortex")
        });

    // Check if model is available before attempting to create embedder
    if !crate::indexer::embedder::is_model_available(&data_dir) {
        return;
    }

    // Try to create the embedder
    let embedder = match crate::indexer::embedder::Embedder::new(&data_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    // Filter to Function, Method, and Class nodes only
    let embeddable_nodes: Vec<&crate::store::types::Node> = nodes
        .iter()
        .filter(|n| {
            n.kind == crate::store::types::NodeKind::Function
                || n.kind == crate::store::types::NodeKind::Method
                || n.kind == crate::store::types::NodeKind::Class
        })
        .collect();

    if embeddable_nodes.is_empty() {
        return;
    }

    tracing::info!(
        "Generating embeddings for {} Function/Class nodes",
        embeddable_nodes.len()
    );

    // Generate embeddings
    let mut entries: Vec<(String, Vec<f32>)> = Vec::new();

    for node in &embeddable_nodes {
        // Read source code for the node (first few lines)
        let code_snippet = read_node_source(repo_root, &node.file, node.start_line, node.end_line);
        let text = crate::indexer::embedder::prepare_node_text(&node.fqn, code_snippet.as_deref());

        match embedder.generate_embedding(&text) {
            Ok(embedding) => {
                entries.push((node.fqn.clone(), embedding));
            }
            Err(e) => {
                tracing::debug!(
                    "Failed to generate embedding for '{}': {e}",
                    crate::telemetry::sanitize_path(Path::new(&node.fqn))
                );
            }
        }
    }

    // Store embeddings in batch
    if !entries.is_empty() {
        let conn = store.write_conn();
        let batch: Vec<(&str, &[f32])> = entries
            .iter()
            .map(|(fqn, emb)| (fqn.as_str(), emb.as_slice()))
            .collect();

        match crate::store::queries::embeddings::store_embeddings_batch(&conn, &batch) {
            Ok(count) => {
                tracing::info!("Stored {count} embeddings");
            }
            Err(e) => {
                tracing::warn!("Failed to store embeddings: {e}");
            }
        }
    }
}

/// Read source code for a node given its file path and line range.
fn read_node_source(
    repo_root: &Path,
    file: &str,
    start_line: u32,
    end_line: u32,
) -> Option<String> {
    let file_path = repo_root.join(file);
    let source = fs::read_to_string(&file_path).ok()?;
    let lines: Vec<&str> = source.lines().collect();

    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());

    if start < end {
        // Take at most 10 lines to keep embedding input manageable
        let end_capped = end.min(start + 10);
        Some(lines[start..end_capped].join("\n"))
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations::run_migrations;
    use std::fs;
    use tempfile::TempDir;

    /// Create a StoreManager with migrations applied.
    fn setup_store(data_dir: &Path) -> StoreManager {
        let store = StoreManager::new(data_dir).expect("failed to create store");
        // Apply migrations
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).expect("failed to run migrations");
        drop(conn);
        store
    }

    /// Create a temp repo with some Python fixture files.
    fn create_fixture_repo(tmp: &TempDir) -> std::path::PathBuf {
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Create a Python file
        fs::write(
            repo.join("main.py"),
            r#"
def greet(name):
    print(f"Hello, {name}")

def main():
    greet("world")
"#,
        )
        .unwrap();

        // Create a subdirectory with another file
        fs::create_dir_all(repo.join("utils")).unwrap();
        fs::write(
            repo.join("utils").join("helpers.py"),
            r#"
def validate(data):
    return data is not None

class Validator:
    def check(self, value):
        return validate(value)
"#,
        )
        .unwrap();

        repo
    }

    // -----------------------------------------------------------------------
    // Test: Index fixture files
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_fixture_produces_nodes() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();

        // Should have scanned 2 files
        assert_eq!(stats.files_scanned, 2);
        // Should have indexed both files
        assert_eq!(stats.files_indexed, 2);
        // Should have added nodes
        assert!(stats.nodes_added > 0, "expected nodes to be added");
        // No files should be skipped on first run
        assert_eq!(stats.files_skipped, 0);
    }

    // -----------------------------------------------------------------------
    // Test: Idempotent second run
    // -----------------------------------------------------------------------

    #[test]
    fn test_idempotent_second_run() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        // First run
        let stats1 = index_repository(&repo, &store).unwrap();
        assert!(stats1.files_indexed > 0);

        // Second run - no changes
        let stats2 = index_repository(&repo, &store).unwrap();
        assert_eq!(stats2.files_indexed, 0, "no files should be re-indexed");
        assert_eq!(stats2.files_skipped, stats1.files_scanned);
        assert_eq!(stats2.nodes_added, 0);
    }

    // -----------------------------------------------------------------------
    // Test: Modified file delta
    // -----------------------------------------------------------------------

    #[test]
    fn test_modified_file_delta() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        // First run
        let stats1 = index_repository(&repo, &store).unwrap();
        assert_eq!(stats1.files_indexed, 2);

        // Modify one file
        fs::write(
            repo.join("main.py"),
            r#"
def greet(name):
    print(f"Hello, {name}")

def main():
    greet("world")

def new_function():
    pass
"#,
        )
        .unwrap();

        // Second run - should detect the change
        let stats2 = index_repository(&repo, &store).unwrap();
        assert_eq!(
            stats2.files_indexed, 1,
            "only modified file should be re-indexed"
        );
        assert_eq!(stats2.files_skipped, 1, "unchanged file should be skipped");
    }

    // -----------------------------------------------------------------------
    // Test: Deleted file cleanup
    // -----------------------------------------------------------------------

    #[test]
    fn test_deleted_file_cleanup() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        // First run
        let stats1 = index_repository(&repo, &store).unwrap();
        assert_eq!(stats1.files_indexed, 2);

        // Delete one file
        fs::remove_file(repo.join("utils").join("helpers.py")).unwrap();

        // Second run - should detect deletion
        let stats2 = index_repository(&repo, &store).unwrap();
        assert_eq!(stats2.files_deleted, 1, "deleted file should be detected");

        // Verify nodes for deleted file are gone
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE file = 'utils/helpers.py'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "nodes for deleted file should be removed");

        // Verify file snapshot is removed
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_snapshots WHERE file = 'utils/helpers.py'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            snap_count, 0,
            "file snapshot for deleted file should be removed"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Excluded directories are skipped
    // -----------------------------------------------------------------------

    #[test]
    fn test_excluded_directories_skipped() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);

        // Create files in excluded directories
        fs::create_dir_all(repo.join("node_modules")).unwrap();
        fs::write(repo.join("node_modules").join("lib.js"), "function x() {}").unwrap();

        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git").join("config.py"), "x = 1").unwrap();

        fs::create_dir_all(repo.join("__pycache__")).unwrap();
        fs::write(repo.join("__pycache__").join("mod.py"), "y = 2").unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();

        // Only the 2 fixture files should be scanned, not the excluded ones
        assert_eq!(stats.files_scanned, 2);
    }

    // -----------------------------------------------------------------------
    // Test: Gitignore patterns respected
    // -----------------------------------------------------------------------

    #[test]
    fn test_gitignore_patterns_respected() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);

        // Create a .gitignore that ignores *.log and build/
        fs::write(repo.join(".gitignore"), "*.py\nbuild/\n").unwrap();

        // Create a file that should be ignored
        fs::create_dir_all(repo.join("build")).unwrap();
        fs::write(repo.join("build").join("output.py"), "def x(): pass").unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();

        // All .py files should be ignored by the gitignore pattern
        assert_eq!(stats.files_scanned, 0);
    }

    // -----------------------------------------------------------------------
    // Test: Only recognized extensions are processed
    // -----------------------------------------------------------------------

    #[test]
    fn test_only_recognized_extensions_processed() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Create files with various extensions
        fs::write(repo.join("readme.md"), "# Hello").unwrap();
        fs::write(repo.join("data.json"), "{}").unwrap();
        fs::write(repo.join("config.toml"), "key = \"value\"").unwrap();
        fs::write(repo.join("main.py"), "def hello(): pass").unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();

        // Only main.py should be scanned (recognized extension)
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_indexed, 1);
    }

    // -----------------------------------------------------------------------
    // Test: Pattern matching helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_matches_any_pattern() {
        // Wildcard extension
        assert!(matches_any_pattern("src/file.log", &["*.log".to_string()]));
        assert!(!matches_any_pattern("src/file.py", &["*.log".to_string()]));

        // Directory pattern
        assert!(matches_any_pattern(
            "build/output.js",
            &["build/".to_string()]
        ));
        assert!(!matches_any_pattern(
            "src/build.py",
            &["build/".to_string()]
        ));

        // Simple name match
        assert!(matches_any_pattern(
            "src/temp/file.py",
            &["temp".to_string()]
        ));

        // Path pattern
        assert!(matches_any_pattern(
            "dist/bundle.js",
            &["dist/bundle.js".to_string()]
        ));
    }

    // -----------------------------------------------------------------------
    // Test: Module node created per file
    // -----------------------------------------------------------------------

    #[test]
    fn test_module_node_created_per_file() {
        let tmp = TempDir::new().unwrap();
        let repo = create_fixture_repo(&tmp);
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();
        assert_eq!(stats.files_indexed, 2);

        // Verify Module nodes exist for each file
        let conn = store.read_conn();
        let module_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE kind = 'Module'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            module_count, 2,
            "each indexed file should have a Module node"
        );

        // Verify Module node FQN equals the file path
        let module_fqn: String = conn
            .query_row(
                "SELECT fqn FROM nodes WHERE kind = 'Module' AND file = 'main.py'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            module_fqn, "main.py",
            "Module FQN should equal the file path"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Single-file change only resolves that file's edges (incremental FQN resolution)
    // -----------------------------------------------------------------------

    #[test]
    fn test_single_file_change_only_resolves_that_files_edges() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Create two Python files with cross-file calls
        fs::write(
            repo.join("main.py"),
            r#"
from utils import validate

def main():
    validate("hello")
"#,
        )
        .unwrap();

        fs::write(
            repo.join("utils.py"),
            r#"
def validate(data):
    return data is not None

def helper():
    validate("internal")
"#,
        )
        .unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        // First run: index both files
        let stats1 = index_repository(&repo, &store).unwrap();
        assert_eq!(stats1.files_indexed, 2);

        // Record edge count after first run
        let conn = store.read_conn();
        let initial_edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        drop(conn);

        // Modify only main.py (add a new function)
        fs::write(
            repo.join("main.py"),
            r#"
from utils import validate

def main():
    validate("hello")

def new_func():
    validate("new")
"#,
        )
        .unwrap();

        // Second run: only main.py should be re-indexed
        let stats2 = index_repository(&repo, &store).unwrap();
        assert_eq!(
            stats2.files_indexed, 1,
            "only modified file should be re-indexed"
        );
        assert_eq!(stats2.files_skipped, 1, "unchanged file should be skipped");

        // Verify the pipeline still produces valid edges
        let conn = store.read_conn();
        let final_edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        // Should have more edges now (new_func also calls validate)
        assert!(
            final_edge_count >= initial_edge_count,
            "edge count should not decrease after adding a function"
        );

        // Verify no orphaned edges (FK integrity)
        let orphan_edges: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE source_fqn NOT IN (SELECT fqn FROM nodes)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_edges, 0,
            "no edges should have orphaned source_fqn references"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Import edges reference Module node FQN as source_fqn
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_edges_reference_module_fqn() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Create a Rust file with use statements
        fs::write(
            repo.join("lib.rs"),
            r#"use std::collections::HashMap;
use serde::Serialize;

pub fn process() -> HashMap<String, String> {
    HashMap::new()
}
"#,
        )
        .unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();
        assert_eq!(stats.files_indexed, 1);

        // Verify Import edges have source_fqn = file path (Module node FQN)
        let conn = store.read_conn();
        let import_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE kind = 'Imports' AND source_fqn = 'lib.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            import_count > 0,
            "Import edges should reference the Module node FQN"
        );

        // Verify the Module node exists with matching FQN
        let module_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE fqn = 'lib.rs' AND kind = 'Module'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(module_exists, 1, "Module node should exist for the file");

        // Verify zero FK failures: all edges have valid source_fqn references
        let orphan_edges: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE source_fqn NOT IN (SELECT fqn FROM nodes)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_edges, 0,
            "no edges should have orphaned source_fqn references"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Index cortex source files with zero FK failures
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_cortex_source_zero_fk_failures() {
        // Index cortex's own source directory
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo_root, &store).unwrap();

        // Cortex has 80+ source files
        assert!(
            stats.files_scanned >= 50,
            "expected at least 50 cortex source files, got {}",
            stats.files_scanned
        );
        assert_eq!(
            stats.files_indexed, stats.files_scanned,
            "all scanned files should be indexed on first run"
        );

        // Verify zero FK failures: all edges have valid source_fqn references
        let conn = store.read_conn();
        let orphan_edges: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE source_fqn NOT IN (SELECT fqn FROM nodes)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_edges, 0,
            "all cortex source files should index with zero FK failures"
        );

        // Verify Module nodes exist for each indexed file
        let module_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE kind = 'Module'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            module_count, stats.files_indexed as i64,
            "each indexed file should have exactly one Module node"
        );

        // Verify total edges > 0 (imports should be successfully stored now)
        let total_edges: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert!(
            total_edges > 0,
            "expected edges to be stored (imports), got {}",
            total_edges
        );
    }

    // -----------------------------------------------------------------------
    // Test: Secret detection tags nodes with contains_secret attribute
    // -----------------------------------------------------------------------

    #[test]
    fn test_secret_detection_tags_nodes() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Create a Python file with an AWS key inside a function
        fs::write(
            repo.join("config.py"),
            r#"
def get_aws_config():
    access_key = "AKIAIOSFODNN7EXAMPLE"
    return access_key
"#,
        )
        .unwrap();

        let data_dir = tmp.path().join("data");
        let store = setup_store(&data_dir);

        let stats = index_repository(&repo, &store).unwrap();
        assert_eq!(stats.files_indexed, 1);

        // Verify the function node has contains_secret attribute
        let conn = store.read_conn();
        let attrs_str: String = conn
            .query_row(
                "SELECT attributes FROM nodes WHERE fqn LIKE '%get_aws_config%' AND kind = 'Function'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let attrs: serde_json::Value = serde_json::from_str(&attrs_str).unwrap();
        assert_eq!(
            attrs.get("contains_secret").and_then(|v| v.as_str()),
            Some("aws_access_key"),
            "function containing AWS key should be tagged with contains_secret"
        );
        assert!(
            attrs.get("secret_line").is_some(),
            "function should have secret_line attribute"
        );
    }

    // -----------------------------------------------------------------------
    // Property Tests: Default exclusion logic
    // -----------------------------------------------------------------------

    use proptest::prelude::*;

    /// Strategy to generate a valid path segment (alphanumeric, no dots at start,
    /// no path separators, and not matching any excluded dir name).
    /// Also excludes Windows reserved device names.
    fn arb_safe_segment() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9]{2,10}".prop_filter(
            "must not be an excluded dir or Windows reserved name",
            |s| {
                let upper = s.to_uppercase();
                !EXCLUDED_DIRS.contains(&s.as_str())
                    && ![
                        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                        "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
                        "LPT7", "LPT8", "LPT9",
                    ]
                    .contains(&upper.as_str())
            },
        )
    }

    /// Strategy to generate a valid filename that is NOT in EXCLUDED_FILES.
    fn arb_safe_filename() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9]{0,8}\\.(py|rs|js|ts)"
            .prop_filter("must not be an excluded file", |s| {
                !EXCLUDED_FILES.contains(&s.as_str())
            })
    }

    // **Validates: Requirements 3.1, 3.4**
    //
    // Property 3: Default directory exclusion
    //
    // Any path containing a segment matching an EXCLUDED_DIRS entry should be
    // excluded by `is_excluded_dir`.
    proptest! {
        #[test]
        fn prop_excluded_dir_detected(
            excluded_dir in proptest::sample::select(EXCLUDED_DIRS.to_vec()),
        ) {
            // Create a temp directory with the excluded dir name
            let tmp = TempDir::new().unwrap();
            let dir_path = tmp.path().join(excluded_dir);
            fs::create_dir_all(&dir_path).unwrap();

            // Walk and check that the excluded dir is detected
            let mut found = false;
            for entry in walkdir::WalkDir::new(tmp.path()).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_dir() {
                    let name = entry.file_name().to_string_lossy();
                    if name == excluded_dir {
                        found = true;
                        prop_assert!(
                            is_excluded_dir(&entry, tmp.path()),
                            "Directory '{}' should be excluded but was not",
                            name
                        );
                    }
                }
            }
            prop_assert!(found, "Should have found directory '{}'", excluded_dir);
        }

        /// Property 3 (completeness): All EXCLUDED_DIRS entries are excluded regardless
        /// of nesting depth.
        #[test]
        fn prop_excluded_dir_nested(
            excluded_dir in proptest::sample::select(EXCLUDED_DIRS.to_vec()),
            parent in arb_safe_segment(),
        ) {
            // Create a nested structure: parent/excluded_dir
            let tmp = TempDir::new().unwrap();
            let dir_path = tmp.path().join(&parent).join(excluded_dir);
            fs::create_dir_all(&dir_path).unwrap();

            // Walk and verify the excluded dir is detected even when nested
            let mut found = false;
            for entry in walkdir::WalkDir::new(tmp.path()).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_dir() {
                    let name = entry.file_name().to_string_lossy();
                    if name == excluded_dir {
                        found = true;
                        prop_assert!(
                            is_excluded_dir(&entry, tmp.path()),
                            "Nested directory '{}' under '{}' should be excluded but was not",
                            excluded_dir,
                            parent
                        );
                    }
                }
            }
            prop_assert!(found, "Should have found nested directory '{}'", excluded_dir);
        }

        /// Property 3 (inverse): Directories NOT in EXCLUDED_DIRS should NOT be excluded.
        #[test]
        fn prop_non_excluded_dir_not_detected(
            dir_name in arb_safe_segment(),
        ) {
            let tmp = TempDir::new().unwrap();
            let dir_path = tmp.path().join(&dir_name);
            fs::create_dir_all(&dir_path).unwrap();

            for entry in walkdir::WalkDir::new(tmp.path()).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_dir() && entry.file_name().to_string_lossy() == dir_name {
                    prop_assert!(
                        !is_excluded_dir(&entry, tmp.path()),
                        "Directory '{}' should NOT be excluded but was",
                        dir_name
                    );
                }
            }
        }
    }

    // **Validates: Requirements 3.2**
    //
    // Property 4: Default file exclusion
    //
    // Any file whose name matches an EXCLUDED_FILES entry should be excluded
    // from the walk results.
    proptest! {
        #[test]
        fn prop_excluded_file_not_in_walk_results(
            excluded_file in proptest::sample::select(EXCLUDED_FILES.to_vec()),
            subdir in arb_safe_segment(),
        ) {
            let tmp = TempDir::new().unwrap();
            let repo = tmp.path().join("repo");
            fs::create_dir_all(repo.join(&subdir)).unwrap();

            // Place the excluded file in the subdirectory
            fs::write(repo.join(&subdir).join(excluded_file), "content").unwrap();

            // Also place a valid source file so walk has something to find
            fs::write(repo.join(&subdir).join("valid.py"), "x = 1").unwrap();

            let gitignore_patterns: Vec<String> = Vec::new();
            let cortex_ignore_patterns: Vec<String> = Vec::new();

            let results = walk_directory(&repo, &gitignore_patterns, &cortex_ignore_patterns);

            // The excluded file should NOT appear in results
            for path in &results {
                let filename = path.split('/').next_back().unwrap_or("");
                prop_assert!(
                    !EXCLUDED_FILES.contains(&filename),
                    "Excluded file '{}' should not appear in walk results, but found path '{}'",
                    filename,
                    path
                );
            }
        }

        /// Property 4 (inverse): Files NOT in EXCLUDED_FILES should appear in walk results
        /// (assuming they have a recognized extension and are not otherwise ignored).
        #[test]
        fn prop_non_excluded_file_in_walk_results(
            filename in arb_safe_filename(),
        ) {
            let tmp = TempDir::new().unwrap();
            let repo = tmp.path().join("repo");
            fs::create_dir_all(&repo).unwrap();

            // Write a file with valid content and a recognized extension
            fs::write(repo.join(&filename), "def hello(): pass").unwrap();

            let gitignore_patterns: Vec<String> = Vec::new();
            let cortex_ignore_patterns: Vec<String> = Vec::new();

            let results = walk_directory(&repo, &gitignore_patterns, &cortex_ignore_patterns);

            prop_assert!(
                results.iter().any(|p| p.ends_with(&filename)),
                "Non-excluded file '{}' should appear in walk results but didn't. Results: {:?}",
                filename,
                results
            );
        }
    }

    // -----------------------------------------------------------------------
    // Property 1: Gitignore-style pattern matching
    // -----------------------------------------------------------------------

    /// Strategy to generate a valid file extension (1-4 lowercase alpha chars).
    fn arb_extension() -> impl Strategy<Value = String> {
        "[a-z]{1,4}"
    }

    /// Strategy to generate a valid path segment (alphanumeric, 1-12 chars).
    fn arb_path_segment() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,11}"
    }

    // Property 1: Gitignore-style pattern matching
    // Validates: Requirements 1.5, 2.1, 2.2
    //
    // Patterns loaded from .cortexignore should correctly match paths using
    // gitignore syntax: directory patterns (`dir/`), wildcard extension patterns
    // (`*.ext`), path prefix patterns (containing `/`), and simple name patterns.
    proptest! {
        /// Directory patterns ending in `/` match paths starting with the prefix.
        #[test]
        fn prop_pattern_dir_prefix_matches(
            dir_name in arb_path_segment(),
            sub_path_segments in proptest::collection::vec(arb_path_segment(), 1..=2),
            filename in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let pattern = format!("{}/", dir_name);
            let patterns = vec![pattern.clone()];

            // A path that starts with the directory prefix should match
            let matching_path = format!("{}/{}/{}.{}", dir_name, sub_path_segments.join("/"), filename, ext);
            prop_assert!(
                matches_any_pattern(&matching_path, &patterns),
                "Path '{}' should match directory pattern '{}'",
                matching_path, pattern
            );
        }

        /// Directory patterns ending in `/` match paths containing the dir as a component.
        #[test]
        fn prop_pattern_dir_component_matches(
            prefix_dir in arb_path_segment(),
            dir_name in arb_path_segment(),
            filename in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let pattern = format!("{}/", dir_name);
            let patterns = vec![pattern.clone()];

            // A path containing the directory as a nested component should match
            let matching_path = format!("{}/{}/{}.{}", prefix_dir, dir_name, filename, ext);
            prop_assert!(
                matches_any_pattern(&matching_path, &patterns),
                "Path '{}' should match directory pattern '{}' (nested component)",
                matching_path, pattern
            );
        }

        /// Wildcard extension patterns (`*.ext`) match paths ending with that extension.
        #[test]
        fn prop_pattern_wildcard_ext_matches(
            dirs in proptest::collection::vec(arb_path_segment(), 1..=3),
            filename in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let pattern = format!("*.{}", ext);
            let patterns = vec![pattern.clone()];

            // A path ending with the extension should match
            let matching_path = format!("{}/{}.{}", dirs.join("/"), filename, ext);
            prop_assert!(
                matches_any_pattern(&matching_path, &patterns),
                "Path '{}' should match wildcard pattern '{}'",
                matching_path, pattern
            );
        }

        /// Wildcard extension patterns do NOT match paths with a different extension.
        #[test]
        fn prop_pattern_wildcard_ext_rejects_different_ext(
            dirs in proptest::collection::vec(arb_path_segment(), 1..=3),
            filename in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let pattern = format!("*.{}", ext);
            let patterns = vec![pattern.clone()];

            // Use a guaranteed-different extension
            let other_ext = format!("{}x", ext);
            let non_matching_path = format!("{}/{}.{}", dirs.join("/"), filename, other_ext);
            prop_assert!(
                !matches_any_pattern(&non_matching_path, &patterns),
                "Path '{}' should NOT match wildcard pattern '{}'",
                non_matching_path, pattern
            );
        }

        /// Path prefix patterns (containing `/`) match paths starting with the prefix.
        #[test]
        fn prop_pattern_path_prefix_matches(
            dir in arb_path_segment(),
            file in arb_path_segment(),
            extra in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let pattern = format!("{}/{}", dir, file);
            let patterns = vec![pattern.clone()];

            // Exact match
            prop_assert!(
                matches_any_pattern(&pattern, &patterns),
                "Path '{}' should match path pattern '{}' (exact)",
                pattern, pattern
            );

            // Path starting with the pattern should match
            let extended_path = format!("{}/{}/{}.{}", dir, file, extra, ext);
            prop_assert!(
                matches_any_pattern(&extended_path, &patterns),
                "Path '{}' should match path pattern '{}' (prefix)",
                extended_path, pattern
            );
        }

        /// Simple name patterns match any path segment equal to the pattern.
        #[test]
        fn prop_pattern_simple_name_matches_segment(
            prefix_dir in arb_path_segment(),
            pattern_name in arb_path_segment(),
            suffix_file in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let patterns = vec![pattern_name.clone()];

            // Path containing the pattern as a segment should match
            let matching_path = format!("{}/{}/{}.{}", prefix_dir, pattern_name, suffix_file, ext);
            prop_assert!(
                matches_any_pattern(&matching_path, &patterns),
                "Path '{}' should match simple pattern '{}'",
                matching_path, pattern_name
            );
        }

        /// An empty pattern list matches nothing.
        #[test]
        fn prop_empty_patterns_match_nothing(
            path in arb_path_segment(),
            ext in arb_extension(),
        ) {
            let patterns: Vec<String> = Vec::new();
            let full_path = format!("{}.{}", path, ext);
            prop_assert!(
                !matches_any_pattern(&full_path, &patterns),
                "Empty patterns should match nothing, but matched '{}'",
                full_path
            );
        }
    }

    // -----------------------------------------------------------------------
    // Property 2: Pattern file parsing filters non-pattern lines
    // -----------------------------------------------------------------------

    // Property 2: Pattern file parsing filters non-pattern lines
    // Validates: Requirements 2.3, 2.4
    //
    // Comments (lines starting with #) and empty/whitespace lines should be
    // filtered out during parsing. Only valid pattern lines remain.
    proptest! {
        /// Mixed content: valid patterns, comments, and blanks. Only valid patterns
        /// should appear in the result.
        #[test]
        fn prop_parse_filters_comments_and_blanks(
            valid_patterns in proptest::collection::vec("[a-z][a-z0-9]{0,8}\\.[a-z]{1,3}", 1..=5),
            comment_bodies in proptest::collection::vec("[a-zA-Z0-9 ]{1,20}", 0..=4),
            blank_count in 0usize..=3,
            whitespace_lines in 0usize..=3,
        ) {
            let tmp = TempDir::new().unwrap();
            let repo = tmp.path().to_path_buf();

            // Build file content mixing valid patterns, comments, and blanks
            let mut lines: Vec<String> = Vec::new();

            // Add comment lines (prefixed with #)
            for body in &comment_bodies {
                lines.push(format!("# {}", body));
            }

            // Add blank lines
            for _ in 0..blank_count {
                lines.push(String::new());
            }

            // Add whitespace-only lines
            for _ in 0..whitespace_lines {
                lines.push("   ".to_string());
            }

            // Add valid patterns
            for pattern in &valid_patterns {
                lines.push(pattern.clone());
            }

            let content = lines.join("\n");
            fs::write(repo.join(".cortexignore"), &content).unwrap();

            let result = load_ignore_patterns(&repo, ".cortexignore");

            // Result should contain exactly the valid patterns
            prop_assert_eq!(
                result.len(),
                valid_patterns.len(),
                "Expected {} patterns but got {}. Result: {:?}",
                valid_patterns.len(), result.len(), result
            );

            // Each valid pattern should appear in the result
            for pattern in &valid_patterns {
                prop_assert!(
                    result.contains(pattern),
                    "Pattern '{}' should be in result {:?}",
                    pattern, result
                );
            }

            // No result line should start with '#'
            for line in &result {
                prop_assert!(
                    !line.starts_with('#'),
                    "Comment line '{}' should have been filtered",
                    line
                );
            }

            // No result line should be empty or whitespace-only
            for line in &result {
                prop_assert!(
                    !line.trim().is_empty(),
                    "Empty/whitespace line should have been filtered"
                );
            }
        }

        /// File with ONLY comments and blank lines should produce empty result.
        #[test]
        fn prop_parse_only_comments_and_blanks_returns_empty(
            comment_bodies in proptest::collection::vec("[a-zA-Z0-9 ]{1,20}", 1..=5),
            blank_count in 0usize..=5,
        ) {
            let tmp = TempDir::new().unwrap();
            let repo = tmp.path().to_path_buf();

            let mut lines: Vec<String> = Vec::new();
            for body in &comment_bodies {
                lines.push(format!("# {}", body));
            }
            for _ in 0..blank_count {
                lines.push(String::new());
            }

            let content = lines.join("\n");
            fs::write(repo.join(".cortexignore"), &content).unwrap();

            let result = load_ignore_patterns(&repo, ".cortexignore");

            prop_assert!(
                result.is_empty(),
                "File with only comments and blanks should produce empty result, got {:?}",
                result
            );
        }

        /// Missing file should produce empty result (no panic, no error).
        #[test]
        fn prop_parse_missing_file_returns_empty(
            filename in "[a-z]{1,8}"
        ) {
            let tmp = TempDir::new().unwrap();
            let repo = tmp.path().to_path_buf();
            // Don't create the file — it should not exist
            let result = load_ignore_patterns(&repo, &filename);
            prop_assert!(
                result.is_empty(),
                "Missing file should produce empty result, got {:?}",
                result
            );
        }
    }
}
