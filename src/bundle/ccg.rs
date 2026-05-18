//! CCG (Code Context Graph) export format.
//!
//! Maps Cortex internal types to the CCG JSON schema for interoperability
//! with other tools that consume the CCG format.

use serde::{Deserialize, Serialize};

use crate::error::BundleError;
use crate::store::db::StoreManager;

/// CCG export statistics.
#[derive(Debug, Clone, Serialize)]
pub struct CcgExportStats {
    pub nodes_exported: usize,
    pub edges_exported: usize,
    pub output_path: String,
}

/// CCG Node representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcgNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub label: String,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub metadata: serde_json::Value,
}

/// CCG Edge representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcgEdge {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    pub weight: f64,
}

/// CCG Document (top-level JSON structure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcgDocument {
    pub version: String,
    pub generator: String,
    pub nodes: Vec<CcgNode>,
    pub edges: Vec<CcgEdge>,
}

/// Export the graph in CCG format.
///
/// Maps Cortex nodes and edges to the CCG JSON schema.
pub fn export_ccg(store: &StoreManager, output_dir: &std::path::Path) -> Result<CcgExportStats, BundleError> {
    let conn = store.read_conn();

    // Query all nodes
    let mut node_stmt = conn
        .prepare("SELECT fqn, kind, file, start_line, end_line, attributes FROM nodes")
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to query nodes: {}", e) })?;

    let ccg_nodes: Vec<CcgNode> = node_stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let file: String = row.get(2)?;
            let start_line: u32 = row.get(3)?;
            let end_line: u32 = row.get(4)?;
            let attrs: String = row.get(5)?;

            Ok(CcgNode {
                id: fqn.clone(),
                node_type: map_node_kind_to_ccg(&kind),
                label: fqn.split("::").last().unwrap_or(&fqn).to_string(),
                file,
                line_start: start_line,
                line_end: end_line,
                metadata: serde_json::from_str(&attrs).unwrap_or(serde_json::json!({})),
            })
        })
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to read nodes: {}", e) })?
        .filter_map(|r| r.ok())
        .collect();

    // Query all edges
    let mut edge_stmt = conn
        .prepare("SELECT source_fqn, target_fqn, kind, confidence FROM edges")
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to query edges: {}", e) })?;

    let ccg_edges: Vec<CcgEdge> = edge_stmt
        .query_map([], |row| {
            let source: String = row.get(0)?;
            let target: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let confidence: f64 = row.get(3)?;

            Ok(CcgEdge {
                source,
                target,
                edge_type: map_edge_kind_to_ccg(&kind),
                weight: confidence,
            })
        })
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to read edges: {}", e) })?
        .filter_map(|r| r.ok())
        .collect();

    let doc = CcgDocument {
        version: "1.0".to_string(),
        generator: format!("cortex {}", crate::version::VERSION),
        nodes: ccg_nodes.clone(),
        edges: ccg_edges.clone(),
    };

    // Write to file
    let output_path = output_dir.join("cortex.ccg.json");
    let json = serde_json::to_string_pretty(&doc)
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to serialize CCG: {}", e) })?;

    std::fs::write(&output_path, &json)
        .map_err(|e| BundleError::ExportFailed { reason: format!("failed to write CCG file: {}", e) })?;

    Ok(CcgExportStats {
        nodes_exported: ccg_nodes.len(),
        edges_exported: ccg_edges.len(),
        output_path: output_path.display().to_string(),
    })
}

/// Map Cortex node kind to CCG type string.
fn map_node_kind_to_ccg(kind: &str) -> String {
    match kind {
        "Function" => "function".to_string(),
        "Class" => "class".to_string(),
        "Module" => "module".to_string(),
        "Route" => "endpoint".to_string(),
        "Interface" => "interface".to_string(),
        "Type" => "type".to_string(),
        other => other.to_lowercase(),
    }
}

/// Map Cortex edge kind to CCG type string.
fn map_edge_kind_to_ccg(kind: &str) -> String {
    match kind {
        "Calls" => "calls".to_string(),
        "Imports" => "imports".to_string(),
        "Inherits" => "extends".to_string(),
        "Implements" => "implements".to_string(),
        "HttpLink" => "http_link".to_string(),
        "DataFlow" => "data_flow".to_string(),
        other => other.to_lowercase(),
    }
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

    fn setup_store() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();
        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).unwrap();
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_ccg_export_produces_valid_json_structure() {
        let (store, _tmp) = setup_store();
        let output_dir = TempDir::new().unwrap();

        // Insert test data
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/lib.rs::helper', 'Function', 'src/lib.rs', 5, 20, 'hash2', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('src/main.rs::main', 'src/lib.rs::helper', 'Calls', 1.0, '{}')",
                [],
            ).unwrap();
        }

        let stats = export_ccg(&store, output_dir.path()).unwrap();
        assert_eq!(stats.nodes_exported, 2);
        assert_eq!(stats.edges_exported, 1);

        // Read and validate the output file
        let content = std::fs::read_to_string(output_dir.path().join("cortex.ccg.json")).unwrap();
        let doc: CcgDocument = serde_json::from_str(&content).unwrap();

        assert_eq!(doc.version, "1.0");
        assert!(doc.generator.contains("cortex"));
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges.len(), 1);

        // Verify node structure
        let main_node = doc.nodes.iter().find(|n| n.id == "src/main.rs::main").unwrap();
        assert_eq!(main_node.node_type, "function");
        assert_eq!(main_node.label, "main");
        assert_eq!(main_node.file, "src/main.rs");

        // Verify edge structure
        assert_eq!(doc.edges[0].source, "src/main.rs::main");
        assert_eq!(doc.edges[0].target, "src/lib.rs::helper");
        assert_eq!(doc.edges[0].edge_type, "calls");
        assert_eq!(doc.edges[0].weight, 1.0);
    }

    #[test]
    fn test_ccg_export_empty_graph() {
        let (store, _tmp) = setup_store();
        let output_dir = TempDir::new().unwrap();

        let stats = export_ccg(&store, output_dir.path()).unwrap();
        assert_eq!(stats.nodes_exported, 0);
        assert_eq!(stats.edges_exported, 0);

        let content = std::fs::read_to_string(output_dir.path().join("cortex.ccg.json")).unwrap();
        let doc: CcgDocument = serde_json::from_str(&content).unwrap();
        assert_eq!(doc.nodes.len(), 0);
        assert_eq!(doc.edges.len(), 0);
    }
}
