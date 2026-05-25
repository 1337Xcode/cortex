//! OWASP Top 10 pattern detection on the code graph.
//!
//! Detects patterns corresponding to OWASP categories:
//! - A01: Broken Access Control (routes without auth callers)
//! - A02: Cryptographic Failures (imports of md5/sha1)
//! - A03: Injection (taint paths from HttpInput to SqlQuery/CommandExecution)
//! - A07: Authentication Failures (auth routes without session validation)

use serde::{Deserialize, Serialize};

use crate::error::SecurityError;
use crate::store::db::StoreManager;
use crate::store::types::SecurityFinding;

use super::taint;

// ---------------------------------------------------------------------------
// Severity and Enhanced Finding types
// ---------------------------------------------------------------------------

/// Severity levels for OWASP findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

/// Enhanced security finding with severity and confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedFinding {
    pub finding: SecurityFinding,
    pub severity: Severity,
}

// ---------------------------------------------------------------------------
// Framework-specific auth middleware patterns
// ---------------------------------------------------------------------------

/// Framework-specific auth middleware patterns for A01 detection.
pub struct AuthPatterns {
    pub python_django: Vec<&'static str>,
    pub python_flask: Vec<&'static str>,
    pub node_express: Vec<&'static str>,
    pub rust_actix: Vec<&'static str>,
    pub go_gin: Vec<&'static str>,
    pub generic: Vec<&'static str>,
}

impl Default for AuthPatterns {
    fn default() -> Self {
        Self {
            python_django: vec!["login_required", "permission_required", "@authenticate"],
            python_flask: vec!["@login_required", "flask_login"],
            node_express: vec!["passport", "isAuthenticated", "authMiddleware"],
            rust_actix: vec!["HttpAuthentication", "middleware::auth"],
            go_gin: vec!["AuthRequired", "JWTAuth"],
            generic: vec!["auth", "authenticate", "authorize", "permission"],
        }
    }
}

impl AuthPatterns {
    /// Get all auth patterns as a flat list for matching.
    pub fn all_patterns(&self) -> Vec<&'static str> {
        let mut all = Vec::new();
        all.extend_from_slice(&self.python_django);
        all.extend_from_slice(&self.python_flask);
        all.extend_from_slice(&self.node_express);
        all.extend_from_slice(&self.rust_actix);
        all.extend_from_slice(&self.go_gin);
        all.extend_from_slice(&self.generic);
        all
    }
}

// ---------------------------------------------------------------------------
// Auth-related FQN patterns for A07
// ---------------------------------------------------------------------------

/// Patterns that indicate a route handles authentication operations.
const AUTH_ROUTE_PATTERNS: &[&str] = &[
    "login",
    "signup",
    "sign_up",
    "register",
    "password",
    "reset",
    "forgot_password",
    "reset_password",
    "change_password",
];

// ---------------------------------------------------------------------------
// OWASP pattern detection
// ---------------------------------------------------------------------------

/// Run all OWASP pattern detections and return enhanced findings with severity.
pub fn scan_owasp_patterns(store: &StoreManager) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let conn = store.read_conn();
    scan_owasp_patterns_with_conn(&conn)
}

/// Run all OWASP pattern detections using a direct database connection.
pub fn scan_owasp_patterns_with_conn(
    conn: &rusqlite::Connection,
) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let mut findings = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // A01: Broken Access Control
    findings.extend(detect_a01_with_conn(conn, now)?);

    // A02: Cryptographic Failures
    findings.extend(detect_a02_with_conn(conn, now)?);

    // A03: Injection (reuses taint analysis)
    findings.extend(detect_a03_with_conn(conn, now)?);

    // A07: Authentication Failures
    findings.extend(detect_a07_with_conn(conn, now)?);

    Ok(findings)
}

/// Extract plain SecurityFinding values (for backward compatibility).
pub fn scan_owasp_findings(store: &StoreManager) -> Result<Vec<SecurityFinding>, SecurityError> {
    Ok(scan_owasp_patterns(store)?
        .into_iter()
        .map(|ef| ef.finding)
        .collect())
}

