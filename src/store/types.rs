//! Store types matching the database schema exactly.
//!
//! All types derive Debug, Clone, serde::Serialize, serde::Deserialize.
//! Enum variants serialize to/from their string representation matching
//! the database CHECK constraints.

use serde::{Deserialize, Serialize};

/// Node kind matching the CHECK constraint:
/// `kind IN ('Function','Class','Module','Route','Interface','Type','Enum','Constant','TypeAlias','Trait','Namespace')`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Function,
    Class,
    Module,
    Route,
    Interface,
    Type,
    Enum,
    Constant,
    TypeAlias,
    Trait,
    Namespace,
}

/// Edge kind matching the CHECK constraint:
/// `kind IN ('Calls','Imports','Inherits','Implements','HttpLink','DataFlow')`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    Calls,
    Imports,
    Inherits,
    Implements,
    HttpLink,
    DataFlow,
}

/// A node in the code graph, corresponding to the `nodes` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub fqn: String,
    pub kind: NodeKind,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub file_hash: String,
    pub indexed_at: i64,
    pub attributes: serde_json::Value,
}

/// An edge in the code graph, corresponding to the `edges` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: Option<i64>,
    pub source_fqn: String,
    pub target_fqn: String,
    pub kind: EdgeKind,
    pub confidence: f64,
    pub attributes: serde_json::Value,
}

/// A file snapshot record, corresponding to the `file_snapshots` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub file: String,
    pub file_hash: String,
    pub node_count: u32,
    pub indexed_at: i64,
}

/// A security finding, corresponding to the `security_findings` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub id: Option<i64>,
    pub node_fqn: String,
    pub kind: String,
    pub owasp_category: Option<String>,
    pub cwe_id: Option<String>,
    pub confidence: f64,
    pub description: String,
    pub indexed_at: i64,
}

/// A taint path, corresponding to the `taint_paths` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintPath {
    pub id: Option<i64>,
    pub source_fqn: String,
    pub source_kind: String,
    pub sink_fqn: String,
    pub sink_kind: String,
    pub path_json: String,
    pub confidence: f64,
    pub cwe_id: Option<String>,
    pub indexed_at: i64,
}

/// An SBOM entry, corresponding to the `sbom_entries` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomEntry {
    pub id: Option<i64>,
    pub name: String,
    pub version: Option<String>,
    pub license: Option<String>,
    pub source_file: String,
    pub import_fqn: String,
    pub indexed_at: i64,
}

/// An observation, corresponding to the `observations` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub node_fqn: String,
    pub observation_text: String,
    pub agent_id: String,
    pub node_hash_at_write: String,
    pub written_at: i64,
    pub status: String,
    pub stale_reason: Option<String>,
}

/// An architectural decision record, corresponding to the `architectural_decisions` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Adr {
    pub id: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub linked_fqn: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A change note, corresponding to the `change_notes` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeNote {
    pub id: String,
    pub text: String,
    pub created_at: i64,
}

/// Bundle metadata, corresponding to the `bundle_metadata` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMetadata {
    pub id: i64,
    pub format_version: i64,
    pub last_export_at: Option<i64>,
    pub export_checksum: Option<String>,
    pub repo_root_hash: Option<String>,
}

