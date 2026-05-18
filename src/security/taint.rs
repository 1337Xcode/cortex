//! Taint analysis: detect sources, sinks, and propagate taint through the call graph.
//!
//! Sources are AST patterns that introduce untrusted data (HTTP input, file reads, env vars).
//! Sinks are AST patterns that consume data in security-sensitive ways (SQL, commands, file writes).
//! Propagation uses BFS from source nodes through Calls edges to sink nodes.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::error::SecurityError;
use crate::store::db::StoreManager;
use crate::store::types::{Node, TaintPath};

// ---------------------------------------------------------------------------
// Taint source and sink kinds
// ---------------------------------------------------------------------------

/// Kinds of taint sources (where untrusted data enters the system).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaintSourceKind {
    /// HTTP request parameters, body, headers.
    HttpInput,
    /// Data read from files.
    FileInput,
    /// Environment variables.
    EnvVar,
    /// User session data.
    UserSession,
}

/// Kinds of taint sinks (where data is consumed in security-sensitive ways).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaintSinkKind {
    /// SQL query execution.
    SqlQuery,
    /// OS command execution.
    CommandExecution,
    /// File write operations.
    FileWrite,
    /// HTTP response output.
    HttpResponse,
    /// Log output (potential log injection).
    LogOutput,
}

/// A detected taint source in the graph.
#[derive(Debug, Clone)]
pub struct TaintSource {
    pub fqn: String,
    pub kind: TaintSourceKind,
}

/// A detected taint sink in the graph.
#[derive(Debug, Clone)]
pub struct TaintSink {
    pub fqn: String,
    pub kind: TaintSinkKind,
}

// ---------------------------------------------------------------------------
// Source/sink detection patterns
// ---------------------------------------------------------------------------

/// Patterns that indicate a taint source.
const HTTP_INPUT_PATTERNS: &[&str] = &[
    "request", "req.body", "req.params", "req.query", "req.headers",
    "flask.request", "Request", "HttpRequest", "get_json", "form_data",
    "request.GET", "request.POST", "request.args", "request.form",
    "r.Body", "c.Param", "c.Query", "c.PostForm", "gin.Context",
    "actix_web::HttpRequest", "axum::extract",
];

const FILE_INPUT_PATTERNS: &[&str] = &[
    "open(", "read_file", "fs.readFile", "fs.read", "io.ReadAll",
    "os.ReadFile", "std::fs::read", "File::open", "BufferedReader",
];

const ENV_VAR_PATTERNS: &[&str] = &[
    "os.environ", "os.getenv", "process.env", "os.Getenv",
    "std::env::var", "env::var", "System.getenv",
];

const USER_SESSION_PATTERNS: &[&str] = &[
    "session", "cookie", "jwt", "token", "auth_user",
    "current_user", "get_user", "req.user",
];

/// Patterns that indicate a taint sink.
const SQL_QUERY_PATTERNS: &[&str] = &[
    "execute", "query", "raw_sql", "cursor.execute", "db.query",
    "sql", "SELECT", "INSERT", "UPDATE", "DELETE",
    "conn.execute", "db.Exec", "db.Query", "sqlx::query",
];

const COMMAND_EXEC_PATTERNS: &[&str] = &[
    "subprocess", "os.system", "exec", "spawn", "popen",
    "child_process", "Command::new", "os/exec", "Runtime.exec",
    "shell_exec", "system(",
];

const FILE_WRITE_PATTERNS: &[&str] = &[
    "write", "fs.writeFile", "open.*w", "File::create",
    "os.WriteFile", "io.WriteString", "BufferedWriter",
];

const HTTP_RESPONSE_PATTERNS: &[&str] = &[
    "response", "res.send", "res.json", "res.write",
    "HttpResponse", "jsonify", "render_template",
    "c.JSON", "c.String", "c.HTML",
];

const LOG_OUTPUT_PATTERNS: &[&str] = &[
    "log", "logger", "logging", "console.log", "println",
    "tracing", "slog", "log.Printf", "log.Println",
];

// ---------------------------------------------------------------------------
// Language-specific source/sink patterns
// ---------------------------------------------------------------------------