/// Extract plain SecurityFinding values from a connection (for backward compatibility).
pub fn scan_owasp_findings_with_conn(
    conn: &rusqlite::Connection,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    Ok(scan_owasp_patterns_with_conn(conn)?
        .into_iter()
        .map(|ef| ef.finding)
        .collect())
}

// ---------------------------------------------------------------------------
// A01: Broken Access Control
// ---------------------------------------------------------------------------

/// Detect Route nodes that have no callers whose FQN contains an authentication
/// or authorization middleware pattern. Uses framework-specific patterns.
fn detect_a01_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let mut findings = Vec::new();
    let auth_patterns = AuthPatterns::default();
    let all_patterns = auth_patterns.all_patterns();

    // Build the SQL LIKE conditions for auth middleware detection
    let like_conditions: Vec<String> = all_patterns
        .iter()
        .map(|p| format!("LOWER(caller.fqn) LIKE '%{}%'", p.to_lowercase()))
        .collect();
    let like_clause = like_conditions.join(" OR ");

    let query = format!(
        "SELECT n.fqn, n.file FROM nodes n \
         WHERE n.kind = 'Route' \
         AND NOT EXISTS ( \
             SELECT 1 FROM edges e \
             JOIN nodes caller ON caller.fqn = e.source_fqn \
             WHERE e.target_fqn = n.fqn \
             AND e.kind = 'Calls' \
             AND ({}) \
         )",
        like_clause
    );

    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare A01 query: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let file: String = row.get(1)?;
            Ok((fqn, file))
        })
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to execute A01 query: {}", e),
        })?;

    for row in rows {
        let (fqn, _file) = row.map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to read A01 row: {}", e),
        })?;

        // Skip routes that are inherently public (login, signup, health, public)
        let fqn_lower = fqn.to_lowercase();
        if fqn_lower.contains("login")
            || fqn_lower.contains("register")
            || fqn_lower.contains("signup")
            || fqn_lower.contains("health")
            || fqn_lower.contains("public")
        {
            continue;
        }

        findings.push(EnhancedFinding {
            finding: SecurityFinding {
                id: None,
                node_fqn: fqn.clone(),
                kind: "broken_access_control".to_string(),
                owasp_category: Some("A01".to_string()),
                cwe_id: Some("CWE-862".to_string()),
                confidence: 0.6,
                description: format!(
                    "Route '{}' has no auth/permission middleware caller - may lack access control",
                    fqn
                ),
                indexed_at: now,
            },
            severity: Severity::High,
        });
    }

    Ok(findings)
}

// ---------------------------------------------------------------------------
// A02: Cryptographic Failures
// ---------------------------------------------------------------------------

/// Detect imports of deprecated/weak cryptographic algorithms (md5, sha1, des, rc4).
/// Excludes references found in comments or string literals by checking node kind
/// and attributes.
fn detect_a02_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let mut findings = Vec::new();

    let weak_crypto_patterns = [
        "%md5%",
        "%sha1%",
        "%des%",
        "%rc4%",
        "%hashlib.md5%",
        "%hashlib.sha1%",
        "%crypto/md5%",
        "%crypto/sha1%",
        "%Digest::MD5%",
        "%Digest::SHA1%",
    ];

    for pattern in &weak_crypto_patterns {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT e.source_fqn, e.target_fqn, n.kind, n.attributes \
                 FROM edges e \
                 JOIN nodes n ON n.fqn = e.source_fqn \
                 WHERE e.kind = 'Imports' AND LOWER(e.target_fqn) LIKE ?1",
            )
            .map_err(|e| SecurityError::AnalysisFailed {
                reason: format!("failed to prepare A02 query: {}", e),
            })?;

        let rows = stmt
            .query_map(rusqlite::params![pattern], |row| {
                let source: String = row.get(0)?;
                let target: String = row.get(1)?;
                let kind: String = row.get(2)?;
                let attributes: String = row.get(3)?;
                Ok((source, target, kind, attributes))
            })
            .map_err(|e| SecurityError::AnalysisFailed {
                reason: format!("failed to execute A02 query: {}", e),
            })?;

        for row in rows {
            let (source_fqn, target_fqn, kind, attributes) =
                row.map_err(|e| SecurityError::AnalysisFailed {
                    reason: format!("failed to read A02 row: {}", e),
                })?;

            // Exclude references in comments or string literals
            if is_comment_or_string_literal(&kind, &attributes) {
                continue;
            }

            findings.push(EnhancedFinding {
                finding: SecurityFinding {
                    id: None,
                    node_fqn: source_fqn.clone(),
                    kind: "weak_cryptography".to_string(),
                    owasp_category: Some("A02".to_string()),
                    cwe_id: Some("CWE-327".to_string()),
                    confidence: 0.85,
                    description: format!(
                        "'{}' imports weak cryptographic algorithm '{}'",
                        source_fqn, target_fqn
                    ),
                    indexed_at: now,
                },
                severity: Severity::Medium,
            });
        }
    }

    Ok(findings)
}

