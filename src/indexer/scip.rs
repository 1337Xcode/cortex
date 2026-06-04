//! SCIP index ingestion for precise symbol resolution.
//!
//! This module handles discovering SCIP index files, computing SCIP coverage
//! metrics, and parsing SCIP protobuf data to create HIGH-confidence edges
//! in the graph database.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use prost::Message;
use thiserror::Error;
use tracing::warn;

use crate::store::confidence::EdgeSource;
use crate::store::db::StoreManager;

// ---------------------------------------------------------------------------
// SCIP Protobuf message definitions (manually defined to match SCIP spec)
// ---------------------------------------------------------------------------

/// Top-level SCIP index message.
#[derive(Clone, PartialEq, Message)]
pub struct ScipIndex {
    /// Metadata about the index.
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<ScipMetadata>,
    /// Documents in the index (one per source file).
    #[prost(message, repeated, tag = "2")]
    pub documents: Vec<ScipDocument>,
    /// External symbols referenced but not defined in this index.
    #[prost(message, repeated, tag = "3")]
    pub external_symbols: Vec<ScipSymbolInformation>,
}

/// Metadata about the SCIP index.
#[derive(Clone, PartialEq, Message)]
pub struct ScipMetadata {
    /// Version of the SCIP protocol.
    #[prost(int32, tag = "1")]
    pub version: i32,
    /// Tool that generated this index.
    #[prost(message, optional, tag = "2")]
    pub tool_info: Option<ScipToolInfo>,
    /// Root URI of the project.
    #[prost(string, tag = "3")]
    pub project_root: String,
    /// Text encoding used in the source files.
    #[prost(int32, tag = "4")]
    pub text_document_encoding: i32,
}

/// Information about the tool that generated the SCIP index.
#[derive(Clone, PartialEq, Message)]
pub struct ScipToolInfo {
    /// Name of the indexer tool.
    #[prost(string, tag = "1")]
    pub name: String,
    /// Version of the indexer tool.
    #[prost(string, tag = "2")]
    pub version: String,
    /// Additional arguments passed to the tool.
    #[prost(string, repeated, tag = "3")]
    pub arguments: Vec<String>,
}

/// A document in the SCIP index, representing a single source file.
#[derive(Clone, PartialEq, Message)]
pub struct ScipDocument {
    /// Language of the document.
    #[prost(string, tag = "4")]
    pub language: String,
    /// Relative path of the document from the project root.
    #[prost(string, tag = "1")]
    pub relative_path: String,
    /// Symbol occurrences in this document.
    #[prost(message, repeated, tag = "2")]
    pub occurrences: Vec<ScipOccurrence>,
    /// Symbol information defined in this document.
    #[prost(message, repeated, tag = "3")]
    pub symbols: Vec<ScipSymbolInformation>,
}

/// An occurrence of a symbol at a specific location.
#[derive(Clone, PartialEq, Message)]
pub struct ScipOccurrence {
    /// Position encoded as [startLine, startChar, endLine, endChar] or
    /// [startLine, startChar, endChar] when start and end are on the same line.
    #[prost(int32, repeated, tag = "1")]
    pub range: Vec<i32>,
    /// The symbol string (SCIP symbol format).
    #[prost(string, tag = "2")]
    pub symbol: String,
    /// Bitmask of SymbolRole values. Bit 0 = Definition.
    #[prost(int32, tag = "3")]
    pub symbol_roles: i32,
    /// Override documentation for this occurrence.
    #[prost(string, repeated, tag = "4")]
    pub override_documentation: Vec<String>,
    /// Syntax kind of this occurrence.
    #[prost(int32, tag = "5")]
    pub syntax_kind: i32,
    /// Diagnostics associated with this occurrence.
    #[prost(message, repeated, tag = "6")]
    pub diagnostics: Vec<ScipDiagnostic>,
}

/// Information about a symbol.
#[derive(Clone, PartialEq, Message)]
pub struct ScipSymbolInformation {
    /// The symbol string (SCIP symbol format).
    #[prost(string, tag = "1")]
    pub symbol: String,
    /// Documentation strings for this symbol.
    #[prost(string, repeated, tag = "3")]
    pub documentation: Vec<String>,
    /// Relationships to other symbols.
    #[prost(message, repeated, tag = "4")]
    pub relationships: Vec<ScipRelationship>,
}

/// A relationship between symbols.
#[derive(Clone, PartialEq, Message)]
pub struct ScipRelationship {
    /// The related symbol.
    #[prost(string, tag = "1")]
    pub symbol: String,
    /// Whether this is an implementation relationship.
    #[prost(bool, tag = "2")]
    pub is_implementation: bool,
    /// Whether this is a reference relationship.
    #[prost(bool, tag = "3")]
    pub is_reference: bool,
    /// Whether this is a type definition relationship.
    #[prost(bool, tag = "4")]
    pub is_type_definition: bool,
}

/// A diagnostic message associated with an occurrence.
#[derive(Clone, PartialEq, Message)]
pub struct ScipDiagnostic {
    /// Severity of the diagnostic.
    #[prost(int32, tag = "1")]
    pub severity: i32,
    /// Diagnostic code.
    #[prost(string, tag = "2")]
    pub code: String,
    /// Human-readable message.
    #[prost(string, tag = "3")]
    pub message: String,
}

// ---------------------------------------------------------------------------
// SCIP Symbol Role constants
// ---------------------------------------------------------------------------

/// Bitmask value indicating a definition occurrence (bit 0).
const SYMBOL_ROLE_DEFINITION: i32 = 1;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during SCIP index ingestion.
#[derive(Debug, Error)]
pub enum ScipError {
    /// The SCIP index file could not be read from disk.
    #[error("failed to read SCIP index at '{path}': {reason}")]
    FileReadFailed { path: String, reason: String },

    /// The SCIP index file contains invalid protobuf data.
    #[error("failed to parse SCIP protobuf at '{path}': {reason}")]
    ProtobufParseFailed { path: String, reason: String },