/// Python-specific source patterns.
const PYTHON_SOURCE_PATTERNS: &[(&str, TaintSourceKind)] = &[
    ("request.args", TaintSourceKind::HttpInput),
    ("request.form", TaintSourceKind::HttpInput),
    ("request.get_json", TaintSourceKind::HttpInput),
    ("request.data", TaintSourceKind::HttpInput),
    ("request.values", TaintSourceKind::HttpInput),
    ("os.environ", TaintSourceKind::EnvVar),
    ("os.getenv", TaintSourceKind::EnvVar),
    ("open(", TaintSourceKind::FileInput),
    ("session", TaintSourceKind::UserSession),
];

/// Python-specific sink patterns.
const PYTHON_SINK_PATTERNS: &[(&str, TaintSinkKind)] = &[
    ("cursor.execute", TaintSinkKind::SqlQuery),
    ("execute(", TaintSinkKind::SqlQuery),
    ("subprocess.run", TaintSinkKind::CommandExecution),
    ("subprocess.call", TaintSinkKind::CommandExecution),
    ("subprocess.popen", TaintSinkKind::CommandExecution),
    ("os.system", TaintSinkKind::CommandExecution),
    ("open(", TaintSinkKind::FileWrite),
    ("logging", TaintSinkKind::LogOutput),
];

/// TypeScript-specific source patterns.
const TYPESCRIPT_SOURCE_PATTERNS: &[(&str, TaintSourceKind)] = &[
    ("req.body", TaintSourceKind::HttpInput),
    ("req.params", TaintSourceKind::HttpInput),
    ("req.query", TaintSourceKind::HttpInput),
    ("req.headers", TaintSourceKind::HttpInput),
    ("process.env", TaintSourceKind::EnvVar),
    ("fs.readfile", TaintSourceKind::FileInput),
    ("req.session", TaintSourceKind::UserSession),
    ("req.cookies", TaintSourceKind::UserSession),
];

/// TypeScript-specific sink patterns.
const TYPESCRIPT_SINK_PATTERNS: &[(&str, TaintSinkKind)] = &[
    ("query(", TaintSinkKind::SqlQuery),
    (".execute(", TaintSinkKind::SqlQuery),
    ("exec(", TaintSinkKind::CommandExecution),
    ("child_process", TaintSinkKind::CommandExecution),
    ("spawn(", TaintSinkKind::CommandExecution),
    ("fs.writefile", TaintSinkKind::FileWrite),
    ("res.send", TaintSinkKind::HttpResponse),
    ("res.json", TaintSinkKind::HttpResponse),
    ("console.log", TaintSinkKind::LogOutput),
];

/// Go-specific source patterns.
const GO_SOURCE_PATTERNS: &[(&str, TaintSourceKind)] = &[
    ("r.formvalue", TaintSourceKind::HttpInput),
    ("r.url.query", TaintSourceKind::HttpInput),
    ("r.body", TaintSourceKind::HttpInput),
    ("c.param", TaintSourceKind::HttpInput),
    ("c.query", TaintSourceKind::HttpInput),
    ("os.getenv", TaintSourceKind::EnvVar),
    ("os.readfile", TaintSourceKind::FileInput),
    ("r.cookie", TaintSourceKind::UserSession),
];

/// Go-specific sink patterns.
const GO_SINK_PATTERNS: &[(&str, TaintSinkKind)] = &[
    ("db.query", TaintSinkKind::SqlQuery),
    ("db.exec", TaintSinkKind::SqlQuery),
    ("db.queryrow", TaintSinkKind::SqlQuery),
    ("exec.command", TaintSinkKind::CommandExecution),
    ("os.writestring", TaintSinkKind::FileWrite),
    ("os.writefile", TaintSinkKind::FileWrite),
    ("fmt.fprintf", TaintSinkKind::HttpResponse),
    ("w.write", TaintSinkKind::HttpResponse),
    ("log.printf", TaintSinkKind::LogOutput),
    ("log.println", TaintSinkKind::LogOutput),
];

// ---------------------------------------------------------------------------
// Detection functions
// ---------------------------------------------------------------------------

