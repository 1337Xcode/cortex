//! Bundle import: deserialize a portable JSON bundle into the graph store.
//!
//! Verifies the SHA-256 checksum, validates the format version, clears
//! existing data, and populates the store from the bundle.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::BundleError;
use crate::store::db::StoreManager;

use super::format::{CortexBundle, validate_version};

// ---------------------------------------------------------------------------
// Import stats
// ---------------------------------------------------------------------------

/// Statistics returned after a successful bundle import.
#[derive(Debug, Clone)]
pub struct ImportStats {
    pub nodes_imported: usize,
    pub edges_imported: usize,
    pub findings_imported: usize,
    pub observations_imported: usize,
}

// ---------------------------------------------------------------------------
// Import implementation
// ---------------------------------------------------------------------------

/// Imports a bundle from a JSON file into the graph store.
///
/// Steps:
/// 1. Read the bundle file and its `.sha256` checksum file
/// 2. Verify the SHA-256 checksum matches
/// 3. Deserialize the JSON into a CortexBundle
/// 4. Validate the format version
/// 5. Clear all existing data tables
/// 6. Insert all data from the bundle
pub fn import_bundle(store: &StoreManager, bundle_path: &Path) -> Result<ImportStats, BundleError> {
    // Step 1: Read files
    let json_str = fs::read_to_string(bundle_path).map_err(|e| BundleError::ImportFailed {
        reason: format!(
            "failed to read bundle file '{}': {e}",
            bundle_path.display()
        ),
    })?;

    let checksum_path = bundle_path.with_extension("json.sha256");
    let expected_checksum =
        fs::read_to_string(&checksum_path).map_err(|e| BundleError::ImportFailed {
            reason: format!(
                "failed to read checksum file '{}': {e}",
                checksum_path.display()
            ),
        })?;

    // Step 2: Verify checksum
    let mut hasher = Sha256::new();
    hasher.update(json_str.as_bytes());
    let actual_checksum = format!("{:x}", hasher.finalize());

    if actual_checksum != expected_checksum.trim() {
        return Err(BundleError::ChecksumMismatch {
            expected: expected_checksum.trim().to_string(),
            actual: actual_checksum,
        });
    }

    // Step 3: Deserialize
    let bundle: CortexBundle =
        serde_json::from_str(&json_str).map_err(|e| BundleError::ImportFailed {
            reason: format!("failed to parse bundle JSON: {e}"),
        })?;

    // Step 4: Validate version
    validate_version(bundle.schema.format_version)?;

    // Step 5 & 6: Clear and populate in a single transaction
    let conn = store.write_conn();
    conn.execute_batch("BEGIN TRANSACTION;")
        .map_err(|e| BundleError::ImportFailed {
            reason: format!("failed to begin transaction: {e}"),
        })?;

    // Clear tables (order matters for foreign keys)
    let clear_result = (|| -> Result<(), BundleError> {
        conn.execute_batch(
            "DELETE FROM sbom_entries;
             DELETE FROM taint_paths;
             DELETE FROM security_findings;
             DELETE FROM observations;
             DELETE FROM architectural_decisions;
             DELETE FROM edges;
             DELETE FROM nodes;",
        )
        .map_err(|e| BundleError::ImportFailed {
            reason: format!("failed to clear tables: {e}"),
        })?;
        Ok(())
    })();

    if let Err(e) = clear_result {
        let _ = conn.execute_batch("ROLLBACK;");
        return Err(e);
    }

    // Insert nodes
    let insert_result = (|| -> Result<ImportStats, BundleError> {
        for node in &bundle.nodes {
            let kind_str =
                serde_json::to_string(&node.kind).unwrap_or_else(|_| "\"Function\"".to_string());
            let kind_str = kind_str.trim_matches('"');
            let attrs_str =
                serde_json::to_string(&node.attributes).unwrap_or_else(|_| "{}".to_string());

            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![node.fqn, kind_str, node.file, node.start_line, node.end_line, node.file_hash, node.indexed_at, attrs_str],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert node '{}': {e}", node.fqn),
            })?;
        }

        // Insert edges
        for edge in &bundle.edges {
            let kind_str =
                serde_json::to_string(&edge.kind).unwrap_or_else(|_| "\"Calls\"".to_string());
            let kind_str = kind_str.trim_matches('"');
            let attrs_str =
                serde_json::to_string(&edge.attributes).unwrap_or_else(|_| "{}".to_string());

            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![edge.source_fqn, edge.target_fqn, kind_str, edge.confidence, attrs_str],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert edge '{}' -> '{}': {e}", edge.source_fqn, edge.target_fqn),
            })?;
        }

        // Insert security findings
        for finding in &bundle.security_findings {
            conn.execute(
                "INSERT INTO security_findings (node_fqn, kind, owasp_category, cwe_id, confidence, description, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![finding.node_fqn, finding.kind, finding.owasp_category, finding.cwe_id, finding.confidence, finding.description, finding.indexed_at],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert security finding: {e}"),
            })?;
        }

        // Insert taint paths
        for tp in &bundle.taint_paths {
            conn.execute(
                "INSERT INTO taint_paths (source_fqn, source_kind, sink_fqn, sink_kind, path_json, confidence, cwe_id, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![tp.source_fqn, tp.source_kind, tp.sink_fqn, tp.sink_kind, tp.path_json, tp.confidence, tp.cwe_id, tp.indexed_at],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert taint path: {e}"),
            })?;
        }

        // Insert observations
        for obs in &bundle.observations {
            conn.execute(
                "INSERT INTO observations (id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![obs.id, obs.node_fqn, obs.observation_text, obs.agent_id, obs.node_hash_at_write, obs.written_at, obs.status, obs.stale_reason],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert observation: {e}"),
            })?;
        }

        // Insert ADRs
        for adr in &bundle.adrs {
            conn.execute(
                "INSERT INTO architectural_decisions (id, title, body, status, linked_fqn, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![adr.id, adr.title, adr.body, adr.status, adr.linked_fqn, adr.created_at, adr.updated_at],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert ADR: {e}"),
            })?;
        }

        // Insert SBOM entries
        for entry in &bundle.sbom_entries {
            conn.execute(
                "INSERT INTO sbom_entries (name, version, license, source_file, import_fqn, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![entry.name, entry.version, entry.license, entry.source_file, entry.import_fqn, entry.indexed_at],
            ).map_err(|e| BundleError::ImportFailed {
                reason: format!("failed to insert SBOM entry: {e}"),
            })?;
        }

        Ok(ImportStats {
            nodes_imported: bundle.nodes.len(),
            edges_imported: bundle.edges.len(),
            findings_imported: bundle.security_findings.len(),
            observations_imported: bundle.observations.len(),
        })
    })();

    match insert_result {
        Ok(stats) => {
            conn.execute_batch("COMMIT;")
                .map_err(|e| BundleError::ImportFailed {
                    reason: format!("failed to commit transaction: {e}"),
                })?;
            Ok(stats)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::export::export_bundle;
    use crate::store::db::StoreManager;
    use crate::store::migrations::run_migrations;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_store_with_migrations() -> (StoreManager, TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let output_dir = tempfile::tempdir().unwrap();
        let store = StoreManager::new(data_dir.path()).unwrap();

        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).unwrap();
        drop(conn);

        (store, data_dir, output_dir)
    }

    #[test]
    fn roundtrip_export_import() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        // Insert test data
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params!["src/main.rs::main", "Function", "src/main.rs", 1, 10, "hash123", 1715270300, "{}"],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params!["src/main.rs::main", "src/lib.rs::run", "Calls", 1.0, "{}"],
            ).unwrap();
        }

        // Export
        let export_stats = export_bundle(&store, output_dir.path()).unwrap();
        assert_eq!(export_stats.node_count, 1);
        assert_eq!(export_stats.edge_count, 1);

        // Clear the store manually to simulate fresh import
        {
            let conn = store.write_conn();
            conn.execute_batch("DELETE FROM edges; DELETE FROM nodes;")
                .unwrap();
        }

        // Import
        let bundle_path = output_dir.path().join("cortex.json");
        let import_stats = import_bundle(&store, &bundle_path).unwrap();
        assert_eq!(import_stats.nodes_imported, 1);
        assert_eq!(import_stats.edges_imported, 1);

        // Verify data is back
        let conn = store.read_conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn tampered_checksum_fails() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        export_bundle(&store, output_dir.path()).unwrap();

        // Tamper with the checksum file
        let checksum_path = output_dir.path().join("cortex.json.sha256");
        fs::write(&checksum_path, "tampered_checksum_value").unwrap();

        let bundle_path = output_dir.path().join("cortex.json");
        let result = import_bundle(&store, &bundle_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn incompatible_version_fails() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        export_bundle(&store, output_dir.path()).unwrap();

        // Modify the bundle to have an incompatible version
        let bundle_path = output_dir.path().join("cortex.json");
        let json_str = fs::read_to_string(&bundle_path).unwrap();
        let mut bundle: CortexBundle = serde_json::from_str(&json_str).unwrap();
        bundle.schema.format_version = 999;
        let modified_json = serde_json::to_string_pretty(&bundle).unwrap();
        fs::write(&bundle_path, &modified_json).unwrap();

        // Update checksum to match modified content
        let mut hasher = Sha256::new();
        hasher.update(modified_json.as_bytes());
        let checksum = format!("{:x}", hasher.finalize());
        fs::write(output_dir.path().join("cortex.json.sha256"), &checksum).unwrap();

        let result = import_bundle(&store, &bundle_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("incompatible"));
    }
}