    /// A database operation failed during SCIP ingestion.
    #[error("database error during SCIP ingestion: {reason}")]
    DatabaseError { reason: String },
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Result of ingesting a SCIP index file.
#[derive(Debug, Clone, PartialEq)]
pub struct ScipIngestResult {
    /// Number of new edges created from SCIP data.
    pub edges_created: usize,
    /// Number of edges deduplicated (SCIP wins over tree-sitter).
    pub edges_deduplicated: usize,
    /// Number of files covered by SCIP data.
    pub files_covered: usize,
    /// Duration of the ingestion in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Known SCIP index file locations, checked in order.
const SCIP_INDEX_PATHS: &[&str] = &[".scip/index.scip", "index.scip", "dump.lsif"];

/// Attempt to ingest a SCIP index, returning `None` on any error.
///
/// This is the safe entry point for the indexing pipeline. It wraps
/// [`ingest_scip_index`] and catches all errors (file read failures,
/// malformed protobuf, database errors) without panicking. On failure
/// it logs a warning and returns `None`, ensuring that tree-sitter
/// edges already in the database remain intact.
///
/// # Returns
///
/// - `Some(ScipIngestResult)` on successful ingestion.
/// - `None` if any error occurs (with a warning logged).
pub fn try_ingest_scip(index_path: &Path, store: &StoreManager) -> Option<ScipIngestResult> {
    match ingest_scip_index(index_path, store) {
        Ok(result) => {
            tracing::info!(
                edges_created = result.edges_created,
                files_covered = result.files_covered,
                duration_ms = result.duration_ms,
                "SCIP ingestion completed successfully"
            );
            Some(result)
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %index_path.display(),
                "SCIP ingestion failed; continuing with tree-sitter data only"
            );
            None
        }
    }
}

/// Check known SCIP index locations and return the first found.
///
/// Checks (in order):
/// 1. `{repo_root}/.scip/index.scip`
/// 2. `{repo_root}/index.scip`
/// 3. `{repo_root}/dump.lsif`
///
/// Returns `None` if no SCIP index file exists at any known location.
pub fn find_scip_index(repo_root: &Path) -> Option<PathBuf> {
    for relative_path in SCIP_INDEX_PATHS {
        let candidate = repo_root.join(relative_path);
        if candidate.is_file() {
            tracing::debug!("Found SCIP index at: {}", candidate.display());
            return Some(candidate);
        }
    }
    None
}

/// Compute SCIP coverage percentage (files with SCIP data / total files).
///
/// Queries the `scip_coverage` table to count files with `has_scip_data = 1`,
/// and the `index_health` table for total `files_indexed`. Returns 0.0 if
/// no files are indexed or if the tables don't exist yet.
pub fn compute_scip_coverage(store: &StoreManager) -> f64 {
    let conn = store.read_conn();

    let total_files: i64 = conn
        .query_row(
            "SELECT files_indexed FROM index_health WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| {
            warn!("Failed to query index_health for files_indexed: {}", e);
            0
        });

    if total_files <= 0 {
        return 0.0;
    }

    let scip_files: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scip_coverage WHERE has_scip_data = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| {
            warn!("Failed to query scip_coverage count: {}", e);
            0
        });

    (scip_files as f64) / (total_files as f64)
}

/// Ingest a SCIP index file, creating edges with edge_source=scip, confidence=1.0.
///
/// Parses the SCIP protobuf, iterates over documents and occurrences,
/// identifies definitions and references, and creates directed edges from
/// reference sites to definition sites. Updates the `scip_coverage` table
/// for each file processed.
///
/// Handles partial SCIP coverage on a per-file basis: only files present
/// in the SCIP index get SCIP edges; other files retain their tree-sitter edges.
pub fn ingest_scip_index(
    index_path: &Path,
    store: &StoreManager,
) -> Result<ScipIngestResult, ScipError> {
    let start = Instant::now();

    // Step 1: Read the SCIP index file from disk.
    let bytes = std::fs::read(index_path).map_err(|e| ScipError::FileReadFailed {
        path: index_path.display().to_string(),
        reason: e.to_string(),
    })?;

    // Step 2: Parse the protobuf.
    let index = ScipIndex::decode(bytes.as_slice()).map_err(|e| {
        ScipError::ProtobufParseFailed {
            path: index_path.display().to_string(),
            reason: e.to_string(),
        }
    })?;

    // Step 3: Build a global definition map: symbol -> defining FQN.
    // We scan all documents to find definition occurrences first.
    let def_map = build_definition_map(&index);

    // Step 4: Create edges from references to definitions.
    let mut edges_created: usize = 0;
    let mut files_covered: usize = 0;

    let conn = store.write_conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Prepare the edge insert statement.
    let mut insert_edge_stmt = conn
        .prepare(
            "INSERT OR IGNORE INTO edges \
             (source_fqn, target_fqn, kind, confidence, edge_source, attributes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|e| ScipError::DatabaseError {
            reason: format!("failed to prepare edge insert: {}", e),
        })?;

    // Prepare the scip_coverage upsert statement.
    let mut upsert_coverage_stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO scip_coverage \
             (file, has_scip_data, symbols_resolved, indexed_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|e| ScipError::DatabaseError {
            reason: format!("failed to prepare coverage upsert: {}", e),
        })?;

    for document in &index.documents {
        let file_path = &document.relative_path;
        if file_path.is_empty() {
            continue;
        }

        let mut symbols_resolved: usize = 0;

        for occurrence in &document.occurrences {
            // Skip empty symbols.
            if occurrence.symbol.is_empty() {
                continue;
            }
            // Skip local symbols (they start with "local ").
            if occurrence.symbol.starts_with("local ") {
                continue;
            }

            let is_definition = (occurrence.symbol_roles & SYMBOL_ROLE_DEFINITION) != 0;

            if is_definition {
                // Definitions don't create edges themselves; they are targets.
                continue;
            }

            // This is a reference occurrence. Create an edge from the
            // reference site (caller) to the definition site (callee).
            let reference_fqn =
                build_reference_fqn(file_path, &occurrence.range);

            // Look up the definition for this symbol.
            if let Some(def_fqn) = def_map.get(&occurrence.symbol) {
                // Don't create self-referencing edges.
                if &reference_fqn == def_fqn {
                    continue;
                }

                match insert_edge_stmt.execute(rusqlite::params![
                    &reference_fqn,
                    def_fqn,
                    "Calls",
                    1.0f64,
                    EdgeSource::Scip.as_str(),
                    "{}",
                ]) {
                    Ok(_) => {
                        edges_created += 1;
                        symbols_resolved += 1;
                    }
                    Err(e) => {
                        // Gracefully skip edges that fail (e.g., FK constraint).
                        tracing::debug!(
                            reference = %reference_fqn,
                            definition = %def_fqn,
                            "skipping SCIP edge: {}",
                            e
                        );
                    }
                }
            }
        }

        // Update scip_coverage for this file.
        let has_scip = if symbols_resolved > 0 || !document.occurrences.is_empty() {
            1i32
        } else {
            0i32
        };

        if let Err(e) = upsert_coverage_stmt.execute(rusqlite::params![
            file_path,
            has_scip,
            symbols_resolved as i64,
            now,
        ]) {
            tracing::debug!(
                file = %file_path,
                "failed to update scip_coverage: {}",
                e
            );
        }

        if has_scip == 1 {
            files_covered += 1;
        }
    }

    // Drop the prepared statements and connection before deduplication
    // to avoid deadlock (deduplicate_scip_edges also acquires write_conn).
    drop(insert_edge_stmt);
    drop(upsert_coverage_stmt);
    drop(conn);

    // Step 5: Deduplicate — remove ast_direct edges where SCIP edges exist
    // for the same (source_fqn, target_fqn) pair.
    let edges_deduplicated = deduplicate_scip_edges(store);

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(ScipIngestResult {
        edges_created,
        edges_deduplicated,
        files_covered,
        duration_ms,
    })
}