/// Detect taint sources and sinks in node attributes based on language patterns.
///
/// Modifies node attributes in-place, adding `{"taint_source": "HttpInput"}` or
/// `{"taint_sink": "SqlQuery"}` to the attributes JSON of matching nodes.
pub fn detect_sources_sinks(nodes: &mut [Node], lang: &str) {
    let (source_patterns, sink_patterns): (
        &[(&str, TaintSourceKind)],
        &[(&str, TaintSinkKind)],
    ) = match lang {
        "python" | "py" => (PYTHON_SOURCE_PATTERNS, PYTHON_SINK_PATTERNS),
        "typescript" | "ts" | "javascript" | "js" => {
            (TYPESCRIPT_SOURCE_PATTERNS, TYPESCRIPT_SINK_PATTERNS)
        }
        "go" => (GO_SOURCE_PATTERNS, GO_SINK_PATTERNS),
        _ => {
            // For unsupported languages, fall back to generic detection
            for node in nodes.iter_mut() {
                detect_generic_source_sink(node);
            }
            return;
        }
    };

    for node in nodes.iter_mut() {
        let fqn_lower = node.fqn.to_lowercase();
        let attrs_str = node.attributes.to_string().to_lowercase();
        let combined = format!("{} {}", fqn_lower, attrs_str);

        // Check source patterns
        for (pattern, kind) in source_patterns {
            if combined.contains(*pattern) {
                if let Some(obj) = node.attributes.as_object_mut() {
                    obj.insert(
                        "taint_source".to_string(),
                        serde_json::Value::String(format!("{:?}", kind)),
                    );
                }
                break;
            }
        }

        // Check sink patterns
        for (pattern, kind) in sink_patterns {
            if combined.contains(*pattern) {
                if let Some(obj) = node.attributes.as_object_mut() {
                    obj.insert(
                        "taint_sink".to_string(),
                        serde_json::Value::String(format!("{:?}", kind)),
                    );
                }
                break;
            }
        }
    }
}

