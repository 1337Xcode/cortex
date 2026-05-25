//! Bundle export: serialize the graph store to a portable JSON file.
//!
//! Writes `.cortex/cortex.json`, `.cortex/cortex.json.sha256`, and
//! `.cortex/.gitignore` to the output directory.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::BundleError;
use crate::store::db::StoreManager;
use crate::store::types::{Adr, Edge, Node, Observation, SbomEntry, SecurityFinding, TaintPath};
use crate::version::VERSION;

use super::format::{BundleSchema, CURRENT_FORMAT_VERSION, CortexBundle};

// ---------------------------------------------------------------------------
// Export stats
// ---------------------------------------------------------------------------

/// Statistics returned after a successful bundle export.
#[derive(Debug, Clone)]
pub struct ExportStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub finding_count: usize,
    pub observation_count: usize,
    pub file_size_bytes: u64,
}

// ---------------------------------------------------------------------------
// Export implementation
// ---------------------------------------------------------------------------

/// Exports the entire graph store to a portable JSON bundle.
///
/// Steps:
/// 1. Run PRAGMA integrity_check on the database
/// 2. Query all tables (nodes, edges, security_findings, taint_paths,
///    observations, adrs, sbom_entries)
/// 3. Serialize to JSON
/// 4. Write `.cortex/cortex.json`
/// 5. Compute and write SHA-256 checksum to `.cortex/cortex.json.sha256`
/// 6. Write `.cortex/.gitignore` (ignoring graph.db files but not the bundle)
pub fn export_bundle(store: &StoreManager, output_dir: &Path) -> Result<ExportStats, BundleError> {
    // Step 1: Integrity check
    {
        let conn = store.read_conn();
        let result: String = conn
            .query_row("PRAGMA integrity_check;", [], |row| row.get(0))
            .map_err(|e| BundleError::ExportFailed {
                reason: format!("integrity check failed: {e}"),
            })?;
        if result != "ok" {
            return Err(BundleError::ExportFailed {
                reason: format!("database integrity check failed: {result}"),
            });
        }
    }

    // Step 2: Query all tables
    let conn = store.read_conn();

    let nodes = query_all_nodes(&conn)?;
    let edges = query_all_edges(&conn)?;
    let security_findings = query_all_findings(&conn)?;
    let taint_paths = query_all_taint_paths(&conn)?;
    let observations = query_all_observations(&conn)?;
    let adrs = query_all_adrs(&conn)?;
    let sbom_entries = query_all_sbom_entries(&conn)?;

    // Build bundle
    let bundle = CortexBundle {
        schema: BundleSchema {
            format_version: CURRENT_FORMAT_VERSION,
            cortex_version: VERSION.to_string(),
            repo_root_hash: "".to_string(),
            exported_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        },
        nodes: nodes.clone(),
        edges: edges.clone(),
        security_findings: security_findings.clone(),
        taint_paths: taint_paths.clone(),
        observations: observations.clone(),
        adrs: adrs.clone(),
        sbom_entries: sbom_entries.clone(),
    };

    // Step 3: Serialize
    let json_str =
        serde_json::to_string_pretty(&bundle).map_err(|e| BundleError::ExportFailed {
            reason: format!("JSON serialization failed: {e}"),
        })?;

    // Step 4: Write cortex.json
    fs::create_dir_all(output_dir).map_err(|e| BundleError::ExportFailed {
        reason: format!("failed to create output directory: {e}"),
    })?;

    let bundle_path = output_dir.join("cortex.json");
    fs::write(&bundle_path, &json_str).map_err(|e| BundleError::ExportFailed {
        reason: format!("failed to write bundle file: {e}"),
    })?;

    let file_size_bytes = json_str.len() as u64;

    // Step 5: Write SHA-256 checksum
    let mut hasher = Sha256::new();
    hasher.update(json_str.as_bytes());
    let checksum = format!("{:x}", hasher.finalize());

    let checksum_path = output_dir.join("cortex.json.sha256");
    fs::write(&checksum_path, &checksum).map_err(|e| BundleError::ExportFailed {
        reason: format!("failed to write checksum file: {e}"),
    })?;

    // Step 6: Write .gitignore
    let gitignore_path = output_dir.join(".gitignore");
    let gitignore_content =
        "# Cortex data files (not portable)\ngraph.db\ngraph.db-wal\ngraph.db-shm\n";
    fs::write(&gitignore_path, gitignore_content).map_err(|e| BundleError::ExportFailed {
        reason: format!("failed to write .gitignore: {e}"),
    })?;

    Ok(ExportStats {
        node_count: nodes.len(),
        edge_count: edges.len(),
        finding_count: security_findings.len(),
        observation_count: observations.len(),
        file_size_bytes,
    })
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

fn query_all_nodes(conn: &rusqlite::Connection) -> Result<Vec<Node>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes FROM nodes")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare nodes query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let kind_str: String = row.get(1)?;
            let attrs_str: String = row.get(7)?;
            Ok(Node {
                fqn: row.get(0)?,
                kind: serde_json::from_str(&format!("\"{kind_str}\""))
                    .unwrap_or(crate::store::types::NodeKind::Function),
                file: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                file_hash: row.get(5)?,
                indexed_at: row.get(6)?,
                attributes: serde_json::from_str(&attrs_str)
                    .unwrap_or(serde_json::Value::Object(Default::default())),
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query nodes: {e}"),
        })?;

    let mut nodes = Vec::new();
    for row in rows {
        nodes.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read node row: {e}"),
        })?);
    }
    Ok(nodes)
}