/// Deduplicate edges after SCIP ingestion.
///
/// For each (source_fqn, target_fqn) pair where both a SCIP edge
/// (edge_source='scip') and a tree-sitter edge (edge_source='ast_direct')
/// exist, delete the tree-sitter edge. SCIP wins as the higher-confidence
/// source.
///
/// Returns the number of edges removed (deduplicated).
pub fn deduplicate_scip_edges(store: &StoreManager) -> usize {
    let conn = store.write_conn();

    let result = conn.execute(
        "DELETE FROM edges WHERE id IN (
            SELECT ast.id FROM edges ast
            INNER JOIN edges scip
                ON ast.source_fqn = scip.source_fqn
                AND ast.target_fqn = scip.target_fqn
            WHERE ast.edge_source = 'ast_direct'
                AND scip.edge_source = 'scip'
        )",
        [],
    );

    match result {
        Ok(count) => {
            if count > 0 {
                tracing::info!(
                    deduplicated = count,
                    "removed ast_direct edges superseded by SCIP edges"
                );
            }
            count
        }
        Err(e) => {
            warn!("failed to deduplicate SCIP edges: {}", e);
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a map from SCIP symbol strings to their definition FQNs.
///
/// Scans all documents in the index for definition occurrences (symbol_roles
/// has bit 0 set) and maps each symbol to a canonical FQN derived from the
/// file path and symbol name.
fn build_definition_map(index: &ScipIndex) -> HashMap<String, String> {
    let mut def_map: HashMap<String, String> = HashMap::new();

    for document in &index.documents {
        let file_path = &document.relative_path;
        if file_path.is_empty() {
            continue;
        }

        for occurrence in &document.occurrences {
            if occurrence.symbol.is_empty() {
                continue;
            }
            // Skip local symbols.
            if occurrence.symbol.starts_with("local ") {
                continue;
            }

            let is_definition = (occurrence.symbol_roles & SYMBOL_ROLE_DEFINITION) != 0;
            if is_definition {
                let fqn = symbol_to_fqn(&occurrence.symbol, file_path);
                def_map.insert(occurrence.symbol.clone(), fqn);
            }
        }

        // Also register symbols from the SymbolInformation entries.
        for sym_info in &document.symbols {
            if sym_info.symbol.is_empty() || sym_info.symbol.starts_with("local ") {
                continue;
            }
            // Only insert if not already present (occurrence-based takes priority).
            def_map
                .entry(sym_info.symbol.clone())
                .or_insert_with(|| symbol_to_fqn(&sym_info.symbol, file_path));
        }
    }

    // Also register external symbols (defined outside this index).
    for ext_sym in &index.external_symbols {
        if ext_sym.symbol.is_empty() || ext_sym.symbol.starts_with("local ") {
            continue;
        }
        def_map
            .entry(ext_sym.symbol.clone())
            .or_insert_with(|| symbol_to_fqn(&ext_sym.symbol, ""));
    }

    def_map
}

/// Convert a SCIP symbol string to a Cortex FQN.
///
/// SCIP symbols follow a structured format like:
///   `rust-analyzer cargo cortex 0.1.0 src/indexer/scip.rs/ingest_scip_index().`
///
/// We extract a simplified FQN by taking the last meaningful segments.
/// If the symbol contains path-like components, we use those.
/// Falls back to using the raw symbol with the file path as prefix.
fn symbol_to_fqn(symbol: &str, file_path: &str) -> String {
    // SCIP symbols are space-separated with a trailing period or hash.
    // Try to extract meaningful parts.
    let trimmed = symbol.trim();

    // If the symbol contains descriptors (segments separated by /),
    // extract the last meaningful parts to form an FQN.
    if let Some(fqn) = extract_fqn_from_scip_symbol(trimmed) {
        return fqn;
    }

    // Fallback: use file_path::symbol_name pattern.
    if file_path.is_empty() {
        trimmed.to_string()
    } else {
        format!("{}::{}", file_path, sanitize_symbol_name(trimmed))
    }
}

/// Attempt to extract a meaningful FQN from a SCIP symbol string.
///
/// SCIP symbol format: `<scheme> <package-manager> <package-name> <version> <descriptors>`
/// Descriptors use suffixes: `.` for term, `#` for type, `()` for method.
///
/// Example: `scip-python python pkg 1.0 src/main.py/MyClass#method().`
/// Becomes: `src/main.py::MyClass.method`
fn extract_fqn_from_scip_symbol(symbol: &str) -> Option<String> {
    // Split by spaces to get the parts.
    let parts: Vec<&str> = symbol.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    // The descriptor portion is the last part (or everything after the 4th space).
    // Typically: scheme manager package version descriptor...
    let descriptor_start = if parts.len() >= 5 { 4 } else { parts.len() - 1 };
    let descriptor = parts[descriptor_start..].join(" ");

    if descriptor.is_empty() {
        return None;
    }

    // Parse the descriptor path: segments separated by `/`
    // Each segment may end with `.` (term), `#` (type), `()` (method).
    let segments: Vec<&str> = descriptor.split('/').collect();
    if segments.is_empty() {
        return None;
    }

    // Find the first segment that looks like a file path
    let mut file_seg_idx = None;
    for (idx, segment) in segments.iter().enumerate() {
        let cleaned = segment
            .trim_end_matches('.')
            .trim_end_matches('#')
            .trim_end_matches("().");
        if looks_like_file_path(cleaned) {
            file_seg_idx = Some(idx);
            break;
        }
    }

    let mut file_part = String::new();
    let mut name_parts: Vec<String> = Vec::new();

    if let Some(file_idx) = file_seg_idx {
        // We found a file segment.
        // Everything up to file_idx is part of the file path.
        for idx in 0..=file_idx {
            let cleaned = segments[idx]
                .trim_end_matches('.')
                .trim_end_matches('#')
                .trim_end_matches("().");
            if !cleaned.is_empty() {
                if !file_part.is_empty() {
                    file_part.push('/');
                }
                file_part.push_str(cleaned);
            }
        }
        // Everything after file_idx is name parts.
        for idx in (file_idx + 1)..segments.len() {
            let cleaned = segments[idx]
                .trim_end_matches('.')
                .trim_end_matches('#')
                .trim_end_matches("().");
            let name = cleaned
                .trim_end_matches('.')
                .trim_end_matches('#')
                .trim_end_matches("()")
                .to_string();
            if !name.is_empty() {
                name_parts.push(name);
            }
        }
    } else {
        // No file segment found. Treat all non-empty segments as name parts.
        for segment in &segments {
            let cleaned = segment
                .trim_end_matches('.')
                .trim_end_matches('#')
                .trim_end_matches("().");
            let name = cleaned
                .trim_end_matches('.')
                .trim_end_matches('#')
                .trim_end_matches("()")
                .to_string();
            if !name.is_empty() {
                name_parts.push(name);
            }
        }
    }

    if name_parts.is_empty() && file_part.is_empty() {
        return None;
    }

    // Build the FQN.
    let fqn = if !file_part.is_empty() && !name_parts.is_empty() {
        format!("{}::{}", file_part, name_parts.join("."))
    } else if !file_part.is_empty() {
        file_part
    } else {
        name_parts.join(".")
    };

    Some(fqn)
}

/// Check if a string looks like a file path (has a known source extension).
fn looks_like_file_path(s: &str) -> bool {
    let extensions = [
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".java", ".go", ".rb",
        ".c", ".cpp", ".h", ".hpp", ".cs", ".kt", ".scala", ".swift",
        ".php", ".ex", ".exs", ".zig", ".lua", ".dart",
    ];
    extensions.iter().any(|ext| s.ends_with(ext))
}

/// Sanitize a SCIP symbol name for use in an FQN.
/// Removes trailing punctuation used in SCIP descriptor format.
fn sanitize_symbol_name(s: &str) -> String {
    s.replace("().", "")
        .replace("()", "")
        .trim_end_matches('.')
        .trim_end_matches('#')
        .replace('/', ".")
        .to_string()
}

/// Build an FQN for a reference occurrence based on file path and position.
///
/// For references, we construct an FQN like `file_path::L{line}` since
/// the reference site doesn't have its own symbol name — it's the call site.
/// This matches how tree-sitter edges use the caller's FQN.
fn build_reference_fqn(file_path: &str, range: &[i32]) -> String {
    let line = range.first().copied().unwrap_or(0);
    format!("{}::L{}", file_path, line)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a StoreManager with migrations applied.
    fn setup_store_with_migrations() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");

        let conn = store.write_conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS nodes (
                fqn TEXT PRIMARY KEY,
                kind TEXT NOT NULL DEFAULT 'Function',
                file TEXT NOT NULL DEFAULT '',
                start_line INTEGER NOT NULL DEFAULT 0,
                end_line INTEGER NOT NULL DEFAULT 0,
                file_hash TEXT NOT NULL DEFAULT '',
                indexed_at INTEGER NOT NULL DEFAULT 0,
                attributes TEXT DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_fqn TEXT NOT NULL,
                target_fqn TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN (
                    'Calls','Imports','Inherits','Implements','HttpLink','DataFlow',
                    'Injects','Middleware','Routes','Renders'
                )),
                confidence REAL NOT NULL DEFAULT 0.5
                    CHECK(confidence >= 0.0 AND confidence <= 1.0),
                edge_source TEXT NOT NULL DEFAULT 'ast_direct'
                    CHECK(edge_source IN ('scip','framework_adapter','ast_direct','name_match')),
                attributes TEXT DEFAULT '{}',
                UNIQUE(source_fqn, target_fqn, kind, edge_source)
            );

            CREATE TABLE IF NOT EXISTS scip_coverage (
                file TEXT PRIMARY KEY,
                has_scip_data INTEGER NOT NULL DEFAULT 0,
                symbols_resolved INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS index_health (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                files_indexed INTEGER NOT NULL DEFAULT 0,
                node_count INTEGER NOT NULL DEFAULT 0,
                edge_count INTEGER NOT NULL DEFAULT 0,
                scip_coverage_percent REAL NOT NULL DEFAULT 0.0,
                last_index_at INTEGER NOT NULL DEFAULT 0,
                frameworks_detected TEXT NOT NULL DEFAULT '[]',
                health_status TEXT NOT NULL DEFAULT 'unknown'
            );

            INSERT OR IGNORE INTO index_health
                (id, files_indexed, node_count, edge_count, last_index_at)
                VALUES (1, 0, 0, 0, 0);",
        )
        .expect("failed to apply test migrations");
        drop(conn);

        (store, tmp)
    }

    /// Helper to create a minimal valid SCIP index protobuf with given documents.
    fn create_scip_index_bytes(documents: Vec<ScipDocument>) -> Vec<u8> {
        let index = ScipIndex {
            metadata: Some(ScipMetadata {
                version: 1,
                tool_info: Some(ScipToolInfo {
                    name: "test-indexer".to_string(),
                    version: "1.0.0".to_string(),
                    arguments: vec![],
                }),
                project_root: "file:///test".to_string(),
                text_document_encoding: 0,
            }),
            documents,
            external_symbols: vec![],
        };
        index.encode_to_vec()
    }

    // ─── find_scip_index tests ───────────────────────────────────────────────

    #[test]
    fn find_scip_index_returns_none_when_no_files_exist() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(find_scip_index(tmp.path()), None);
    }

    #[test]
    fn find_scip_index_finds_dot_scip_index() {
        let tmp = TempDir::new().unwrap();
        let scip_dir = tmp.path().join(".scip");
        fs::create_dir_all(&scip_dir).unwrap();
        fs::write(scip_dir.join("index.scip"), b"fake scip data").unwrap();

        let result = find_scip_index(tmp.path());
        assert_eq!(result, Some(scip_dir.join("index.scip")));
    }

    #[test]
    fn find_scip_index_finds_root_index_scip() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("index.scip"), b"fake scip data").unwrap();

        let result = find_scip_index(tmp.path());
        assert_eq!(result, Some(tmp.path().join("index.scip")));
    }

    #[test]
    fn find_scip_index_finds_dump_lsif() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("dump.lsif"), b"fake lsif data").unwrap();

        let result = find_scip_index(tmp.path());
        assert_eq!(result, Some(tmp.path().join("dump.lsif")));
    }

    #[test]
    fn find_scip_index_prefers_dot_scip_over_root() {
        let tmp = TempDir::new().unwrap();
        let scip_dir = tmp.path().join(".scip");
        fs::create_dir_all(&scip_dir).unwrap();
        fs::write(scip_dir.join("index.scip"), b"preferred").unwrap();
        fs::write(tmp.path().join("index.scip"), b"fallback").unwrap();

        let result = find_scip_index(tmp.path());
        assert_eq!(result, Some(scip_dir.join("index.scip")));
    }

    #[test]
    fn find_scip_index_ignores_directories_with_same_name() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("index.scip")).unwrap();

        let result = find_scip_index(tmp.path());
        assert_eq!(result, None);
    }

    // ─── compute_scip_coverage tests ─────────────────────────────────────────

    #[test]
    fn compute_scip_coverage_returns_zero_when_no_files_indexed() {
        let (store, _tmp) = setup_store_with_migrations();
        let coverage = compute_scip_coverage(&store);
        assert_eq!(coverage, 0.0);
    }

    #[test]
    fn compute_scip_coverage_returns_zero_when_no_scip_data() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        conn.execute(
            "UPDATE index_health SET files_indexed = 10 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(conn);

        let coverage = compute_scip_coverage(&store);
        assert_eq!(coverage, 0.0);
    }

    #[test]
    fn compute_scip_coverage_computes_correct_ratio() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        conn.execute(
            "UPDATE index_health SET files_indexed = 10 WHERE id = 1",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scip_coverage (file, has_scip_data, symbols_resolved, indexed_at) \
             VALUES ('a.py', 1, 5, 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scip_coverage (file, has_scip_data, symbols_resolved, indexed_at) \
             VALUES ('b.py', 1, 3, 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scip_coverage (file, has_scip_data, symbols_resolved, indexed_at) \
             VALUES ('c.py', 1, 7, 1000)",
            [],
        )
        .unwrap();
        drop(conn);

        let coverage = compute_scip_coverage(&store);
        assert!((coverage - 0.3).abs() < f64::EPSILON);
    }

    // ─── ingest_scip_index tests ─────────────────────────────────────────────

    #[test]
    fn ingest_scip_index_with_empty_index() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Write an empty but valid SCIP index.
        let bytes = create_scip_index_bytes(vec![]);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        assert_eq!(result.edges_created, 0);
        assert_eq!(result.files_covered, 0);
    }

    #[test]
    fn ingest_scip_index_creates_edges_from_references_to_definitions() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Create a SCIP index with a definition and a reference.
        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "src/main.py".to_string(),
            occurrences: vec![
                // Definition of `foo` at line 5.
                ScipOccurrence {
                    range: vec![5, 0, 3],
                    symbol: "scip-python python pkg 1.0 src/main.py/foo().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                // Reference to `foo` at line 10.
                ScipOccurrence {
                    range: vec![10, 4, 7],
                    symbol: "scip-python python pkg 1.0 src/main.py/foo().".to_string(),
                    symbol_roles: 0, // Reference (no definition bit)
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        assert_eq!(result.edges_created, 1);
        assert_eq!(result.files_covered, 1);
        assert!(result.duration_ms < 5000); // Should be fast
    }

    #[test]
    fn ingest_scip_index_skips_local_symbols() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "src/main.py".to_string(),
            occurrences: vec![
                // Local definition.
                ScipOccurrence {
                    range: vec![1, 0, 3],
                    symbol: "local 0".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                // Local reference.
                ScipOccurrence {
                    range: vec![5, 0, 3],
                    symbol: "local 0".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        // Local symbols should be skipped entirely.
        assert_eq!(result.edges_created, 0);
    }

    #[test]
    fn ingest_scip_index_handles_cross_file_references() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![
            // File with definition.
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/lib.py".to_string(),
                occurrences: vec![ScipOccurrence {
                    range: vec![1, 0, 10],
                    symbol: "scip-python python pkg 1.0 src/lib.py/helper().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                }],
                symbols: vec![],
            },
            // File with reference.
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/main.py".to_string(),
                occurrences: vec![ScipOccurrence {
                    range: vec![3, 4, 10],
                    symbol: "scip-python python pkg 1.0 src/lib.py/helper().".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                }],
                symbols: vec![],
            },
        ];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        assert_eq!(result.edges_created, 1);
        assert_eq!(result.files_covered, 2);
    }

    #[test]
    fn ingest_scip_index_sets_correct_edge_source_and_confidence() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![ScipDocument {
            language: "rust".to_string(),
            relative_path: "src/main.rs".to_string(),
            occurrences: vec![
                ScipOccurrence {
                    range: vec![1, 0, 5],
                    symbol: "rust-analyzer cargo test 0.1 src/main.rs/run().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                ScipOccurrence {
                    range: vec![10, 4, 7],
                    symbol: "rust-analyzer cargo test 0.1 src/main.rs/run().".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        ingest_scip_index(&index_path, &store).unwrap();

        // Verify the edge was created with correct source and confidence.
        let conn = store.read_conn();
        let (confidence, edge_source): (f64, String) = conn
            .query_row(
                "SELECT confidence, edge_source FROM edges LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(confidence, 1.0);
        assert_eq!(edge_source, "scip");
    }

    #[test]
    fn ingest_scip_index_updates_scip_coverage_table() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/a.py".to_string(),
                occurrences: vec![ScipOccurrence {
                    range: vec![1, 0, 3],
                    symbol: "scip-python python p 1 src/a.py/foo().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                }],
                symbols: vec![],
            },
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/b.py".to_string(),
                occurrences: vec![],
                symbols: vec![],
            },
        ];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        ingest_scip_index(&index_path, &store).unwrap();

        // Check scip_coverage entries.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM scip_coverage", [], |row| row.get(0))
            .unwrap();
        // Both files should have coverage entries.
        assert_eq!(count, 2);
    }

    #[test]
    fn ingest_scip_index_fails_on_nonexistent_file() {
        let (store, _tmp) = setup_store_with_migrations();
        let result = ingest_scip_index(Path::new("/nonexistent/index.scip"), &store);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScipError::FileReadFailed { path, .. } => {
                assert!(path.contains("nonexistent"));
            }
            other => panic!("expected FileReadFailed, got: {:?}", other),
        }
    }

    #[test]
    fn ingest_scip_index_fails_on_invalid_protobuf() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Write garbage bytes that aren't valid protobuf.
        fs::write(&index_path, b"this is not valid protobuf data!!!").unwrap();

        let result = ingest_scip_index(&index_path, &store);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScipError::ProtobufParseFailed { .. } => {}
            other => panic!("expected ProtobufParseFailed, got: {:?}", other),
        }
    }

    #[test]
    fn ingest_scip_index_skips_empty_symbol_occurrences() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "src/main.py".to_string(),
            occurrences: vec![
                // Empty symbol should be skipped.
                ScipOccurrence {
                    range: vec![1, 0, 3],
                    symbol: "".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        assert_eq!(result.edges_created, 0);
    }

    #[test]
    fn ingest_scip_index_skips_documents_with_empty_path() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "".to_string(), // Empty path
            occurrences: vec![ScipOccurrence {
                range: vec![1, 0, 3],
                symbol: "scip-python python p 1 foo().".to_string(),
                symbol_roles: SYMBOL_ROLE_DEFINITION,
                override_documentation: vec![],
                syntax_kind: 0,
                diagnostics: vec![],
            }],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        assert_eq!(result.files_covered, 0);
    }

    #[test]
    fn ingest_scip_index_multiple_references_to_same_definition() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        let sym = "scip-python python pkg 1.0 src/lib.py/helper().".to_string();
        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "src/main.py".to_string(),
            occurrences: vec![
                // Definition.
                ScipOccurrence {
                    range: vec![1, 0, 6],
                    symbol: sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                // Reference at line 5.
                ScipOccurrence {
                    range: vec![5, 4, 10],
                    symbol: sym.clone(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                // Reference at line 10.
                ScipOccurrence {
                    range: vec![10, 4, 10],
                    symbol: sym.clone(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        // Two references should create two edges.
        assert_eq!(result.edges_created, 2);
    }

    // ─── deduplicate_scip_edges tests ───────────────────────────────────────

    #[test]
    fn deduplicate_removes_ast_direct_when_scip_exists() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        // Insert an ast_direct edge.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('src/main.py::L10', 'src/lib.py::helper', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        // Insert a SCIP edge for the same pair.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('src/main.py::L10', 'src/lib.py::helper', 'Calls', 1.0, 'scip')",
            [],
        )
        .unwrap();
        drop(conn);

        let deduped = deduplicate_scip_edges(&store);
        assert_eq!(deduped, 1);

        // Only the SCIP edge should remain.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let edge_source: String = conn
            .query_row("SELECT edge_source FROM edges LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(edge_source, "scip");
    }

    #[test]
    fn deduplicate_keeps_ast_direct_when_no_scip_exists() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        // Insert only an ast_direct edge (no SCIP counterpart).
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('src/main.py::L5', 'src/utils.py::parse', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        drop(conn);

        let deduped = deduplicate_scip_edges(&store);
        assert_eq!(deduped, 0);

        // The ast_direct edge should still be there.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn deduplicate_handles_multiple_pairs() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        // Pair 1: both SCIP and ast_direct exist.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('a', 'b', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('a', 'b', 'Calls', 1.0, 'scip')",
            [],
        )
        .unwrap();
        // Pair 2: both SCIP and ast_direct exist.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('c', 'd', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('c', 'd', 'Calls', 1.0, 'scip')",
            [],
        )
        .unwrap();
        // Pair 3: only ast_direct (no SCIP) — should NOT be removed.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('e', 'f', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        drop(conn);

        let deduped = deduplicate_scip_edges(&store);
        assert_eq!(deduped, 2);

        // 2 SCIP edges + 1 remaining ast_direct = 3 total.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn deduplicate_does_not_remove_framework_adapter_edges() {
        let (store, _tmp) = setup_store_with_migrations();

        let conn = store.write_conn();
        // A framework_adapter edge and a SCIP edge for the same pair.
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('x', 'y', 'Calls', 0.8, 'framework_adapter')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('x', 'y', 'Calls', 1.0, 'scip')",
            [],
        )
        .unwrap();
        drop(conn);

        let deduped = deduplicate_scip_edges(&store);
        // framework_adapter edges should NOT be removed by deduplication.
        assert_eq!(deduped, 0);

        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn deduplicate_on_empty_table() {
        let (store, _tmp) = setup_store_with_migrations();
        let deduped = deduplicate_scip_edges(&store);
        assert_eq!(deduped, 0);
    }

    #[test]
    fn ingest_scip_index_deduplicates_existing_ast_direct_edges() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Pre-insert an ast_direct edge that will conflict with SCIP.
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('src/main.py::L10', 'src/lib.py::helper', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        drop(conn);

        // Create a SCIP index that produces an edge for the same pair.
        let documents = vec![
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/lib.py".to_string(),
                occurrences: vec![ScipOccurrence {
                    range: vec![1, 0, 6],
                    symbol: "scip-python python pkg 1.0 src/lib.py/helper().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                }],
                symbols: vec![],
            },
            ScipDocument {
                language: "python".to_string(),
                relative_path: "src/main.py".to_string(),
                occurrences: vec![ScipOccurrence {
                    range: vec![10, 4, 10],
                    symbol: "scip-python python pkg 1.0 src/lib.py/helper().".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                }],
                symbols: vec![],
            },
        ];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = ingest_scip_index(&index_path, &store).unwrap();
        // The SCIP edge was created and the ast_direct edge was deduplicated.
        assert_eq!(result.edges_created, 1);
        assert_eq!(result.edges_deduplicated, 1);

        // Only the SCIP edge should remain for this pair.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE source_fqn = 'src/main.py::L10' \
                 AND target_fqn = 'src/lib.py::helper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let edge_source: String = conn
            .query_row(
                "SELECT edge_source FROM edges WHERE source_fqn = 'src/main.py::L10' \
                 AND target_fqn = 'src/lib.py::helper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edge_source, "scip");
    }

    // ─── Helper function tests ───────────────────────────────────────────────

    #[test]
    fn symbol_to_fqn_with_standard_scip_symbol() {
        let fqn = symbol_to_fqn(
            "scip-python python pkg 1.0 src/main.py/MyClass#method().",
            "src/main.py",
        );
        // Should extract meaningful FQN from the symbol.
        assert!(!fqn.is_empty());
        assert!(fqn.contains("main.py") || fqn.contains("MyClass") || fqn.contains("method"));
    }

    #[test]
    fn symbol_to_fqn_fallback_for_simple_symbol() {
        let fqn = symbol_to_fqn("x", "src/main.py");
        assert_eq!(fqn, "src/main.py::x");
    }

    #[test]
    fn build_reference_fqn_uses_line_number() {
        let fqn = build_reference_fqn("src/main.py", &[42, 4, 10]);
        assert_eq!(fqn, "src/main.py::L42");
    }

    #[test]
    fn build_reference_fqn_handles_empty_range() {
        let fqn = build_reference_fqn("src/main.py", &[]);
        assert_eq!(fqn, "src/main.py::L0");
    }

    // ─── try_ingest_scip tests ───────────────────────────────────────────────

    #[test]
    fn try_ingest_scip_returns_none_on_invalid_protobuf() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Write garbage bytes that aren't valid protobuf.
        fs::write(&index_path, b"this is not valid protobuf data at all!!!").unwrap();

        // Pre-insert a tree-sitter edge to verify it remains intact.
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('src/app.py::L5', 'src/utils.py::parse', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        drop(conn);

        // Should return None without panicking.
        let result = try_ingest_scip(&index_path, &store);
        assert_eq!(result, None);

        // Tree-sitter edges must remain intact.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE edge_source = 'ast_direct'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn try_ingest_scip_returns_none_on_nonexistent_file() {
        let (store, _tmp) = setup_store_with_migrations();

        // Should return None without panicking.
        let result = try_ingest_scip(Path::new("/nonexistent/path/index.scip"), &store);
        assert_eq!(result, None);
    }

    #[test]
    fn try_ingest_scip_returns_some_on_valid_index() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Create a valid SCIP index with a definition and reference.
        let documents = vec![ScipDocument {
            language: "python".to_string(),
            relative_path: "src/main.py".to_string(),
            occurrences: vec![
                ScipOccurrence {
                    range: vec![1, 0, 3],
                    symbol: "scip-python python pkg 1.0 src/main.py/foo().".to_string(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                ScipOccurrence {
                    range: vec![10, 4, 7],
                    symbol: "scip-python python pkg 1.0 src/main.py/foo().".to_string(),
                    symbol_roles: 0,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }];

        let bytes = create_scip_index_bytes(documents);
        fs::write(&index_path, &bytes).unwrap();

        let result = try_ingest_scip(&index_path, &store);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.edges_created, 1);
        assert_eq!(result.files_covered, 1);
    }

    #[test]
    fn try_ingest_scip_preserves_existing_edges_on_failure() {
        let (store, _tmp) = setup_store_with_migrations();
        let tmp_dir = TempDir::new().unwrap();
        let index_path = tmp_dir.path().join("index.scip");

        // Pre-insert multiple tree-sitter edges.
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('a.py::L1', 'b.py::func', 'Calls', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('c.py::L3', 'd.py::Class', 'Imports', 0.5, 'ast_direct')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
             VALUES ('e.py::L7', 'f.py::helper', 'Calls', 0.8, 'framework_adapter')",
            [],
        )
        .unwrap();
        drop(conn);

        // Write invalid protobuf to trigger failure.
        fs::write(&index_path, b"\x00\x01\x02\x03 garbage protobuf").unwrap();

        let result = try_ingest_scip(&index_path, &store);
        assert_eq!(result, None);

        // All pre-existing edges must remain intact.
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    // ─── Property-based tests ────────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy to generate valid FQN-like strings for source and target.
    /// Produces strings like "src/foo.py::L42" or "src/bar.rs::helper".
    fn arb_fqn() -> impl Strategy<Value = String> {
        let file_part = prop_oneof![
            Just("src/main.py".to_string()),
            Just("src/lib.rs".to_string()),
            Just("src/utils.ts".to_string()),
            Just("pkg/handler.go".to_string()),
            Just("app/models.py".to_string()),
        ]
        .boxed();
        let symbol_part = prop_oneof![
            Just("L1".to_string()),
            Just("L42".to_string()),
            Just("func".to_string()),
            Just("MyClass".to_string()),
            Just("helper".to_string()),
            Just("parse".to_string()),
            Just("connect".to_string()),
            Just("run".to_string()),
        ]
        .boxed();
        (file_part, symbol_part).prop_map(|(f, s)| format!("{}::{}", f, s))
    }

    /// Strategy to generate a valid edge kind string accepted by the schema.
    fn arb_edge_kind() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just("Calls"),
            Just("Imports"),
            Just("Inherits"),
            Just("Implements"),
            Just("HttpLink"),
            Just("DataFlow"),
            Just("Injects"),
            Just("Middleware"),
            Just("Routes"),
            Just("Renders"),
        ]
    }

    // **Feature: cortex-intelligence-overhaul**
    // **Property 2: SCIP precedence and deduplication**
    // **Validates: Requirements 1.2, 1.4, 1.7**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// For any (source_fqn, target_fqn) pair where both a SCIP edge and a
        /// tree-sitter edge exist, after deduplication only the SCIP edge
        /// (edge_source=scip, confidence=1.0) SHALL remain as the primary edge,
        /// and the edge direction SHALL be from reference site to definition site.
        ///
        /// **Feature: cortex-intelligence-overhaul**
        /// **Property: SCIP precedence and deduplication**
        /// **Validates: Requirements 1.2, 1.4, 1.7**
        #[test]
        fn prop_scip_precedence_and_deduplication(
            source_fqn in arb_fqn(),
            target_fqn in arb_fqn(),
            kind in arb_edge_kind(),
        ) {
            // Ensure source and target are different.
            prop_assume!(source_fqn != target_fqn);

            let (store, _tmp) = setup_store_with_migrations();

            let conn = store.write_conn();

            // Insert an ast_direct (tree-sitter) edge with LOW confidence.
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES (?1, ?2, ?3, 0.5, 'ast_direct')",
                rusqlite::params![&source_fqn, &target_fqn, kind],
            )
            .unwrap();

            // Insert a SCIP edge for the same (source_fqn, target_fqn) pair
            // with HIGH confidence. Direction is from reference site (source_fqn)
            // to definition site (target_fqn).
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES (?1, ?2, ?3, 1.0, 'scip')",
                rusqlite::params![&source_fqn, &target_fqn, kind],
            )
            .unwrap();
            drop(conn);

            // Run deduplication.
            let deduped = deduplicate_scip_edges(&store);

            // Exactly one ast_direct edge should have been removed.
            prop_assert_eq!(
                deduped, 1,
                "Expected 1 edge deduplicated, got {}",
                deduped
            );

            // Verify only one edge remains for this pair.
            let conn = store.read_conn();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM edges WHERE source_fqn = ?1 AND target_fqn = ?2",
                    rusqlite::params![&source_fqn, &target_fqn],
                    |row| row.get(0),
                )
                .unwrap();
            prop_assert_eq!(
                count, 1,
                "Expected 1 edge remaining, got {}",
                count
            );

            // The remaining edge must be the SCIP edge.
            let (remaining_source, remaining_confidence): (String, f64) = conn
                .query_row(
                    "SELECT edge_source, confidence FROM edges \
                     WHERE source_fqn = ?1 AND target_fqn = ?2",
                    rusqlite::params![&source_fqn, &target_fqn],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            prop_assert_eq!(
                remaining_source.as_str(), "scip",
                "Remaining edge should be SCIP, got '{}'",
                remaining_source
            );
            prop_assert!(
                (remaining_confidence - 1.0).abs() < f64::EPSILON,
                "SCIP edge confidence should be 1.0, got {}",
                remaining_confidence
            );

            // Verify edge direction: source_fqn is the reference site,
            // target_fqn is the definition site. The edge stored in the DB
            // must preserve this direction (source_fqn → target_fqn).
            let (stored_source, stored_target): (String, String) = conn
                .query_row(
                    "SELECT source_fqn, target_fqn FROM edges \
                     WHERE source_fqn = ?1 AND target_fqn = ?2",
                    rusqlite::params![&source_fqn, &target_fqn],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            prop_assert_eq!(
                stored_source, source_fqn,
                "Edge source_fqn (reference site) mismatch"
            );
            prop_assert_eq!(
                stored_target, target_fqn,
                "Edge target_fqn (definition site) mismatch"
            );
        }
    }

    // **Feature: cortex-intelligence-overhaul**
    // **Property 3: Malformed SCIP graceful recovery**
    // **Validates: Requirements 2.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// For any byte sequence presented as a SCIP index file that is not valid
        /// protobuf, the SCIP ingester SHALL return successfully (not panic or
        /// error-propagate) and the tree-sitter edges for all files SHALL remain
        /// intact in the database.
        ///
        /// **Feature: cortex-intelligence-overhaul**
        /// **Property: Malformed SCIP graceful recovery**
        /// **Validates: Requirements 2.3**
        #[test]
        fn prop_malformed_scip_graceful_recovery(data in prop::collection::vec(any::<u8>(), 0..1024)) {
            // Set up a store with pre-existing tree-sitter edges.
            let (store, _tmp) = setup_store_with_migrations();

            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES ('src/app.py::L1', 'src/utils.py::parse', 'Calls', 0.5, 'ast_direct')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES ('src/main.py::L5', 'src/db.py::connect', 'Calls', 0.5, 'ast_direct')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, edge_source) \
                 VALUES ('src/api.py::L10', 'src/auth.py::verify', 'Imports', 0.8, 'framework_adapter')",
                [],
            ).unwrap();
            drop(conn);

            // Write the arbitrary bytes to a temp file as a "SCIP index".
            let tmp_dir = TempDir::new().expect("failed to create temp dir");
            let index_path = tmp_dir.path().join("index.scip");
            fs::write(&index_path, &data).expect("failed to write test file");

            // Call ingest_scip_index — it should return an error (not panic).
            let result = ingest_scip_index(&index_path, &store);

            // The result is either Ok (if the random bytes happen to be valid
            // protobuf — extremely unlikely but possible) or Err. Either way,
            // it must NOT panic.
            if result.is_err() {
                // Verify it's a ProtobufParseFailed error (not a panic).
                match result.unwrap_err() {
                    ScipError::ProtobufParseFailed { .. } => { /* expected */ }
                    ScipError::FileReadFailed { .. } => { /* also acceptable */ }
                    ScipError::DatabaseError { .. } => { /* acceptable — no panic */ }
                }
            }

            // Regardless of success or failure, all pre-existing edges must
            // remain intact in the database.
            let conn = store.read_conn();
            let edge_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM edges WHERE edge_source IN ('ast_direct', 'framework_adapter')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            prop_assert!(
                edge_count >= 3,
                "Pre-existing edges were lost! Expected >= 3, got {}",
                edge_count
            );

            // Also verify the safe wrapper (try_ingest_scip) doesn't panic.
            // Re-write the file in case ingest_scip_index consumed it.
            fs::write(&index_path, &data).expect("failed to re-write test file");
            let safe_result = try_ingest_scip(&index_path, &store);
            // safe_result is either Some or None — never panics.
            if safe_result.is_none() {
                // Edges must still be intact after the safe wrapper too.
                let count_after: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM edges WHERE edge_source IN ('ast_direct', 'framework_adapter')",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                prop_assert!(
                    count_after >= 3,
                    "Pre-existing edges lost after try_ingest_scip! Expected >= 3, got {}",
                    count_after
                );
            }
        }
    }
}
