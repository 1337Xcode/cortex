//! OWASP Top 10 pattern detection on the code graph.
//!
//! Detects patterns corresponding to OWASP categories:
//! - A01: Broken Access Control (routes without auth callers)
//! - A02: Cryptographic Failures (imports of md5/sha1)
//! - A03: Injection (taint paths from HttpInput to SqlQuery/CommandExecution)
//! - A07: Authentication Failures (auth routes without session validation)

use crate::error::SecurityError;
use crate::store::db::StoreManager;
use crate::store::types::SecurityFinding;

use super::taint;

// ---------------------------------------------------------------------------
// OWASP pattern detection
// ---------------------------------------------------------------------------

/// Run all OWASP pattern detections and return findings.
pub fn scan_owasp_patterns(store: &StoreManager) -> Result<Vec<SecurityFinding>, SecurityError> {
    let mut findings = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // A01: Broken Access Control
    findings.extend(detect_a01_broken_access_control(store, now)?);

    // A02: Cryptographic Failures
    findings.extend(detect_a02_cryptographic_failures(store, now)?);

    // A03: Injection (reuses taint analysis)
    findings.extend(detect_a03_injection(store, now)?);

    // A07: Authentication Failures
    findings.extend(detect_a07_auth_failures(store, now)?);

    Ok(findings)
}

/// Run all OWASP pattern detections using a direct database connection.
pub fn scan_owasp_patterns_with_conn(
    conn: &rusqlite::Connection,
) -> Result<Vec<SecurityFinding>, SecurityError> {
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

// ---------------------------------------------------------------------------
// A01: Broken Access Control
// ---------------------------------------------------------------------------

/// Detect Route nodes that have no callers with "auth" or "permission" in their FQN.
/// This suggests the route may lack access control checks.
fn detect_a01_broken_access_control(
    store: &StoreManager,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let conn = store.read_conn();
    detect_a01_with_conn(&conn, now)
}

// ---------------------------------------------------------------------------
// A02: Cryptographic Failures
// ---------------------------------------------------------------------------

/// Detect imports of deprecated/weak cryptographic algorithms (md5, sha1).
fn detect_a02_cryptographic_failures(
    store: &StoreManager,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let conn = store.read_conn();
    detect_a02_with_conn(&conn, now)
}

// ---------------------------------------------------------------------------
// A03: Injection
// ---------------------------------------------------------------------------

/// Detect injection vulnerabilities by reusing taint analysis results.
/// Looks for taint paths from HttpInput to SqlQuery or CommandExecution.
fn detect_a03_injection(
    store: &StoreManager,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let taint_paths = taint::propagate_taint(store)?;
    let mut findings = Vec::new();

    for path in &taint_paths {
        let is_injection = path.sink_kind.contains("SqlQuery")
            || path.sink_kind.contains("CommandExecution");

        if !is_injection {
            continue;
        }

        let description = if path.sink_kind.contains("SqlQuery") {
            format!(
                "Potential SQL injection: taint flows from '{}' to '{}'",
                path.source_fqn, path.sink_fqn
            )
        } else {
            format!(
                "Potential command injection: taint flows from '{}' to '{}'",
                path.source_fqn, path.sink_fqn
            )
        };

        findings.push(SecurityFinding {
            id: None,
            node_fqn: path.sink_fqn.clone(),
            kind: "injection".to_string(),
            owasp_category: Some("A03".to_string()),
            cwe_id: path.cwe_id.clone(),
            confidence: path.confidence,
            description,
            indexed_at: now,
        });
    }

    Ok(findings)
}

// ---------------------------------------------------------------------------
// A07: Authentication Failures
// ---------------------------------------------------------------------------

/// Detect auth-related routes that don't validate sessions.
/// Looks for Route nodes with "auth" in their FQN that don't call session/token validation.
fn detect_a07_auth_failures(
    store: &StoreManager,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let conn = store.read_conn();
    detect_a07_with_conn(&conn, now)
}

// ---------------------------------------------------------------------------
// Connection-based detection functions (shared logic)
// ---------------------------------------------------------------------------

/// A01 detection using a direct connection.
fn detect_a01_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let mut findings = Vec::new();

    let mut stmt = conn
        .prepare(
            "SELECT n.fqn, n.file FROM nodes n \
             WHERE n.kind = 'Route' \
             AND NOT EXISTS ( \
                 SELECT 1 FROM edges e \
                 JOIN nodes caller ON caller.fqn = e.source_fqn \
                 WHERE e.target_fqn = n.fqn \
                 AND e.kind = 'Calls' \
                 AND (LOWER(caller.fqn) LIKE '%auth%' \
                      OR LOWER(caller.fqn) LIKE '%permission%' \
                      OR LOWER(caller.fqn) LIKE '%middleware%' \
                      OR LOWER(caller.fqn) LIKE '%guard%' \
                      OR LOWER(caller.fqn) LIKE '%protect%') \
             )",
        )
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

        let fqn_lower = fqn.to_lowercase();
        if fqn_lower.contains("login")
            || fqn_lower.contains("register")
            || fqn_lower.contains("signup")
            || fqn_lower.contains("health")
            || fqn_lower.contains("public")
        {
            continue;
        }

        findings.push(SecurityFinding {
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
        });
    }

    Ok(findings)
}

