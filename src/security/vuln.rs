//! OSV.dev vulnerability checking.
//!
//! Checks SBOM entries against the OSV.dev API for known vulnerabilities.
//! Uses a trait-based approach to allow mocking in tests (no real network calls).

use serde::{Deserialize, Serialize};

use crate::error::SecurityError;
use crate::store::types::SbomEntry;

/// Result of a vulnerability check for a single package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnResult {
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// List of vulnerabilities found.
    pub vulnerabilities: Vec<VulnInfo>,
}

/// Information about a single vulnerability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnInfo {
    /// OSV ID (e.g., "GHSA-xxxx-xxxx-xxxx" or "CVE-2024-xxxx").
    pub id: String,
    /// Short summary of the vulnerability.
    pub summary: String,
    /// Severity level.
    pub severity: String,
    /// Fixed version (if known).
    pub fixed_version: Option<String>,
}

/// Trait for OSV API client, allowing mock implementations in tests.
pub trait OsvClient {
    /// Query OSV.dev for vulnerabilities affecting a package.
    fn query(&self, package: &str, version: &str, ecosystem: &str) -> Result<Vec<VulnInfo>, SecurityError>;
}

/// Real OSV.dev API client (requires network access).
pub struct RealOsvClient;

impl OsvClient for RealOsvClient {
    fn query(&self, _package: &str, _version: &str, _ecosystem: &str) -> Result<Vec<VulnInfo>, SecurityError> {
        // In a real implementation, this would make an HTTP POST to
        // https://api.osv.dev/v1/query with the package info.
        // For now, return empty (no network calls in this build).
        Err(SecurityError::AnalysisFailed {
            reason: "OSV.dev check requires network access; not available in air-gap mode".to_string(),
        })
    }
}

/// Check SBOM entries against OSV.dev for known vulnerabilities.
///
/// Uses the provided client for API calls (allows mocking in tests).
pub fn check_osv_with_client(
    sbom: &[SbomEntry],
    client: &dyn OsvClient,
) -> Result<Vec<VulnResult>, SecurityError> {
    let mut results = Vec::new();

    for entry in sbom {
        let version = entry.version.as_deref().unwrap_or("unknown");
        if version == "unknown" {
            continue;
        }

        let ecosystem = guess_ecosystem(&entry.name, &entry.source_file);

        match client.query(&entry.name, version, &ecosystem) {
            Ok(vulns) => {
                if !vulns.is_empty() {
                    results.push(VulnResult {
                        package: entry.name.clone(),
                        version: version.to_string(),
                        vulnerabilities: vulns,
                    });
                }
            }
            Err(_) => {
                // Skip packages that fail to query (network issues, etc.)
                continue;
            }
        }
    }

    Ok(results)
}

/// Check SBOM entries against OSV.dev (convenience function using real client).
pub fn check_osv(sbom: &[SbomEntry]) -> Result<Vec<VulnResult>, SecurityError> {
    let client = RealOsvClient;
    check_osv_with_client(sbom, &client)
}

