//! CWE (Common Weakness Enumeration) classification for security findings.
//!
//! Maps detected security patterns to their corresponding CWE identifiers
//! and provides descriptions for reporting.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CWE classification types
// ---------------------------------------------------------------------------

/// A CWE classification with ID, name, and description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CweClassification {
    pub id: String,
    pub name: String,
    pub description: String,
    pub severity: CweSeverity,
}

/// Severity level for a CWE finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CweSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

// ---------------------------------------------------------------------------
// CWE database (subset relevant to our detection)
// ---------------------------------------------------------------------------

/// Get the CWE classification for a given CWE ID.
pub fn get_cwe_info(cwe_id: &str) -> Option<CweClassification> {
    match cwe_id {
        "CWE-22" => Some(CweClassification {
            id: "CWE-22".to_string(),
            name: "Path Traversal".to_string(),
            description: "Improper limitation of a pathname to a restricted directory".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-78" => Some(CweClassification {
            id: "CWE-78".to_string(),
            name: "OS Command Injection".to_string(),
            description: "Improper neutralization of special elements used in an OS command"
                .to_string(),
            severity: CweSeverity::Critical,
        }),
        "CWE-79" => Some(CweClassification {
            id: "CWE-79".to_string(),
            name: "Cross-site Scripting (XSS)".to_string(),
            description: "Improper neutralization of input during web page generation".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-89" => Some(CweClassification {
            id: "CWE-89".to_string(),
            name: "SQL Injection".to_string(),
            description: "Improper neutralization of special elements used in an SQL command"
                .to_string(),
            severity: CweSeverity::Critical,
        }),
        "CWE-117" => Some(CweClassification {
            id: "CWE-117".to_string(),
            name: "Log Injection".to_string(),
            description: "Improper output neutralization for logs".to_string(),
            severity: CweSeverity::Medium,
        }),
        "CWE-259" => Some(CweClassification {
            id: "CWE-259".to_string(),
            name: "Hard-coded Password".to_string(),
            description: "Use of hard-coded password".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-284" => Some(CweClassification {
            id: "CWE-284".to_string(),
            name: "Improper Access Control".to_string(),
            description: "The software does not restrict or incorrectly restricts access to a resource".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-287" => Some(CweClassification {
            id: "CWE-287".to_string(),
            name: "Improper Authentication".to_string(),
            description: "The software does not sufficiently verify that a claim of identity is correct".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-327" => Some(CweClassification {
            id: "CWE-327".to_string(),
            name: "Broken Cryptographic Algorithm".to_string(),
            description: "Use of a broken or risky cryptographic algorithm".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-306" => Some(CweClassification {
            id: "CWE-306".to_string(),
            name: "Missing Authentication".to_string(),
            description: "Missing authentication for critical function".to_string(),
            severity: CweSeverity::High,
        }),
        "CWE-798" => Some(CweClassification {
            id: "CWE-798".to_string(),
            name: "Hard-coded Credentials".to_string(),
            description: "Use of hard-coded credentials".to_string(),
            severity: CweSeverity::Critical,
        }),
        "CWE-862" => Some(CweClassification {
            id: "CWE-862".to_string(),
            name: "Missing Authorization".to_string(),
            description: "The software does not perform an authorization check when an actor attempts to access a resource".to_string(),
            severity: CweSeverity::High,
        }),
        _ => None,
    }
}

/// Map an OWASP category to its primary CWE IDs.
pub fn owasp_to_cwes(owasp_category: &str) -> Vec<&'static str> {
    match owasp_category {
        "A01" => vec!["CWE-862", "CWE-284"],
        "A02" => vec!["CWE-327", "CWE-259", "CWE-798"],
        "A03" => vec!["CWE-89", "CWE-78", "CWE-79"],
        "A07" => vec!["CWE-287", "CWE-306"],
        _ => vec![],
    }
}

/// Classify a security finding based on its OWASP category and context.
///
/// Maps OWASP categories to their primary CWE ID, using context to disambiguate
/// when multiple CWEs are possible (e.g., A03 can be CWE-89 or CWE-78).
pub fn classify_finding(owasp_category: &str, context: &str) -> Option<String> {
    let ctx_lower = context.to_lowercase();
    match owasp_category {
        "A01" => Some("CWE-862".to_string()),
        "A02" => Some("CWE-327".to_string()),
        "A03" => {
            if ctx_lower.contains("command")
                || ctx_lower.contains("exec")
                || ctx_lower.contains("shell")
            {
                Some("CWE-78".to_string())
            } else {
                // Default to SQL injection for A03
                Some("CWE-89".to_string())
            }
        }
        "A07" => Some("CWE-287".to_string()),
        _ => None,
    }
}