/// Detect generic source/sink for nodes when language is not specifically supported.
fn detect_generic_source_sink(node: &mut Node) {
    let fqn_lower = node.fqn.to_lowercase();
    let attrs_str = node.attributes.to_string().to_lowercase();
    let combined = format!("{} {}", fqn_lower, attrs_str);

    if matches_any_pattern(&combined, HTTP_INPUT_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_source".to_string(),
                serde_json::Value::String("HttpInput".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, ENV_VAR_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_source".to_string(),
                serde_json::Value::String("EnvVar".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, FILE_INPUT_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_source".to_string(),
                serde_json::Value::String("FileInput".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, USER_SESSION_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_source".to_string(),
                serde_json::Value::String("UserSession".to_string()),
            );
        }
    }

    if matches_any_pattern(&combined, SQL_QUERY_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_sink".to_string(),
                serde_json::Value::String("SqlQuery".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, COMMAND_EXEC_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_sink".to_string(),
                serde_json::Value::String("CommandExecution".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, FILE_WRITE_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_sink".to_string(),
                serde_json::Value::String("FileWrite".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, HTTP_RESPONSE_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_sink".to_string(),
                serde_json::Value::String("HttpResponse".to_string()),
            );
        }
    } else if matches_any_pattern(&combined, LOG_OUTPUT_PATTERNS) {
        if let Some(obj) = node.attributes.as_object_mut() {
            obj.insert(
                "taint_sink".to_string(),
                serde_json::Value::String("LogOutput".to_string()),
            );
        }
    }
}

/// Detect taint sources from a set of nodes based on their FQN and attributes.
pub fn detect_sources(nodes: &[Node]) -> Vec<TaintSource> {
    let mut sources = Vec::new();

    for node in nodes {
        let fqn_lower = node.fqn.to_lowercase();
        let attrs_str = node.attributes.to_string().to_lowercase();
        let combined = format!("{} {}", fqn_lower, attrs_str);

        if matches_any_pattern(&combined, HTTP_INPUT_PATTERNS) {
            sources.push(TaintSource {
                fqn: node.fqn.clone(),
                kind: TaintSourceKind::HttpInput,
            });
        } else if matches_any_pattern(&combined, FILE_INPUT_PATTERNS) {
            sources.push(TaintSource {
                fqn: node.fqn.clone(),
                kind: TaintSourceKind::FileInput,
            });
        } else if matches_any_pattern(&combined, ENV_VAR_PATTERNS) {
            sources.push(TaintSource {
                fqn: node.fqn.clone(),
                kind: TaintSourceKind::EnvVar,
            });
        } else if matches_any_pattern(&combined, USER_SESSION_PATTERNS) {
            sources.push(TaintSource {
                fqn: node.fqn.clone(),
                kind: TaintSourceKind::UserSession,
            });
        }
    }

    sources
}

/// Detect taint sinks from a set of nodes based on their FQN and attributes.
pub fn detect_sinks(nodes: &[Node]) -> Vec<TaintSink> {
    let mut sinks = Vec::new();

    for node in nodes {
        let fqn_lower = node.fqn.to_lowercase();
        let attrs_str = node.attributes.to_string().to_lowercase();
        let combined = format!("{} {}", fqn_lower, attrs_str);

        if matches_any_pattern(&combined, SQL_QUERY_PATTERNS) {
            sinks.push(TaintSink {
                fqn: node.fqn.clone(),
                kind: TaintSinkKind::SqlQuery,
            });
        } else if matches_any_pattern(&combined, COMMAND_EXEC_PATTERNS) {
            sinks.push(TaintSink {
                fqn: node.fqn.clone(),
                kind: TaintSinkKind::CommandExecution,
            });
        } else if matches_any_pattern(&combined, FILE_WRITE_PATTERNS) {
            sinks.push(TaintSink {
                fqn: node.fqn.clone(),
                kind: TaintSinkKind::FileWrite,
            });
        } else if matches_any_pattern(&combined, HTTP_RESPONSE_PATTERNS) {
            sinks.push(TaintSink {
                fqn: node.fqn.clone(),
                kind: TaintSinkKind::HttpResponse,
            });
        } else if matches_any_pattern(&combined, LOG_OUTPUT_PATTERNS) {
            sinks.push(TaintSink {
                fqn: node.fqn.clone(),
                kind: TaintSinkKind::LogOutput,
            });
        }
    }

    sinks
}

/// Check if a string matches any of the given patterns (case-insensitive substring match).
fn matches_any_pattern(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| text.contains(&p.to_lowercase()))
}

// ---------------------------------------------------------------------------
// CWE classification
// ---------------------------------------------------------------------------

/// Classify a taint path into a CWE ID based on source and sink kinds.
pub fn classify_cwe(source: &TaintSourceKind, sink: &TaintSinkKind) -> Option<String> {
    match (source, sink) {
        (TaintSourceKind::HttpInput, TaintSinkKind::SqlQuery) => Some("CWE-89".to_string()),
        (TaintSourceKind::HttpInput, TaintSinkKind::CommandExecution) => {
            Some("CWE-78".to_string())
        }
        (TaintSourceKind::HttpInput, TaintSinkKind::FileWrite) => Some("CWE-22".to_string()),
        (TaintSourceKind::HttpInput, TaintSinkKind::LogOutput) => Some("CWE-117".to_string()),
        (TaintSourceKind::HttpInput, TaintSinkKind::HttpResponse) => Some("CWE-79".to_string()),
        (TaintSourceKind::FileInput, TaintSinkKind::SqlQuery) => Some("CWE-89".to_string()),
        (TaintSourceKind::FileInput, TaintSinkKind::CommandExecution) => {
            Some("CWE-78".to_string())
        }
        (TaintSourceKind::EnvVar, TaintSinkKind::CommandExecution) => Some("CWE-78".to_string()),
        (TaintSourceKind::UserSession, TaintSinkKind::SqlQuery) => Some("CWE-89".to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Taint propagation via BFS
// ---------------------------------------------------------------------------

/// Propagate taint from sources to sinks via BFS through Calls edges.
///
/// Queries the graph store for all nodes and edges, detects sources/sinks,
/// then performs BFS from each source to find reachable sinks.
pub fn propagate_taint(store: &StoreManager) -> Result<Vec<TaintPath>, SecurityError> {
    let conn = store.read_conn();
    propagate_taint_with_conn(&conn)
}

/// Propagate taint from sources to sinks via BFS through Calls edges.
///
/// Takes a direct database connection reference. Loads all nodes and edges,
/// detects sources/sinks, then performs BFS from each source to find reachable sinks.
/// Returns detected taint paths with CWE classification.
pub fn propagate_taint_with_conn(conn: &rusqlite::Connection) -> Result<Vec<TaintPath>, SecurityError> {

    // Load all nodes
    let nodes = load_all_nodes(&conn)?;

    // Detect sources and sinks
    let sources = detect_sources(&nodes);
    let sinks = detect_sinks(&nodes);

    if sources.is_empty() || sinks.is_empty() {
        return Ok(Vec::new());
    }

    // Build adjacency list from Calls edges
    let adjacency = build_adjacency_list(&conn)?;

    // Build sink lookup
    let sink_map: HashMap<&str, &TaintSink> = sinks.iter().map(|s| (s.fqn.as_str(), s)).collect();

    let mut taint_paths = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // BFS from each source
    for source in &sources {
        let paths = bfs_to_sinks(&source.fqn, &adjacency, &sink_map);

        for (sink_fqn, path) in paths {
            let sink = sink_map[sink_fqn];
            let cwe_id = classify_cwe(&source.kind, &sink.kind);
            let path_json =
                serde_json::to_string(&path).unwrap_or_else(|_| "[]".to_string());

            taint_paths.push(TaintPath {
                id: None,
                source_fqn: source.fqn.clone(),
                source_kind: format!("{:?}", source.kind),
                sink_fqn: sink_fqn.to_string(),
                sink_kind: format!("{:?}", sink.kind),
                path_json,
                confidence: compute_confidence(path.len()),
                cwe_id,
                indexed_at: now,
            });
        }
    }

    Ok(taint_paths)
}

/// BFS from a source node to find all reachable sinks.
/// Returns a list of (sink_fqn, path) pairs.
fn bfs_to_sinks<'a>(
    source_fqn: &str,
    adjacency: &HashMap<String, Vec<String>>,
    sink_map: &HashMap<&'a str, &TaintSink>,
) -> Vec<(&'a str, Vec<String>)> {
    let mut results = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

    visited.insert(source_fqn.to_string());
    queue.push_back((source_fqn.to_string(), vec![source_fqn.to_string()]));

    // Limit BFS depth to prevent infinite loops
    let max_depth = 10;

    while let Some((current, path)) = queue.pop_front() {
        if path.len() > max_depth {
            continue;
        }

        // Check if current node is a sink (and not the source itself)
        if current != source_fqn {
            if let Some(&sink_fqn_key) = sink_map.keys().find(|&&k| k == current.as_str()) {
                results.push((sink_fqn_key, path.clone()));
                // Don't stop - continue BFS to find other sinks
            }
        }

        // Expand neighbors
        if let Some(neighbors) = adjacency.get(&current) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    visited.insert(neighbor.clone());
                    let mut new_path = path.clone();
                    new_path.push(neighbor.clone());
                    queue.push_back((neighbor.clone(), new_path));
                }
            }
        }
    }

    results
}