fn query_all_edges(conn: &rusqlite::Connection) -> Result<Vec<Edge>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, source_fqn, target_fqn, kind, confidence, attributes FROM edges")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare edges query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let kind_str: String = row.get(3)?;
            let attrs_str: String = row.get(5)?;
            Ok(Edge {
                id: row.get(0)?,
                source_fqn: row.get(1)?,
                target_fqn: row.get(2)?,
                kind: serde_json::from_str(&format!("\"{kind_str}\""))
                    .unwrap_or(crate::store::types::EdgeKind::Calls),
                confidence: row.get(4)?,
                attributes: serde_json::from_str(&attrs_str)
                    .unwrap_or(serde_json::Value::Object(Default::default())),
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query edges: {e}"),
        })?;

    let mut edges = Vec::new();
    for row in rows {
        edges.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read edge row: {e}"),
        })?);
    }
    Ok(edges)
}

fn query_all_findings(conn: &rusqlite::Connection) -> Result<Vec<SecurityFinding>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, node_fqn, kind, owasp_category, cwe_id, confidence, description, indexed_at FROM security_findings")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare findings query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SecurityFinding {
                id: row.get(0)?,
                node_fqn: row.get(1)?,
                kind: row.get(2)?,
                owasp_category: row.get(3)?,
                cwe_id: row.get(4)?,
                confidence: row.get(5)?,
                description: row.get(6)?,
                indexed_at: row.get(7)?,
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query findings: {e}"),
        })?;

    let mut findings = Vec::new();
    for row in rows {
        findings.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read finding row: {e}"),
        })?);
    }
    Ok(findings)
}

fn query_all_taint_paths(conn: &rusqlite::Connection) -> Result<Vec<TaintPath>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, source_fqn, source_kind, sink_fqn, sink_kind, path_json, confidence, cwe_id, indexed_at FROM taint_paths")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare taint_paths query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(TaintPath {
                id: row.get(0)?,
                source_fqn: row.get(1)?,
                source_kind: row.get(2)?,
                sink_fqn: row.get(3)?,
                sink_kind: row.get(4)?,
                path_json: row.get(5)?,
                confidence: row.get(6)?,
                cwe_id: row.get(7)?,
                indexed_at: row.get(8)?,
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query taint_paths: {e}"),
        })?;

    let mut paths = Vec::new();
    for row in rows {
        paths.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read taint_path row: {e}"),
        })?);
    }
    Ok(paths)
}