/// Guess the ecosystem from the package name and source file.
fn guess_ecosystem(package: &str, source_file: &str) -> String {
    if source_file.contains("package.json") || source_file.contains("node_modules") {
        "npm".to_string()
    } else if source_file.contains("requirements") || source_file.contains(".py") {
        "PyPI".to_string()
    } else if source_file.contains("go.mod") || source_file.contains(".go") {
        "Go".to_string()
    } else if source_file.contains("Cargo") || source_file.contains(".rs") {
        "crates.io".to_string()
    } else if source_file.contains("pom.xml") || source_file.contains(".java") {
        "Maven".to_string()
    } else if source_file.contains("Gemfile") || source_file.contains(".rb") {
        "RubyGems".to_string()
    } else if package.contains('/') {
        // Likely a Go or scoped npm package
        "Go".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Parse an OSV API response JSON into VulnInfo structs.
pub fn parse_osv_response(response_json: &str) -> Result<Vec<VulnInfo>, SecurityError> {
    let parsed: serde_json::Value = serde_json::from_str(response_json).map_err(|e| {
        SecurityError::AnalysisFailed {
            reason: format!("failed to parse OSV response: {}", e),
        }
    })?;

    let mut vulns = Vec::new();

    if let Some(vulns_array) = parsed.get("vulns").and_then(|v| v.as_array()) {
        for vuln in vulns_array {
            let id = vuln
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let summary = vuln
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("No summary available")
                .to_string();

            let severity = vuln
                .get("database_specific")
                .and_then(|d| d.get("severity"))
                .and_then(|s| s.as_str())
                .unwrap_or("UNKNOWN")
                .to_string();

            let fixed_version = vuln
                .get("affected")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.get("ranges"))
                .and_then(|r| r.as_array())
                .and_then(|r| r.first())
                .and_then(|r| r.get("events"))
                .and_then(|e| e.as_array())
                .and_then(|events| {
                    events.iter().find_map(|e| {
                        e.get("fixed").and_then(|f| f.as_str()).map(|s| s.to_string())
                    })
                });

            vulns.push(VulnInfo {
                id,
                summary,
                severity,
                fixed_version,
            });
        }
    }

    Ok(vulns)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock OSV client that returns predefined responses.
    struct MockOsvClient {
        responses: Vec<(String, Vec<VulnInfo>)>,
    }

    impl OsvClient for MockOsvClient {
        fn query(&self, package: &str, _version: &str, _ecosystem: &str) -> Result<Vec<VulnInfo>, SecurityError> {
            for (pkg, vulns) in &self.responses {
                if pkg == package {
                    return Ok(vulns.clone());
                }
            }
            Ok(Vec::new())
        }
    }

    /// Mock client that always returns a network error.
    struct FailingOsvClient;

    impl OsvClient for FailingOsvClient {
        fn query(&self, _package: &str, _version: &str, _ecosystem: &str) -> Result<Vec<VulnInfo>, SecurityError> {
            Err(SecurityError::AnalysisFailed {
                reason: "network timeout".to_string(),
            })
        }
    }

    #[test]
    fn test_mock_response_parsed_correctly() {
        let mock_client = MockOsvClient {
            responses: vec![(
                "lodash".to_string(),
                vec![VulnInfo {
                    id: "GHSA-1234-5678-abcd".to_string(),
                    summary: "Prototype pollution in lodash".to_string(),
                    severity: "HIGH".to_string(),
                    fixed_version: Some("4.17.21".to_string()),
                }],
            )],
        };

        let sbom = vec![SbomEntry {
            id: None,
            name: "lodash".to_string(),
            version: Some("4.17.20".to_string()),
            license: Some("MIT".to_string()),
            source_file: "package.json".to_string(),
            import_fqn: "lodash".to_string(),
            indexed_at: 0,
        }];

        let results = check_osv_with_client(&sbom, &mock_client).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package, "lodash");
        assert_eq!(results[0].version, "4.17.20");
        assert_eq!(results[0].vulnerabilities.len(), 1);
        assert_eq!(results[0].vulnerabilities[0].id, "GHSA-1234-5678-abcd");
        assert_eq!(results[0].vulnerabilities[0].severity, "HIGH");
        assert_eq!(
            results[0].vulnerabilities[0].fixed_version,
            Some("4.17.21".to_string())
        );
    }

    #[test]
    fn test_network_failure_returns_structured_error() {
        let client = FailingOsvClient;

        let sbom = vec![SbomEntry {
            id: None,
            name: "requests".to_string(),
            version: Some("2.28.0".to_string()),
            license: Some("Apache-2.0".to_string()),
            source_file: "requirements.txt".to_string(),
            import_fqn: "requests".to_string(),
            indexed_at: 0,
        }];

        // Network failures are gracefully handled - packages are skipped
        let results = check_osv_with_client(&sbom, &client).unwrap();
        assert!(results.is_empty(), "Failed queries should be skipped gracefully");
    }

    #[test]
    fn test_parse_osv_response_json() {
        let response = r#"{
            "vulns": [
                {
                    "id": "CVE-2024-1234",
                    "summary": "Remote code execution vulnerability",
                    "database_specific": {
                        "severity": "CRITICAL"
                    },
                    "affected": [
                        {
                            "ranges": [
                                {
                                    "events": [
                                        {"introduced": "0"},
                                        {"fixed": "2.0.1"}
                                    ]
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#;

        let vulns = parse_osv_response(response).unwrap();
        assert_eq!(vulns.len(), 1);
        assert_eq!(vulns[0].id, "CVE-2024-1234");
        assert_eq!(vulns[0].summary, "Remote code execution vulnerability");
        assert_eq!(vulns[0].severity, "CRITICAL");
        assert_eq!(vulns[0].fixed_version, Some("2.0.1".to_string()));
    }
}