/// Compute confidence based on path length (shorter paths = higher confidence).
fn compute_confidence(path_len: usize) -> f64 {
    match path_len {
        0..=2 => 0.95,
        3 => 0.85,
        4 => 0.75,
        5 => 0.65,
        _ => 0.5,
    }
}

/// Load all nodes from the database.
fn load_all_nodes(
    conn: &rusqlite::Connection,
) -> Result<Vec<Node>, SecurityError> {
    let mut stmt = conn
        .prepare(
            "SELECT fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes FROM nodes",
        )
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare node query: {}", e),
        })?;

    let nodes = stmt
        .query_map([], |row| {
            let kind_str: String = row.get(1)?;
            let attrs_str: String = row.get(7)?;
            Ok(Node {
                fqn: row.get(0)?,
                kind: parse_node_kind(&kind_str),
                file: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                file_hash: row.get(5)?,
                indexed_at: row.get(6)?,
                attributes: serde_json::from_str(&attrs_str).unwrap_or_default(),
            })
        })
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to query nodes: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to collect nodes: {}", e),
        })?;

    Ok(nodes)
}

/// Build an adjacency list from Calls edges in the database.
fn build_adjacency_list(
    conn: &rusqlite::Connection,
) -> Result<HashMap<String, Vec<String>>, SecurityError> {
    let mut stmt = conn
        .prepare("SELECT source_fqn, target_fqn FROM edges WHERE kind = 'Calls'")
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare edge query: {}", e),
        })?;

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    let rows = stmt
        .query_map([], |row| {
            let source: String = row.get(0)?;
            let target: String = row.get(1)?;
            Ok((source, target))
        })
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to query edges: {}", e),
        })?;

    for row in rows {
        let (source, target) = row.map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to read edge row: {}", e),
        })?;
        adjacency.entry(source).or_default().push(target);
    }

    Ok(adjacency)
}