fn query_all_observations(conn: &rusqlite::Connection) -> Result<Vec<Observation>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason FROM observations")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare observations query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Observation {
                id: row.get(0)?,
                node_fqn: row.get(1)?,
                observation_text: row.get(2)?,
                agent_id: row.get(3)?,
                node_hash_at_write: row.get(4)?,
                written_at: row.get(5)?,
                status: row.get(6)?,
                stale_reason: row.get(7)?,
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query observations: {e}"),
        })?;

    let mut observations = Vec::new();
    for row in rows {
        observations.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read observation row: {e}"),
        })?);
    }
    Ok(observations)
}

fn query_all_adrs(conn: &rusqlite::Connection) -> Result<Vec<Adr>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, title, body, status, linked_fqn, created_at, updated_at FROM architectural_decisions")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare adrs query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Adr {
                id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
                status: row.get(3)?,
                linked_fqn: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query adrs: {e}"),
        })?;

    let mut adrs = Vec::new();
    for row in rows {
        adrs.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read adr row: {e}"),
        })?);
    }
    Ok(adrs)
}

fn query_all_sbom_entries(conn: &rusqlite::Connection) -> Result<Vec<SbomEntry>, BundleError> {
    let mut stmt = conn
        .prepare("SELECT id, name, version, license, source_file, import_fqn, indexed_at FROM sbom_entries")
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to prepare sbom_entries query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SbomEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                license: row.get(3)?,
                source_file: row.get(4)?,
                import_fqn: row.get(5)?,
                indexed_at: row.get(6)?,
            })
        })
        .map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to query sbom_entries: {e}"),
        })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| BundleError::ExportFailed {
            reason: format!("failed to read sbom_entry row: {e}"),
        })?);
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations::run_migrations;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_store_with_migrations() -> (StoreManager, TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let output_dir = tempfile::tempdir().unwrap();
        let store = StoreManager::new(data_dir.path()).unwrap();

        // Find migrations directory
        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).unwrap();
        drop(conn);

        (store, data_dir, output_dir)
    }

    #[test]
    fn export_produces_valid_json() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        // Insert a test node
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params!["src/main.rs::main", "Function", "src/main.rs", 1, 10, "hash123", 1715270300, "{}"],
            ).unwrap();
        }

        let stats = export_bundle(&store, output_dir.path()).unwrap();
        assert_eq!(stats.node_count, 1);
        assert_eq!(stats.edge_count, 0);

        // Verify the JSON is valid
        let json_str = fs::read_to_string(output_dir.path().join("cortex.json")).unwrap();
        let bundle: CortexBundle = serde_json::from_str(&json_str).unwrap();
        assert_eq!(bundle.nodes.len(), 1);
        assert_eq!(bundle.schema.format_version, CURRENT_FORMAT_VERSION);
    }

    #[test]
    fn export_checksum_matches_content() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        export_bundle(&store, output_dir.path()).unwrap();

        let json_str = fs::read_to_string(output_dir.path().join("cortex.json")).unwrap();
        let stored_checksum =
            fs::read_to_string(output_dir.path().join("cortex.json.sha256")).unwrap();

        // Compute expected checksum
        let mut hasher = Sha256::new();
        hasher.update(json_str.as_bytes());
        let expected = format!("{:x}", hasher.finalize());

        assert_eq!(stored_checksum, expected);
    }

    #[test]
    fn export_gitignore_correct() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        export_bundle(&store, output_dir.path()).unwrap();

        let gitignore = fs::read_to_string(output_dir.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains("graph.db"));
        assert!(gitignore.contains("graph.db-wal"));
        assert!(gitignore.contains("graph.db-shm"));
    }

    #[test]
    fn export_empty_db() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        let stats = export_bundle(&store, output_dir.path()).unwrap();
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.finding_count, 0);
        assert_eq!(stats.observation_count, 0);

        // Should still produce valid JSON
        let json_str = fs::read_to_string(output_dir.path().join("cortex.json")).unwrap();
        let bundle: CortexBundle = serde_json::from_str(&json_str).unwrap();
        assert!(bundle.nodes.is_empty());
    }
}
