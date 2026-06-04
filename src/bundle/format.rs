//! Bundle format definition for portable committed bundles.
//!
//! Defines the JSON schema for Cortex bundle files, including version
//! compatibility constants and validation logic.

use serde::{Deserialize, Serialize};

use crate::store::types::{Adr, Edge, Node, Observation, SbomEntry, SecurityFinding, TaintPath};

// ---------------------------------------------------------------------------
// Version constants
// ---------------------------------------------------------------------------

/// Current bundle format version produced by this binary.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// Minimum format version this binary can import.
pub const MIN_SUPPORTED_FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Bundle schema types
// ---------------------------------------------------------------------------

/// Metadata about the bundle export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSchema {
    #[serde(rename = "format_version")]
    pub format_version: u32,
    #[serde(rename = "cortex_version")]
    pub cortex_version: String,
    #[serde(rename = "repo_root_hash")]
    pub repo_root_hash: String,
    #[serde(rename = "exported_at")]
    pub exported_at: i64,
}

/// The complete portable bundle containing all graph data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CortexBundle {
    #[serde(rename = "schema")]
    pub schema: BundleSchema,
    #[serde(rename = "nodes")]
    pub nodes: Vec<Node>,
    #[serde(rename = "edges")]
    pub edges: Vec<Edge>,
    #[serde(rename = "security_findings")]
    pub security_findings: Vec<SecurityFinding>,
    #[serde(rename = "taint_paths")]
    pub taint_paths: Vec<TaintPath>,
    #[serde(rename = "observations")]
    pub observations: Vec<Observation>,
    #[serde(rename = "adrs")]
    pub adrs: Vec<Adr>,
    #[serde(rename = "sbom_entries")]
    pub sbom_entries: Vec<SbomEntry>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validates that a bundle's format version is compatible with this binary.
///
/// Returns `Ok(())` if the version is within the supported range, or an error
/// describing the incompatibility.
pub fn validate_version(format_version: u32) -> Result<(), crate::error::BundleError> {
    if format_version < MIN_SUPPORTED_FORMAT_VERSION {
        return Err(crate::error::BundleError::VersionIncompatible {
            found: format_version,
            expected: MIN_SUPPORTED_FORMAT_VERSION,
        });
    }
    if format_version > CURRENT_FORMAT_VERSION {
        return Err(crate::error::BundleError::VersionIncompatible {
            found: format_version,
            expected: CURRENT_FORMAT_VERSION,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::types::{EdgeKind, NodeKind};
    use serde_json::json;

    #[test]
    fn serialize_deserialize_roundtrip() {
        let bundle = CortexBundle {
            schema: BundleSchema {
                format_version: CURRENT_FORMAT_VERSION,
                cortex_version: "0.1.0".to_string(),
                repo_root_hash: "sha256:abc123".to_string(),
                exported_at: 1715270400,
            },
            nodes: vec![Node {
                fqn: "src/main.rs::main".to_string(),
                kind: NodeKind::Function,
                file: "src/main.rs".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "sha256:def456".to_string(),
                indexed_at: 1715270300,
                attributes: json!({}),
            }],
            edges: vec![Edge {
                id: None,
                source_fqn: "src/main.rs::main".to_string(),
                target_fqn: "src/lib.rs::run".to_string(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                edge_source: crate::store::confidence::EdgeSource::AstDirect,
                attributes: json!({}),
            }],
            security_findings: vec![],
            taint_paths: vec![],
            observations: vec![],
            adrs: vec![],
            sbom_entries: vec![],
        };

        let json_str = serde_json::to_string_pretty(&bundle).unwrap();
        let deserialized: CortexBundle = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.schema.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(deserialized.schema.cortex_version, "0.1.0");
        assert_eq!(deserialized.nodes.len(), 1);
        assert_eq!(deserialized.edges.len(), 1);
        assert_eq!(deserialized.nodes[0].fqn, "src/main.rs::main");
    }

    #[test]
    fn incompatible_version_too_high() {
        let result = validate_version(CURRENT_FORMAT_VERSION + 1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("incompatible"));
    }

    #[test]
    fn incompatible_version_too_low() {
        // Only testable if MIN > 0; version 0 is always below minimum.
        let result = validate_version(0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("incompatible"));
    }

    #[test]
    fn current_version_is_valid() {
        assert!(validate_version(CURRENT_FORMAT_VERSION).is_ok());
        assert!(validate_version(MIN_SUPPORTED_FORMAT_VERSION).is_ok());
    }
}