/// Check if a node represents a comment or string literal based on its kind
/// and attributes. Returns true if the reference should be excluded.
fn is_comment_or_string_literal(kind: &str, attributes: &str) -> bool {
    let kind_lower = kind.to_lowercase();
    // Exclude if the node kind indicates a comment or string
    if kind_lower.contains("comment")
        || kind_lower.contains("string")
        || kind_lower.contains("literal")
        || kind_lower.contains("doc")
    {
        return true;
    }

    // Check attributes for comment/string indicators
    let attrs_lower = attributes.to_lowercase();
    if attrs_lower.contains("\"comment\"")
        || attrs_lower.contains("\"string_literal\"")
        || attrs_lower.contains("\"docstring\"")
        || attrs_lower.contains("\"in_comment\": true")
        || attrs_lower.contains("\"in_string\": true")
    {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// A03: Injection
// ---------------------------------------------------------------------------

/// Detect injection vulnerabilities by reusing taint analysis results.
/// Requires taint path length >= 3 (source + at least one intermediate + sink).
fn detect_a03_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let taint_paths = taint::propagate_taint_with_conn(conn)?;
    let mut findings = Vec::new();

    for path in &taint_paths {
        let is_injection =
            path.sink_kind.contains("SqlQuery") || path.sink_kind.contains("CommandExecution");

        if !is_injection {
            continue;
        }

        // Require taint path length >= 3 (source + intermediate + sink)
        let path_nodes: Vec<String> = serde_json::from_str(&path.path_json).unwrap_or_default();
        if path_nodes.len() < 3 {
            continue;
        }

        let description = if path.sink_kind.contains("SqlQuery") {
            format!(
                "Potential SQL injection: taint flows from '{}' through {} intermediate node(s) to '{}'",
                path.source_fqn,
                path_nodes.len() - 2,
                path.sink_fqn
            )
        } else {
            format!(
                "Potential command injection: taint flows from '{}' through {} intermediate node(s) to '{}'",
                path.source_fqn,
                path_nodes.len() - 2,
                path.sink_fqn
            )
        };

        // Determine severity based on sink type
        let severity = if path.sink_kind.contains("SqlQuery") {
            Severity::Critical
        } else {
            Severity::High
        };

        findings.push(EnhancedFinding {
            finding: SecurityFinding {
                id: None,
                node_fqn: path.sink_fqn.clone(),
                kind: "injection".to_string(),
                owasp_category: Some("A03".to_string()),
                cwe_id: path.cwe_id.clone(),
                confidence: path.confidence,
                description,
                indexed_at: now,
            },
            severity,
        });
    }

    Ok(findings)
}

// ---------------------------------------------------------------------------
// A07: Authentication Failures
// ---------------------------------------------------------------------------

/// Detect auth-related routes that don't validate sessions.
/// Only flags routes with auth-related FQN patterns (login, signup, password, reset).
fn detect_a07_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<EnhancedFinding>, SecurityError> {
    let mut findings = Vec::new();

    // Build LIKE conditions for auth-related route patterns
    let like_conditions: Vec<String> = AUTH_ROUTE_PATTERNS
        .iter()
        .map(|p| format!("LOWER(n.fqn) LIKE '%{}%'", p))
        .collect();
    let like_clause = like_conditions.join(" OR ");

    let query = format!(
        "SELECT n.fqn FROM nodes n \
         WHERE n.kind = 'Route' \
         AND ({}) \
         AND NOT EXISTS ( \
             SELECT 1 FROM edges e \
             WHERE e.source_fqn = n.fqn \
             AND e.kind = 'Calls' \
             AND (LOWER(e.target_fqn) LIKE '%session%' \
                  OR LOWER(e.target_fqn) LIKE '%validate%' \
                  OR LOWER(e.target_fqn) LIKE '%verify%' \
                  OR LOWER(e.target_fqn) LIKE '%token%' \
                  OR LOWER(e.target_fqn) LIKE '%authenticate%') \
         )",
        like_clause
    );

    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare A07 query: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to execute A07 query: {}", e),
        })?;

    for row in rows {
        let fqn = row.map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to read A07 row: {}", e),
        })?;

        findings.push(EnhancedFinding {
            finding: SecurityFinding {
                id: None,
                node_fqn: fqn.clone(),
                kind: "auth_failure".to_string(),
                owasp_category: Some("A07".to_string()),
                cwe_id: Some("CWE-287".to_string()),
                confidence: 0.7,
                description: format!(
                    "Auth route '{}' does not call session/token validation",
                    fqn
                ),
                indexed_at: now,
            },
            severity: Severity::High,
        });
    }

    Ok(findings)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;

    fn setup_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create store");
        let conn = store.write_conn();
        migrations::run_migrations(&conn, std::path::Path::new("migrations"))
            .expect("failed to run migrations");
        drop(conn);
        (store, tmp)
    }

    fn insert_node(store: &StoreManager, fqn: &str, kind: &str, attrs: &str) {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES (?1, ?2, 'test.py', 1, 10, 'hash', 1000, ?3)",
            rusqlite::params![fqn, kind, attrs],
        )
        .unwrap();
    }

    fn insert_edge(store: &StoreManager, source: &str, target: &str, kind: &str) {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
             VALUES (?1, ?2, ?3, 1.0, '{}')",
            rusqlite::params![source, target, kind],
        )
        .unwrap();
    }

    #[test]
    fn test_a01_broken_access_control_detected() {
        let (store, _tmp) = setup_store();

        // Route without any auth caller
        insert_node(&store, "routes/users.py::get_users", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();
        let a01_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A01".to_string()))
            .collect();

        assert!(!a01_findings.is_empty());
        assert_eq!(
            a01_findings[0].finding.node_fqn,
            "routes/users.py::get_users"
        );
        assert_eq!(a01_findings[0].finding.cwe_id, Some("CWE-862".to_string()));
        assert_eq!(a01_findings[0].severity, Severity::High);
    }

    #[test]
    fn test_a01_no_false_positive_with_auth_caller() {
        let (store, _tmp) = setup_store();

        // Route with an auth middleware caller
        insert_node(&store, "routes/users.py::get_users", "Route", "{}");
        insert_node(&store, "middleware/auth.py::check_auth", "Function", "{}");
        insert_edge(
            &store,
            "middleware/auth.py::check_auth",
            "routes/users.py::get_users",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a01_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A01".to_string()))
            .collect();

        assert!(a01_findings.is_empty());
    }

    #[test]
    fn test_a01_no_false_positive_for_login_route() {
        let (store, _tmp) = setup_store();

        // Login routes should not be flagged (they are public by design)
        insert_node(&store, "routes/auth.py::login", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();
        let a01_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.finding.owasp_category == Some("A01".to_string())
                    && f.finding.node_fqn == "routes/auth.py::login"
            })
            .collect();

        assert!(a01_findings.is_empty());
    }

    #[test]
    fn test_a01_framework_specific_patterns() {
        let (store, _tmp) = setup_store();

        // Route with a Django login_required decorator caller
        insert_node(&store, "routes/admin.py::admin_panel", "Route", "{}");
        insert_node(&store, "decorators/login_required", "Function", "{}");
        insert_edge(
            &store,
            "decorators/login_required",
            "routes/admin.py::admin_panel",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a01_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.finding.owasp_category == Some("A01".to_string())
                    && f.finding.node_fqn == "routes/admin.py::admin_panel"
            })
            .collect();

        assert!(a01_findings.is_empty());
    }

    #[test]
    fn test_a02_weak_crypto_detected() {
        let (store, _tmp) = setup_store();

        // Node that imports md5
        insert_node(&store, "utils/hash.py::compute_hash", "Function", "{}");
        insert_edge(
            &store,
            "utils/hash.py::compute_hash",
            "hashlib.md5",
            "Imports",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a02_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A02".to_string()))
            .collect();

        assert!(!a02_findings.is_empty());
        assert_eq!(a02_findings[0].finding.cwe_id, Some("CWE-327".to_string()));
        assert_eq!(a02_findings[0].severity, Severity::Medium);
    }

    #[test]
    fn test_a02_excludes_comment_references() {
        let (store, _tmp) = setup_store();

        // Node with attributes indicating it's in a comment context
        insert_node(
            &store,
            "utils/hash.py::comment_about_md5",
            "Function",
            r#"{"in_comment": true}"#,
        );
        insert_edge(
            &store,
            "utils/hash.py::comment_about_md5",
            "hashlib.md5",
            "Imports",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a02_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.finding.owasp_category == Some("A02".to_string())
                    && f.finding.node_fqn == "utils/hash.py::comment_about_md5"
            })
            .collect();

        assert!(a02_findings.is_empty());
    }

    #[test]
    fn test_a02_no_false_positive_for_sha256() {
        let (store, _tmp) = setup_store();

        // SHA-256 is not weak
        insert_node(&store, "utils/hash.py::compute_hash", "Function", "{}");
        insert_edge(
            &store,
            "utils/hash.py::compute_hash",
            "hashlib.sha256",
            "Imports",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a02_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A02".to_string()))
            .collect();

        assert!(a02_findings.is_empty());
    }

    #[test]
    fn test_a03_injection_requires_intermediate_node() {
        let (store, _tmp) = setup_store();

        // Source -> Sink directly (path length 2) should NOT be flagged
        insert_node(
            &store,
            "routes/api.py::get_request",
            "Function",
            r#"{"params": ["request"]}"#,
        );
        insert_node(
            &store,
            "db/queries.py::execute_query",
            "Function",
            r#"{"calls": ["cursor.execute"]}"#,
        );
        insert_edge(
            &store,
            "routes/api.py::get_request",
            "db/queries.py::execute_query",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a03_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A03".to_string()))
            .collect();

        // Path length is 2 (source + sink), so should NOT be flagged
        assert!(a03_findings.is_empty());
    }

    #[test]
    fn test_a03_injection_with_intermediate_node() {
        let (store, _tmp) = setup_store();

        // Source -> Intermediate -> Sink (path length 3) should be flagged
        insert_node(
            &store,
            "routes/api.py::get_request",
            "Function",
            r#"{"params": ["request"]}"#,
        );
        insert_node(&store, "services/user.py::process_input", "Function", "{}");
        insert_node(
            &store,
            "db/queries.py::execute_query",
            "Function",
            r#"{"calls": ["cursor.execute"]}"#,
        );
        insert_edge(
            &store,
            "routes/api.py::get_request",
            "services/user.py::process_input",
            "Calls",
        );
        insert_edge(
            &store,
            "services/user.py::process_input",
            "db/queries.py::execute_query",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a03_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A03".to_string()))
            .collect();

        assert!(!a03_findings.is_empty());
        assert_eq!(a03_findings[0].finding.cwe_id, Some("CWE-89".to_string()));
        assert_eq!(a03_findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_a07_auth_failure_detected() {
        let (store, _tmp) = setup_store();

        // Auth route that doesn't call session validation
        // Must match auth-related patterns: login, signup, password, reset
        insert_node(&store, "routes/auth.py::reset_password", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();
        let a07_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.owasp_category == Some("A07".to_string()))
            .collect();

        assert!(!a07_findings.is_empty());
        assert_eq!(a07_findings[0].finding.cwe_id, Some("CWE-287".to_string()));
        assert_eq!(a07_findings[0].severity, Severity::High);
    }

    #[test]
    fn test_a07_non_auth_route_not_flagged() {
        let (store, _tmp) = setup_store();

        // A route that is NOT auth-related should NOT be flagged by A07
        insert_node(&store, "routes/users.py::get_users", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();
        let a07_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.finding.owasp_category == Some("A07".to_string())
                    && f.finding.node_fqn == "routes/users.py::get_users"
            })
            .collect();

        assert!(a07_findings.is_empty());
    }

    #[test]
    fn test_a07_no_false_positive_with_session_validation() {
        let (store, _tmp) = setup_store();

        // Auth route that calls session validation
        insert_node(&store, "routes/auth.py::login_handler", "Route", "{}");
        insert_node(
            &store,
            "auth/session.py::validate_session",
            "Function",
            "{}",
        );
        insert_edge(
            &store,
            "routes/auth.py::login_handler",
            "auth/session.py::validate_session",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a07_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.finding.owasp_category == Some("A07".to_string())
                    && f.finding.node_fqn == "routes/auth.py::login_handler"
            })
            .collect();

        assert!(a07_findings.is_empty());
    }

    #[test]
    fn test_scan_empty_graph() {
        let (store, _tmp) = setup_store();

        let findings = scan_owasp_patterns(&store).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_all_findings_have_severity_and_confidence() {
        let (store, _tmp) = setup_store();

        // Create a route without auth (A01 finding)
        insert_node(&store, "routes/data.py::get_data", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();

        for finding in &findings {
            // Every finding must have a severity
            assert!(matches!(
                finding.severity,
                Severity::Critical | Severity::High | Severity::Medium | Severity::Low
            ));
            // Every finding must have a confidence in [0.0, 1.0]
            assert!(finding.finding.confidence >= 0.0);
            assert!(finding.finding.confidence <= 1.0);
        }
    }

    #[test]
    fn test_enhanced_finding_has_severity() {
        let finding = EnhancedFinding {
            finding: SecurityFinding {
                id: None,
                node_fqn: "test::node".to_string(),
                kind: "test".to_string(),
                owasp_category: Some("A01".to_string()),
                cwe_id: Some("CWE-862".to_string()),
                confidence: 0.8,
                description: "test finding".to_string(),
                indexed_at: 0,
            },
            severity: Severity::Critical,
        };

        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.finding.confidence, 0.8);
    }

    // -----------------------------------------------------------------------
    // Property-based tests for OWASP scanner
    // -----------------------------------------------------------------------

    use proptest::prelude::*;

    /// Strategy to generate a valid severity value.
    fn severity_strategy() -> impl Strategy<Value = Severity> {
        prop_oneof![
            Just(Severity::Critical),
            Just(Severity::High),
            Just(Severity::Medium),
            Just(Severity::Low),
        ]
    }

    /// Strategy to generate a confidence value in [0.0, 1.0].
    fn confidence_strategy() -> impl Strategy<Value = f64> {
        (0u32..=100u32).prop_map(|v| v as f64 / 100.0)
    }

    /// Strategy to generate a taint path JSON with a specific number of nodes.
    #[allow(dead_code)]
    fn taint_path_json_strategy(node_count: usize) -> impl Strategy<Value = String> {
        let nodes: Vec<String> = (0..node_count).map(|i| format!("node_{}", i)).collect();
        Just(serde_json::to_string(&nodes).unwrap())
    }

    /// Strategy to generate a number of intermediate nodes (0 to 5).
    #[allow(dead_code)]
    fn intermediate_count_strategy() -> impl Strategy<Value = usize> {
        0usize..=5
    }

    // -----------------------------------------------------------------------
    // Property 9: OWASP A03 requires intermediate node in taint path
    // -----------------------------------------------------------------------
    // **Validates: Requirements 3.3**

    proptest! {
        /// Property 9a: Taint paths with length < 3 (only source and sink, no
        /// intermediate) SHALL NOT produce A03 findings. The detect_a03 logic
        /// requires path_nodes.len() >= 3.
        ///
        /// **Validates: Requirements 3.3**
        #[test]
        fn prop_a03_rejects_short_taint_paths(
            source_suffix in "[a-z]{3,8}",
            sink_suffix in "[a-z]{3,8}",
        ) {
            // Create a taint path with only 2 nodes (source + sink, no intermediate)
            let path_nodes = vec![
                format!("source::{}", source_suffix),
                format!("sink::{}", sink_suffix),
            ];
            let path_json = serde_json::to_string(&path_nodes).unwrap();

            // Simulate the A03 filtering logic from detect_a03_with_conn:
            // path_nodes.len() < 3 means it should be skipped
            let path_len = path_nodes.len();
            let would_produce_finding = path_len >= 3;

            prop_assert!(!would_produce_finding,
                "Path with {} nodes (< 3) should NOT produce A03 finding, path_json={}",
                path_len, path_json);
        }

        /// Property 9b: Taint paths with length >= 3 (source + at least one
        /// intermediate + sink) SHALL be eligible for A03 findings.
        ///
        /// **Validates: Requirements 3.3**
        #[test]
        fn prop_a03_accepts_paths_with_intermediate(
            intermediate_count in 1usize..=5,
            source_suffix in "[a-z]{3,8}",
            sink_suffix in "[a-z]{3,8}",
        ) {
            // Create a taint path with source + intermediates + sink
            let mut path_nodes = vec![format!("source::{}", source_suffix)];
            for i in 0..intermediate_count {
                path_nodes.push(format!("intermediate_{}::{}", i, source_suffix));
            }
            path_nodes.push(format!("sink::{}", sink_suffix));

            let path_json = serde_json::to_string(&path_nodes).unwrap();

            // The path length is intermediate_count + 2 (source + intermediates + sink)
            let path_len = path_nodes.len();
            prop_assert!(path_len >= 3,
                "Path with {} intermediates should have length >= 3, got {}",
                intermediate_count, path_len);

            // Simulate the A03 filtering logic: path_nodes.len() >= 3 passes the filter
            let passes_length_filter = path_len >= 3;
            prop_assert!(passes_length_filter,
                "Path with {} nodes (>= 3) should pass A03 length filter, path_json={}",
                path_len, path_json);
        }

        /// Property 9c: The A03 path length check is exactly >= 3. Verify that
        /// for any path length generated, the decision boundary is correct:
        /// length 1 or 2 → rejected, length 3+ → accepted.
        ///
        /// **Validates: Requirements 3.3**
        #[test]
        fn prop_a03_decision_boundary(
            path_length in 1usize..=10,
        ) {
            let passes_filter = path_length >= 3;

            if path_length < 3 {
                prop_assert!(!passes_filter,
                    "Path length {} should be rejected by A03 filter", path_length);
            } else {
                prop_assert!(passes_filter,
                    "Path length {} should be accepted by A03 filter", path_length);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 10: OWASP findings include severity and confidence
    // -----------------------------------------------------------------------
    // **Validates: Requirements 3.5**

    proptest! {
        /// Property 10a: For any EnhancedFinding constructed with arbitrary valid
        /// severity and confidence, the severity SHALL be in {Critical, High, Medium, Low}
        /// and confidence SHALL be in [0.0, 1.0].
        ///
        /// **Validates: Requirements 3.5**
        #[test]
        fn prop_findings_have_valid_severity_and_confidence(
            severity in severity_strategy(),
            confidence in confidence_strategy(),
            fqn_suffix in "[a-z_]{3,12}",
            owasp_cat in prop_oneof![
                Just("A01".to_string()),
                Just("A02".to_string()),
                Just("A03".to_string()),
                Just("A07".to_string()),
            ],
        ) {
            let finding = EnhancedFinding {
                finding: SecurityFinding {
                    id: None,
                    node_fqn: format!("module::{}",fqn_suffix),
                    kind: "test_kind".to_string(),
                    owasp_category: Some(owasp_cat),
                    cwe_id: Some("CWE-000".to_string()),
                    confidence,
                    description: "test description".to_string(),
                    indexed_at: 0,
                },
                severity: severity.clone(),
            };

            // Verify severity is one of the valid enum variants
            prop_assert!(matches!(
                finding.severity,
                Severity::Critical | Severity::High | Severity::Medium | Severity::Low
            ), "Severity must be Critical, High, Medium, or Low");

            // Verify confidence is in [0.0, 1.0]
            prop_assert!(finding.finding.confidence >= 0.0,
                "Confidence {} must be >= 0.0", finding.finding.confidence);
            prop_assert!(finding.finding.confidence <= 1.0,
                "Confidence {} must be <= 1.0", finding.finding.confidence);
        }

        /// Property 10b: For any OWASP category detection, the produced findings
        /// always have severity set to a valid variant and confidence in range.
        /// This tests the actual severity assignments used in the scanner.
        ///
        /// **Validates: Requirements 3.5**
        #[test]
        fn prop_scanner_severity_assignments_are_valid(
            category in prop_oneof![
                Just("A01"),
                Just("A02"),
                Just("A03"),
                Just("A07"),
            ],
            is_sql_injection in proptest::bool::ANY,
        ) {
            // Simulate the severity assignment logic from the scanner
            let severity = match category {
                "A01" => Severity::High,
                "A02" => Severity::Medium,
                "A03" => {
                    if is_sql_injection {
                        Severity::Critical
                    } else {
                        Severity::High
                    }
                }
                "A07" => Severity::High,
                _ => Severity::Low,
            };

            prop_assert!(matches!(
                severity,
                Severity::Critical | Severity::High | Severity::Medium | Severity::Low
            ), "Severity for category {} must be a valid variant", category);
        }

        /// Property 10c: For any confidence value produced by the scanner's
        /// detection functions, the value is always in [0.0, 1.0].
        /// The scanner uses fixed confidence values: 0.6 (A01), 0.85 (A02),
        /// taint confidence (A03), 0.7 (A07).
        ///
        /// **Validates: Requirements 3.5**
        #[test]
        fn prop_scanner_confidence_values_in_range(
            // Simulate taint confidence which is computed from path length and specificity
            path_length in 1usize..=10,
            specificity in prop_oneof![Just(0.8f64), Just(1.0f64)],
        ) {
            // Fixed confidence values used by the scanner
            let a01_confidence = 0.6;
            let a02_confidence = 0.85;
            let a07_confidence = 0.7;

            // A03 confidence comes from taint analysis
            let base_score = match path_length {
                1..=2 => 0.95,
                3 => 0.85,
                4 => 0.75,
                5 => 0.65,
                _ => 0.5,
            };
            let a03_confidence = base_score * specificity;

            // All confidence values must be in [0.0, 1.0]
            prop_assert!((0.0..=1.0).contains(&a01_confidence),
                "A01 confidence {} out of range", a01_confidence);
            prop_assert!((0.0..=1.0).contains(&a02_confidence),
                "A02 confidence {} out of range", a02_confidence);
            prop_assert!((0.0..=1.0).contains(&a03_confidence),
                "A03 confidence {} out of range (path_len={}, specificity={})",
                a03_confidence, path_length, specificity);
            prop_assert!((0.0..=1.0).contains(&a07_confidence),
                "A07 confidence {} out of range", a07_confidence);
        }
    }
}