/// A02 detection using a direct connection.
fn detect_a02_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let mut findings = Vec::new();

    let weak_crypto_patterns = [
        "%md5%", "%sha1%", "%des%", "%rc4%", "%hashlib.md5%",
        "%hashlib.sha1%", "%crypto/md5%", "%crypto/sha1%",
        "%Digest::MD5%", "%Digest::SHA1%",
    ];

    for pattern in &weak_crypto_patterns {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT e.source_fqn, e.target_fqn FROM edges e \
                 WHERE e.kind = 'Imports' AND LOWER(e.target_fqn) LIKE ?1",
            )
            .map_err(|e| SecurityError::AnalysisFailed {
                reason: format!("failed to prepare A02 query: {}", e),
            })?;

        let rows = stmt
            .query_map(rusqlite::params![pattern], |row| {
                let source: String = row.get(0)?;
                let target: String = row.get(1)?;
                Ok((source, target))
            })
            .map_err(|e| SecurityError::AnalysisFailed {
                reason: format!("failed to execute A02 query: {}", e),
            })?;

        for row in rows {
            let (source_fqn, target_fqn) = row.map_err(|e| SecurityError::AnalysisFailed {
                reason: format!("failed to read A02 row: {}", e),
            })?;

            findings.push(SecurityFinding {
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
            });
        }
    }

    // Also check node attributes for weak crypto usage
    let mut stmt = conn
        .prepare(
            "SELECT fqn FROM nodes \
             WHERE LOWER(attributes) LIKE '%md5%' \
             OR LOWER(attributes) LIKE '%sha1%' \
             OR LOWER(attributes) LIKE '%\"des\"%'",
        )
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare A02 attributes query: {}", e),
        })?;

    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to execute A02 attributes query: {}", e),
        })?;

    for row in rows {
        let fqn = row.map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to read A02 attributes row: {}", e),
        })?;

        if findings.iter().any(|f| f.node_fqn == fqn) {
            continue;
        }

        findings.push(SecurityFinding {
            id: None,
            node_fqn: fqn.clone(),
            kind: "weak_cryptography".to_string(),
            owasp_category: Some("A02".to_string()),
            cwe_id: Some("CWE-327".to_string()),
            confidence: 0.7,
            description: format!(
                "'{}' uses weak cryptographic algorithm (md5/sha1/des)",
                fqn
            ),
            indexed_at: now,
        });
    }

    Ok(findings)
}