/// Result of extracting nodes and edges from a parsed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_kind_serialize_deserialize_roundtrip() {
        let variants = vec![
            NodeKind::Function,
            NodeKind::Class,
            NodeKind::Module,
            NodeKind::Route,
            NodeKind::Interface,
            NodeKind::Type,
            NodeKind::Enum,
            NodeKind::Constant,
            NodeKind::TypeAlias,
            NodeKind::Trait,
            NodeKind::Namespace,
        ];

        for variant in &variants {
            let serialized = serde_json::to_string(variant).unwrap();
            let deserialized: NodeKind = serde_json::from_str(&serialized).unwrap();
            assert_eq!(variant, &deserialized);
        }
    }

    #[test]
    fn node_kind_variants_match_schema() {
        // These must match: CHECK(kind IN ('Function','Class','Module','Route','Interface','Type','Enum','Constant','TypeAlias','Trait','Namespace'))
        let expected = vec!["Function", "Class", "Module", "Route", "Interface", "Type", "Enum", "Constant", "TypeAlias", "Trait", "Namespace"];
        for name in &expected {
            let json_str = format!("\"{}\"", name);
            let kind: NodeKind = serde_json::from_str(&json_str).unwrap();
            let reserialized = serde_json::to_string(&kind).unwrap();
            assert_eq!(reserialized, json_str);
        }
    }

    #[test]
    fn edge_kind_serialize_deserialize_roundtrip() {
        let variants = vec![
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::Inherits,
            EdgeKind::Implements,
            EdgeKind::HttpLink,
            EdgeKind::DataFlow,
        ];

        for variant in &variants {
            let serialized = serde_json::to_string(variant).unwrap();
            let deserialized: EdgeKind = serde_json::from_str(&serialized).unwrap();
            assert_eq!(variant, &deserialized);
        }
    }

    #[test]
    fn edge_kind_variants_match_schema() {
        // These must match: CHECK(kind IN ('Calls','Imports','Inherits','Implements','HttpLink','DataFlow'))
        let expected = vec!["Calls", "Imports", "Inherits", "Implements", "HttpLink", "DataFlow"];
        for name in &expected {
            let json_str = format!("\"{}\"", name);
            let kind: EdgeKind = serde_json::from_str(&json_str).unwrap();
            let reserialized = serde_json::to_string(&kind).unwrap();
            assert_eq!(reserialized, json_str);
        }
    }

    #[test]
    fn node_serialize_deserialize_roundtrip() {
        let node = Node {
            fqn: "src/main.rs::main".to_string(),
            kind: NodeKind::Function,
            file: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 10,
            file_hash: "abc123".to_string(),
            indexed_at: 1715270300,
            attributes: json!({"async": true}),
        };

        let serialized = serde_json::to_string(&node).unwrap();
        let deserialized: Node = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.fqn, node.fqn);
        assert_eq!(deserialized.kind, node.kind);
        assert_eq!(deserialized.file, node.file);
        assert_eq!(deserialized.start_line, node.start_line);
        assert_eq!(deserialized.end_line, node.end_line);
        assert_eq!(deserialized.file_hash, node.file_hash);
        assert_eq!(deserialized.indexed_at, node.indexed_at);
        assert_eq!(deserialized.attributes, node.attributes);
    }

    #[test]
    fn edge_serialize_deserialize_roundtrip() {
        let edge = Edge {
            id: Some(1),
            source_fqn: "src/main.rs::main".to_string(),
            target_fqn: "src/lib.rs::run".to_string(),
            kind: EdgeKind::Calls,
            confidence: 0.95,
            attributes: json!({}),
        };

        let serialized = serde_json::to_string(&edge).unwrap();
        let deserialized: Edge = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, edge.id);
        assert_eq!(deserialized.source_fqn, edge.source_fqn);
        assert_eq!(deserialized.target_fqn, edge.target_fqn);
        assert_eq!(deserialized.kind, edge.kind);
        assert_eq!(deserialized.confidence, edge.confidence);
        assert_eq!(deserialized.attributes, edge.attributes);
    }

    #[test]
    fn file_snapshot_serialize_deserialize_roundtrip() {
        let snapshot = FileSnapshot {
            file: "src/main.rs".to_string(),
            file_hash: "sha256:abc".to_string(),
            node_count: 5,
            indexed_at: 1715270300,
        };

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let deserialized: FileSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.file, snapshot.file);
        assert_eq!(deserialized.file_hash, snapshot.file_hash);
        assert_eq!(deserialized.node_count, snapshot.node_count);
        assert_eq!(deserialized.indexed_at, snapshot.indexed_at);
    }

    #[test]
    fn security_finding_serialize_deserialize_roundtrip() {
        let finding = SecurityFinding {
            id: Some(1),
            node_fqn: "src/auth.rs::login".to_string(),
            kind: "sql_injection".to_string(),
            owasp_category: Some("A03".to_string()),
            cwe_id: Some("CWE-89".to_string()),
            confidence: 0.9,
            description: "Unsanitized input in SQL query".to_string(),
            indexed_at: 1715270300,
        };

        let serialized = serde_json::to_string(&finding).unwrap();
        let deserialized: SecurityFinding = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, finding.id);
        assert_eq!(deserialized.node_fqn, finding.node_fqn);
        assert_eq!(deserialized.kind, finding.kind);
        assert_eq!(deserialized.owasp_category, finding.owasp_category);
        assert_eq!(deserialized.cwe_id, finding.cwe_id);
        assert_eq!(deserialized.confidence, finding.confidence);
        assert_eq!(deserialized.description, finding.description);
    }

    #[test]
    fn taint_path_serialize_deserialize_roundtrip() {
        let path = TaintPath {
            id: Some(1),
            source_fqn: "src/routes.rs::get_input".to_string(),
            source_kind: "user_input".to_string(),
            sink_fqn: "src/db.rs::execute_query".to_string(),
            sink_kind: "sql_query".to_string(),
            path_json: r#"["get_input","process","execute_query"]"#.to_string(),
            confidence: 0.85,
            cwe_id: Some("CWE-89".to_string()),
            indexed_at: 1715270300,
        };

        let serialized = serde_json::to_string(&path).unwrap();
        let deserialized: TaintPath = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, path.id);
        assert_eq!(deserialized.source_fqn, path.source_fqn);
        assert_eq!(deserialized.source_kind, path.source_kind);
        assert_eq!(deserialized.sink_fqn, path.sink_fqn);
        assert_eq!(deserialized.sink_kind, path.sink_kind);
        assert_eq!(deserialized.path_json, path.path_json);
        assert_eq!(deserialized.confidence, path.confidence);
        assert_eq!(deserialized.cwe_id, path.cwe_id);
    }

    #[test]
    fn sbom_entry_serialize_deserialize_roundtrip() {
        let entry = SbomEntry {
            id: Some(1),
            name: "serde".to_string(),
            version: Some("1.0.0".to_string()),
            license: Some("MIT OR Apache-2.0".to_string()),
            source_file: "Cargo.toml".to_string(),
            import_fqn: "src/main.rs::serde".to_string(),
            indexed_at: 1715270300,
        };

        let serialized = serde_json::to_string(&entry).unwrap();
        let deserialized: SbomEntry = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, entry.id);
        assert_eq!(deserialized.name, entry.name);
        assert_eq!(deserialized.version, entry.version);
        assert_eq!(deserialized.license, entry.license);
        assert_eq!(deserialized.source_file, entry.source_file);
        assert_eq!(deserialized.import_fqn, entry.import_fqn);
    }

    #[test]
    fn observation_serialize_deserialize_roundtrip() {
        let obs = Observation {
            id: "obs-001".to_string(),
            node_fqn: "src/main.rs::main".to_string(),
            observation_text: "This function handles startup".to_string(),
            agent_id: "claude".to_string(),
            node_hash_at_write: "hash123".to_string(),
            written_at: 1715270300,
            status: "active".to_string(),
            stale_reason: None,
        };

        let serialized = serde_json::to_string(&obs).unwrap();
        let deserialized: Observation = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, obs.id);
        assert_eq!(deserialized.node_fqn, obs.node_fqn);
        assert_eq!(deserialized.observation_text, obs.observation_text);
        assert_eq!(deserialized.agent_id, obs.agent_id);
        assert_eq!(deserialized.node_hash_at_write, obs.node_hash_at_write);
        assert_eq!(deserialized.written_at, obs.written_at);
        assert_eq!(deserialized.status, obs.status);
        assert_eq!(deserialized.stale_reason, obs.stale_reason);
    }

    #[test]
    fn adr_serialize_deserialize_roundtrip() {
        let adr = Adr {
            id: "adr-001".to_string(),
            title: "Use SQLite for storage".to_string(),
            body: "We chose SQLite because...".to_string(),
            status: "accepted".to_string(),
            linked_fqn: Some("src/store/mod.rs::StoreManager".to_string()),
            created_at: 1715270300,
            updated_at: 1715270400,
        };

        let serialized = serde_json::to_string(&adr).unwrap();
        let deserialized: Adr = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, adr.id);
        assert_eq!(deserialized.title, adr.title);
        assert_eq!(deserialized.body, adr.body);
        assert_eq!(deserialized.status, adr.status);
        assert_eq!(deserialized.linked_fqn, adr.linked_fqn);
        assert_eq!(deserialized.created_at, adr.created_at);
        assert_eq!(deserialized.updated_at, adr.updated_at);
    }

    #[test]
    fn change_note_serialize_deserialize_roundtrip() {
        let note = ChangeNote {
            id: "cn-001".to_string(),
            text: "Refactored auth module".to_string(),
            created_at: 1715270300,
        };

        let serialized = serde_json::to_string(&note).unwrap();
        let deserialized: ChangeNote = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, note.id);
        assert_eq!(deserialized.text, note.text);
        assert_eq!(deserialized.created_at, note.created_at);
    }

    #[test]
    fn bundle_metadata_serialize_deserialize_roundtrip() {
        let meta = BundleMetadata {
            id: 1,
            format_version: 1,
            last_export_at: Some(1715270400),
            export_checksum: Some("sha256:abc123".to_string()),
            repo_root_hash: Some("sha256:def456".to_string()),
        };

        let serialized = serde_json::to_string(&meta).unwrap();
        let deserialized: BundleMetadata = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, meta.id);
        assert_eq!(deserialized.format_version, meta.format_version);
        assert_eq!(deserialized.last_export_at, meta.last_export_at);
        assert_eq!(deserialized.export_checksum, meta.export_checksum);
        assert_eq!(deserialized.repo_root_hash, meta.repo_root_hash);
    }

    #[test]
    fn extraction_result_serialize_deserialize_roundtrip() {
        let result = ExtractionResult {
            nodes: vec![Node {
                fqn: "src/main.rs::main".to_string(),
                kind: NodeKind::Function,
                file: "src/main.rs".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "abc123".to_string(),
                indexed_at: 1715270300,
                attributes: json!({}),
            }],
            edges: vec![Edge {
                id: None,
                source_fqn: "src/main.rs::main".to_string(),
                target_fqn: "src/lib.rs::run".to_string(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: json!({}),
            }],
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: ExtractionResult = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.nodes.len(), 1);
        assert_eq!(deserialized.edges.len(), 1);
        assert_eq!(deserialized.nodes[0].fqn, "src/main.rs::main");
        assert_eq!(deserialized.edges[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn invalid_node_kind_deserialization_fails() {
        let invalid = "\"InvalidKind\"";
        let result: Result<NodeKind, _> = serde_json::from_str(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_edge_kind_deserialization_fails() {
        let invalid = "\"InvalidKind\"";
        let result: Result<EdgeKind, _> = serde_json::from_str(invalid);
        assert!(result.is_err());
    }
}