/// Parse a node kind string into a NodeKind enum.
fn parse_node_kind(s: &str) -> crate::store::types::NodeKind {
    match s {
        "Function" => crate::store::types::NodeKind::Function,
        "Class" => crate::store::types::NodeKind::Class,
        "Module" => crate::store::types::NodeKind::Module,
        "Route" => crate::store::types::NodeKind::Route,
        "Interface" => crate::store::types::NodeKind::Interface,
        "Type" => crate::store::types::NodeKind::Type,
        "Enum" => crate::store::types::NodeKind::Enum,
        "Constant" => crate::store::types::NodeKind::Constant,
        "TypeAlias" => crate::store::types::NodeKind::TypeAlias,
        "Trait" => crate::store::types::NodeKind::Trait,
        "Namespace" => crate::store::types::NodeKind::Namespace,
        _ => crate::store::types::NodeKind::Function,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;
    use serde_json::json;

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

    fn insert_edge(store: &StoreManager, source: &str, target: &str) {
        let conn = store.write_conn();
        conn.execute(
            "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
             VALUES (?1, ?2, 'Calls', 1.0, '{}')",
            rusqlite::params![source, target],
        )
        .unwrap();
    }

    #[test]
    fn test_detect_sources_http_input() {
        let nodes = vec![Node {
            fqn: "app/routes.py::handle_request".to_string(),
            kind: crate::store::types::NodeKind::Function,
            file: "app/routes.py".to_string(),
            start_line: 1,
            end_line: 10,
            file_hash: "hash".to_string(),
            indexed_at: 1000,
            attributes: json!({"params": ["request"]}),
        }];

        let sources = detect_sources(&nodes);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, TaintSourceKind::HttpInput);
    }

    #[test]
    fn test_detect_sinks_sql_query() {
        let nodes = vec![Node {
            fqn: "app/db.py::execute_query".to_string(),
            kind: crate::store::types::NodeKind::Function,
            file: "app/db.py".to_string(),
            start_line: 1,
            end_line: 10,
            file_hash: "hash".to_string(),
            indexed_at: 1000,
            attributes: json!({"calls": ["cursor.execute"]}),
        }];

        let sinks = detect_sinks(&nodes);
        assert_eq!(sinks.len(), 1);
        assert_eq!(sinks[0].kind, TaintSinkKind::SqlQuery);
    }

    #[test]
    fn test_cwe_classification() {
        assert_eq!(
            classify_cwe(&TaintSourceKind::HttpInput, &TaintSinkKind::SqlQuery),
            Some("CWE-89".to_string())
        );
        assert_eq!(
            classify_cwe(&TaintSourceKind::HttpInput, &TaintSinkKind::CommandExecution),
            Some("CWE-78".to_string())
        );
        assert_eq!(
            classify_cwe(&TaintSourceKind::HttpInput, &TaintSinkKind::FileWrite),
            Some("CWE-22".to_string())
        );
        assert_eq!(
            classify_cwe(&TaintSourceKind::HttpInput, &TaintSinkKind::LogOutput),
            Some("CWE-117".to_string())
        );
    }

    #[test]
    fn test_source_to_sink_path_stored() {
        let (store, _tmp) = setup_store();

        // Create a source -> intermediate -> sink chain
        insert_node(&store, "app/routes.py::get_request", "Function", r#"{"params": ["request"]}"#);
        insert_node(&store, "app/service.py::process", "Function", "{}");
        insert_node(&store, "app/db.py::execute_query", "Function", r#"{"calls": ["cursor.execute"]}"#);

        insert_edge(&store, "app/routes.py::get_request", "app/service.py::process");
        insert_edge(&store, "app/service.py::process", "app/db.py::execute_query");

        let paths = propagate_taint(&store).unwrap();
        assert!(!paths.is_empty());

        let path = &paths[0];
        assert_eq!(path.source_fqn, "app/routes.py::get_request");
        assert_eq!(path.sink_fqn, "app/db.py::execute_query");
        assert_eq!(path.cwe_id, Some("CWE-89".to_string()));

        // Verify path_json contains the intermediate node
        let path_nodes: Vec<String> = serde_json::from_str(&path.path_json).unwrap();
        assert!(path_nodes.contains(&"app/service.py::process".to_string()));
    }

    #[test]
    fn test_multi_hop_path() {
        let (store, _tmp) = setup_store();

        // Create a longer chain: source -> A -> B -> C -> sink
        insert_node(&store, "src/handler.py::request_handler", "Function", r#"{"params": ["request"]}"#);
        insert_node(&store, "src/a.py::step_a", "Function", "{}");
        insert_node(&store, "src/b.py::step_b", "Function", "{}");
        insert_node(&store, "src/c.py::step_c", "Function", "{}");
        insert_node(&store, "src/cmd.py::run_command", "Function", r#"{"calls": ["subprocess.run"]}"#);

        insert_edge(&store, "src/handler.py::request_handler", "src/a.py::step_a");
        insert_edge(&store, "src/a.py::step_a", "src/b.py::step_b");
        insert_edge(&store, "src/b.py::step_b", "src/c.py::step_c");
        insert_edge(&store, "src/c.py::step_c", "src/cmd.py::run_command");

        let paths = propagate_taint(&store).unwrap();
        assert!(!paths.is_empty());

        let path = &paths[0];
        assert_eq!(path.source_fqn, "src/handler.py::request_handler");
        assert_eq!(path.sink_fqn, "src/cmd.py::run_command");
        assert_eq!(path.cwe_id, Some("CWE-78".to_string()));

        // Verify multi-hop path
        let path_nodes: Vec<String> = serde_json::from_str(&path.path_json).unwrap();
        assert_eq!(path_nodes.len(), 5); // source + 3 intermediates + sink
    }

    #[test]
    fn test_no_path_means_no_finding() {
        let (store, _tmp) = setup_store();

        // Source and sink exist but are not connected
        insert_node(&store, "app/routes.py::get_request", "Function", r#"{"params": ["request"]}"#);
        insert_node(&store, "app/db.py::execute_query", "Function", r#"{"calls": ["cursor.execute"]}"#);
        // No edges between them

        let paths = propagate_taint(&store).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn test_detect_sources_env_var() {
        let nodes = vec![Node {
            fqn: "config.py::load_env".to_string(),
            kind: crate::store::types::NodeKind::Function,
            file: "config.py".to_string(),
            start_line: 1,
            end_line: 5,
            file_hash: "hash".to_string(),
            indexed_at: 1000,
            attributes: json!({"calls": ["os.getenv"]}),
        }];

        let sources = detect_sources(&nodes);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, TaintSourceKind::EnvVar);
    }

    #[test]
    fn test_detect_sinks_command_execution() {
        let nodes = vec![Node {
            fqn: "utils.py::run_shell".to_string(),
            kind: crate::store::types::NodeKind::Function,
            file: "utils.py".to_string(),
            start_line: 1,
            end_line: 5,
            file_hash: "hash".to_string(),
            indexed_at: 1000,
            attributes: json!({"calls": ["subprocess.run"]}),
        }];

        let sinks = detect_sinks(&nodes);
        assert_eq!(sinks.len(), 1);
        assert_eq!(sinks[0].kind, TaintSinkKind::CommandExecution);
    }

    #[test]
    fn test_multiple_sources_to_same_sink() {
        let (store, _tmp) = setup_store();

        // Two sources both connected to the same sink
        insert_node(&store, "app/routes.py::get_user_input", "Function", r#"{"params": ["request"]}"#);
        insert_node(&store, "app/config.py::load_env", "Function", r#"{"calls": ["os.getenv"]}"#);
        insert_node(&store, "app/db.py::run_query", "Function", r#"{"calls": ["cursor.execute"]}"#);

        // Both sources connect to the same sink
        insert_edge(&store, "app/routes.py::get_user_input", "app/db.py::run_query");
        insert_edge(&store, "app/config.py::load_env", "app/db.py::run_query");

        let paths = propagate_taint(&store).unwrap();
        assert_eq!(paths.len(), 2);

        // Both paths should lead to the same sink
        let sink_fqns: Vec<&str> = paths.iter().map(|p| p.sink_fqn.as_str()).collect();
        assert!(sink_fqns.iter().all(|&s| s == "app/db.py::run_query"));

        // Source FQNs should be different
        let source_fqns: Vec<&str> = paths.iter().map(|p| p.source_fqn.as_str()).collect();
        assert!(source_fqns.contains(&"app/routes.py::get_user_input"));
        assert!(source_fqns.contains(&"app/config.py::load_env"));
    }

    #[test]
    fn test_detect_sources_sinks_python() {
        let mut nodes = vec![
            Node {
                fqn: "app/views.py::handle_request".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "app/views.py".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"calls": ["request.args.get"]}),
            },
            Node {
                fqn: "app/db.py::execute_sql".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "app/db.py".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"calls": ["cursor.execute"]}),
            },
            Node {
                fqn: "app/utils.py::helper".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "app/utils.py".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({}),
            },
        ];

        detect_sources_sinks(&mut nodes, "python");

        // First node should be marked as source
        assert_eq!(
            nodes[0].attributes.get("taint_source").and_then(|v| v.as_str()),
            Some("HttpInput")
        );
        // Second node should be marked as sink
        assert_eq!(
            nodes[1].attributes.get("taint_sink").and_then(|v| v.as_str()),
            Some("SqlQuery")
        );
        // Third node should have no taint markers
        assert!(nodes[2].attributes.get("taint_source").is_none());
        assert!(nodes[2].attributes.get("taint_sink").is_none());
    }

    #[test]
    fn test_detect_sources_sinks_typescript() {
        let mut nodes = vec![
            Node {
                fqn: "src/controller.ts::getUser".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "src/controller.ts".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"params": ["req.body"]}),
            },
            Node {
                fqn: "src/db.ts::runQuery".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "src/db.ts".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"calls": ["db.query("]}),
            },
        ];

        detect_sources_sinks(&mut nodes, "typescript");

        assert_eq!(
            nodes[0].attributes.get("taint_source").and_then(|v| v.as_str()),
            Some("HttpInput")
        );
        assert_eq!(
            nodes[1].attributes.get("taint_sink").and_then(|v| v.as_str()),
            Some("SqlQuery")
        );
    }

    #[test]
    fn test_detect_sources_sinks_go() {
        let mut nodes = vec![
            Node {
                fqn: "handlers/user.go::HandleUser".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "handlers/user.go".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"calls": ["r.FormValue"]}),
            },
            Node {
                fqn: "db/queries.go::RunQuery".to_string(),
                kind: crate::store::types::NodeKind::Function,
                file: "db/queries.go".to_string(),
                start_line: 1,
                end_line: 10,
                file_hash: "hash".to_string(),
                indexed_at: 1000,
                attributes: json!({"calls": ["db.Query"]}),
            },
        ];

        detect_sources_sinks(&mut nodes, "go");

        assert_eq!(
            nodes[0].attributes.get("taint_source").and_then(|v| v.as_str()),
            Some("HttpInput")
        );
        assert_eq!(
            nodes[1].attributes.get("taint_sink").and_then(|v| v.as_str()),
            Some("SqlQuery")
        );
    }
}