/// Classify a SecurityFinding into a CweClassification.
///
/// Uses the finding's owasp_category and description to determine the CWE.
pub fn classify_security_finding(
    finding: &crate::store::types::SecurityFinding,
) -> Option<CweClassification> {
    // If the finding already has a CWE ID, look it up directly
    if let Some(ref cwe_id) = finding.cwe_id {
        return get_cwe_info(cwe_id);
    }

    // Otherwise, classify based on OWASP category and description
    if let Some(ref owasp_cat) = finding.owasp_category {
        let cwe_id = classify_finding(owasp_cat, &finding.description)?;
        return get_cwe_info(&cwe_id);
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cwe_info_known_ids() {
        let cwe89 = get_cwe_info("CWE-89").unwrap();
        assert_eq!(cwe89.id, "CWE-89");
        assert_eq!(cwe89.name, "SQL Injection");
        assert_eq!(cwe89.severity, CweSeverity::Critical);

        let cwe78 = get_cwe_info("CWE-78").unwrap();
        assert_eq!(cwe78.id, "CWE-78");
        assert_eq!(cwe78.name, "OS Command Injection");
        assert_eq!(cwe78.severity, CweSeverity::Critical);

        let cwe22 = get_cwe_info("CWE-22").unwrap();
        assert_eq!(cwe22.id, "CWE-22");
        assert_eq!(cwe22.name, "Path Traversal");
        assert_eq!(cwe22.severity, CweSeverity::High);
    }

    #[test]
    fn test_get_cwe_info_unknown_id() {
        assert!(get_cwe_info("CWE-9999").is_none());
        assert!(get_cwe_info("invalid").is_none());
    }

    #[test]
    fn test_owasp_to_cwes_mapping() {
        let a01 = owasp_to_cwes("A01");
        assert!(a01.contains(&"CWE-862"));
        assert!(a01.contains(&"CWE-284"));

        let a02 = owasp_to_cwes("A02");
        assert!(a02.contains(&"CWE-327"));

        let a03 = owasp_to_cwes("A03");
        assert!(a03.contains(&"CWE-89"));
        assert!(a03.contains(&"CWE-78"));

        let a07 = owasp_to_cwes("A07");
        assert!(a07.contains(&"CWE-287"));
        assert!(a07.contains(&"CWE-306"));
    }

    #[test]
    fn test_owasp_to_cwes_unknown_category() {
        let unknown = owasp_to_cwes("A99");
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_cwe_severity_serialization() {
        let classification = CweClassification {
            id: "CWE-89".to_string(),
            name: "SQL Injection".to_string(),
            description: "test".to_string(),
            severity: CweSeverity::Critical,
        };

        let json = serde_json::to_string(&classification).unwrap();
        let deserialized: CweClassification = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.severity, CweSeverity::Critical);
    }

    #[test]
    fn test_classify_finding_a01() {
        let result = classify_finding("A01", "route without auth");
        assert_eq!(result, Some("CWE-862".to_string()));
    }

    #[test]
    fn test_classify_finding_a02() {
        let result = classify_finding("A02", "uses md5 hash");
        assert_eq!(result, Some("CWE-327".to_string()));
    }

    #[test]
    fn test_classify_finding_a03_sql() {
        let result = classify_finding("A03", "taint flows to sql query");
        assert_eq!(result, Some("CWE-89".to_string()));
    }

    #[test]
    fn test_classify_finding_a03_command() {
        let result = classify_finding("A03", "taint flows to command execution");
        assert_eq!(result, Some("CWE-78".to_string()));
    }

    #[test]
    fn test_classify_finding_a07() {
        let result = classify_finding("A07", "auth route without session");
        assert_eq!(result, Some("CWE-287".to_string()));
    }

    #[test]
    fn test_classify_finding_unknown_category() {
        let result = classify_finding("A99", "unknown");
        assert_eq!(result, None);
    }

    #[test]
    fn test_classify_security_finding_with_existing_cwe() {
        use crate::store::types::SecurityFinding;
        let finding = SecurityFinding {
            id: None,
            node_fqn: "test::func".to_string(),
            kind: "injection".to_string(),
            owasp_category: Some("A03".to_string()),
            cwe_id: Some("CWE-89".to_string()),
            confidence: 0.9,
            description: "SQL injection".to_string(),
            indexed_at: 1000,
        };
        let classification = classify_security_finding(&finding);
        assert!(classification.is_some());
        assert_eq!(classification.unwrap().id, "CWE-89");
    }

    #[test]
    fn test_classify_security_finding_from_owasp_category() {
        use crate::store::types::SecurityFinding;
        let finding = SecurityFinding {
            id: None,
            node_fqn: "test::route".to_string(),
            kind: "broken_access_control".to_string(),
            owasp_category: Some("A01".to_string()),
            cwe_id: None,
            confidence: 0.6,
            description: "route without auth".to_string(),
            indexed_at: 1000,
        };
        let classification = classify_security_finding(&finding);
        assert!(classification.is_some());
        assert_eq!(classification.unwrap().id, "CWE-862");
    }
}