/// A03 detection using a direct connection (reuses taint analysis).
fn detect_a03_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let taint_paths = taint::propagate_taint_with_conn(conn)?;
    let mut findings = Vec::new();

    for path in &taint_paths {
        let is_injection = path.sink_kind.contains("SqlQuery")
            || path.sink_kind.contains("CommandExecution");

        if !is_injection {
            continue;
        }

        let description = if path.sink_kind.contains("SqlQuery") {
            format!(
                "Potential SQL injection: taint flows from '{}' to '{}'",
                path.source_fqn, path.sink_fqn
            )
        } else {
            format!(
                "Potential command injection: taint flows from '{}' to '{}'",
                path.source_fqn, path.sink_fqn
            )
        };

        findings.push(SecurityFinding {
            id: None,
            node_fqn: path.sink_fqn.clone(),
            kind: "injection".to_string(),
            owasp_category: Some("A03".to_string()),
            cwe_id: path.cwe_id.clone(),
            confidence: path.confidence,
            description,
            indexed_at: now,
        });
    }

    Ok(findings)
}

/// A07 detection using a direct connection.
fn detect_a07_with_conn(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<Vec<SecurityFinding>, SecurityError> {
    let mut findings = Vec::new();

    let mut stmt = conn
        .prepare(
            "SELECT n.fqn FROM nodes n \
             WHERE n.kind = 'Route' \
             AND (LOWER(n.fqn) LIKE '%auth%' OR LOWER(n.fqn) LIKE '%login%' \
                  OR LOWER(n.fqn) LIKE '%session%') \
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
        )
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

        findings.push(SecurityFinding {
            id: None,
            node_fqn: fqn.clone(),
            kind: "auth_failure".to_string(),
            owasp_category: Some("A07".to_string()),
            cwe_id: Some("CWE-287".to_string()),
            confidence: 0.55,
            description: format!(
                "Auth route '{}' does not call session/token validation",
                fqn
            ),
            indexed_at: now,
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
            .filter(|f| f.owasp_category == Some("A01".to_string()))
            .collect();

        assert!(!a01_findings.is_empty());
        assert_eq!(a01_findings[0].node_fqn, "routes/users.py::get_users");
        assert_eq!(a01_findings[0].cwe_id, Some("CWE-862".to_string()));
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
            .filter(|f| f.owasp_category == Some("A01".to_string()))
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
                f.owasp_category == Some("A01".to_string())
                    && f.node_fqn == "routes/auth.py::login"
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
            .filter(|f| f.owasp_category == Some("A02".to_string()))
            .collect();

        assert!(!a02_findings.is_empty());
        assert_eq!(a02_findings[0].cwe_id, Some("CWE-327".to_string()));
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
            .filter(|f| f.owasp_category == Some("A02".to_string()))
            .collect();

        assert!(a02_findings.is_empty());
    }

    #[test]
    fn test_a03_injection_detected() {
        let (store, _tmp) = setup_store();

        // Source -> Sink taint path for SQL injection
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
            .filter(|f| f.owasp_category == Some("A03".to_string()))
            .collect();

        assert!(!a03_findings.is_empty());
        assert_eq!(a03_findings[0].cwe_id, Some("CWE-89".to_string()));
    }

    #[test]
    fn test_a07_auth_failure_detected() {
        let (store, _tmp) = setup_store();

        // Auth route that doesn't call session validation
        insert_node(&store, "routes/auth.py::check_auth_status", "Route", "{}");

        let findings = scan_owasp_patterns(&store).unwrap();
        let a07_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.owasp_category == Some("A07".to_string()))
            .collect();

        assert!(!a07_findings.is_empty());
        assert_eq!(a07_findings[0].cwe_id, Some("CWE-287".to_string()));
    }

    #[test]
    fn test_a07_no_false_positive_with_session_validation() {
        let (store, _tmp) = setup_store();

        // Auth route that calls session validation
        insert_node(&store, "routes/auth.py::check_auth_status", "Route", "{}");
        insert_node(&store, "auth/session.py::validate_session", "Function", "{}");
        insert_edge(
            &store,
            "routes/auth.py::check_auth_status",
            "auth/session.py::validate_session",
            "Calls",
        );

        let findings = scan_owasp_patterns(&store).unwrap();
        let a07_findings: Vec<_> = findings
            .iter()
            .filter(|f| {
                f.owasp_category == Some("A07".to_string())
                    && f.node_fqn == "routes/auth.py::check_auth_status"
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
}
