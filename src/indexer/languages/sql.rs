//! SQL language extractor (regex-based).
//!
//! Extracts SQL DDL constructs from SQL source code using regex patterns.
//! Handles: CREATE TABLE, CREATE FUNCTION, CREATE OR REPLACE FUNCTION,
//! CREATE PROCEDURE, CREATE OR REPLACE PROCEDURE, CREATE VIEW,
//! CREATE MATERIALIZED VIEW, CREATE TRIGGER, CREATE SCHEMA, and CTEs (WITH name AS).
//!
//! tree-sitter-sql has uncertain compatibility with tree-sitter 0.25.x,
//! so this extractor remains regex-based with enhanced pattern coverage.

use regex::Regex;
use serde_json::json;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Estimate the end line of a SQL block starting at `start_byte` in `source`.
///
/// SQL blocks are delimited by `BEGIN`/`END` keyword pairs (case-insensitive)
/// rather than braces.  We count nesting depth: each `BEGIN` increments depth,
/// each `END` decrements it.  When depth returns to zero after the first `BEGIN`
/// we return the current line number.
///
/// If no `BEGIN` is found (e.g. a simple `CREATE VIEW … AS SELECT …;`) we fall
/// back to scanning for the terminating semicolon and return that line.
///
/// Falls back to `start_line + fallback_offset` when neither is found.
fn estimate_end_line_sql(
    source: &str,
    start_byte: usize,
    start_line: u32,
    fallback_offset: u32,
) -> u32 {
    let slice = &source[start_byte..];
    let mut depth: i32 = 0;
    let mut found_begin = false;
    let mut line = start_line;

    // Tokenise by whitespace/newlines to find BEGIN/END keywords.
    // We iterate character-by-character to track line numbers accurately.
    let bytes = slice.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }

        // Try to match a word boundary keyword (BEGIN or END), case-insensitive.
        // A keyword must be preceded by a non-word character (or start of slice)
        // and followed by a non-word character.
        let prev_is_word = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if !prev_is_word {
            if i + 5 <= len {
                let word5 = &slice[i..i + 5];
                if word5.eq_ignore_ascii_case("BEGIN") {
                    let next_is_word = i + 5 < len
                        && (bytes[i + 5].is_ascii_alphanumeric() || bytes[i + 5] == b'_');
                    if !next_is_word {
                        depth += 1;
                        found_begin = true;
                        i += 5;
                        continue;
                    }
                }
            }
            if i + 3 <= len {
                let word3 = &slice[i..i + 3];
                if word3.eq_ignore_ascii_case("END") {
                    let next_is_word = i + 3 < len
                        && (bytes[i + 3].is_ascii_alphanumeric() || bytes[i + 3] == b'_');
                    if !next_is_word {
                        if found_begin {
                            depth -= 1;
                            if depth == 0 {
                                return line;
                            }
                        }
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // If no BEGIN found yet, look for a terminating semicolon.
        if !found_begin && bytes[i] == b';' {
            return line;
        }

        i += 1;
    }

    start_line + fallback_offset
}

/// Estimate cyclomatic complexity for a SQL function/procedure body (regex heuristic).
/// Counts decision keywords: IF, ELSIF/ELSEIF, CASE, WHEN, LOOP, WHILE, FOR, EXCEPTION.
fn estimate_complexity_sql(source: &str, start_byte: usize, end_byte: usize) -> u32 {
    let end = end_byte.min(source.len());
    if start_byte >= end {
        return 1;
    }
    let body = &source[start_byte..end];
    let mut complexity: u32 = 1; // base

    let decision_re =
        Regex::new(r"(?i)\b(IF|ELSIF|ELSEIF|CASE|WHEN|LOOP|WHILE|FOR|EXCEPTION)\b").unwrap();
    complexity += decision_re.find_iter(body).count() as u32;

    // Count AND/OR in WHERE clauses as additional paths
    let logical_re = Regex::new(r"(?i)\b(AND|OR)\b").unwrap();
    complexity += logical_re.find_iter(body).count() as u32 / 2; // discount: not all are branching

    complexity
}

/// Extract nodes and edges from SQL source code using regex.
pub fn extract_sql(file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // -----------------------------------------------------------------------
    // 1. CREATE SCHEMA [IF NOT EXISTS] name  →  NodeKind::Module
    // -----------------------------------------------------------------------
    let schema_re = Regex::new(
        r"(?im)^\s*CREATE\s+SCHEMA\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:AUTHORIZATION\s+\w+\s+)?(\w+)",
    )
    .unwrap();
    for caps in schema_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Module,
            file: file.to_string(),
            start_line: line,
            end_line: line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "schema"}),
        });
    }

    // -----------------------------------------------------------------------
    // 2. CREATE TABLE [IF NOT EXISTS] [schema.]name  →  NodeKind::Type
    // -----------------------------------------------------------------------
    let table_re = Regex::new(
        r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:TEMP(?:ORARY)?\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:\w+\.)?(\w+)"
    ).unwrap();
    for caps in table_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line_sql(source, match_start, line, 10);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Type,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "table"}),
        });
    }

    // -----------------------------------------------------------------------
    // 3. CREATE [OR REPLACE] FUNCTION [schema.]name  →  NodeKind::Function
    // -----------------------------------------------------------------------
    let func_re =
        Regex::new(r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+(?:\w+\.)?(\w+)").unwrap();
    for caps in func_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line_sql(source, match_start, line, 10);
        let end_byte = source
            .lines()
            .take(end_line as usize)
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let complexity = estimate_complexity_sql(source, match_start, end_byte);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "function", "complexity": complexity}),
        });
    }

    // -----------------------------------------------------------------------
    // 4. CREATE [OR REPLACE] [MATERIALIZED] VIEW [IF NOT EXISTS] [schema.]name
    //    →  NodeKind::Class  (materialized=true when MATERIALIZED keyword present)
    // -----------------------------------------------------------------------
    let view_re = Regex::new(
        r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(MATERIALIZED\s+)?VIEW\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:\w+\.)?(\w+)"
    ).unwrap();
    for caps in view_re.captures_iter(source) {
        let is_materialized = caps.get(1).is_some();
        let name = caps.get(2).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line_sql(source, match_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "view", "materialized": is_materialized}),
        });
    }

    // -----------------------------------------------------------------------
    // 5. CREATE [OR REPLACE] PROCEDURE [schema.]name  →  NodeKind::Function
    // -----------------------------------------------------------------------
    let proc_re =
        Regex::new(r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?PROCEDURE\s+(?:\w+\.)?(\w+)").unwrap();
    for caps in proc_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line_sql(source, match_start, line, 10);
        let end_byte = source
            .lines()
            .take(end_line as usize)
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let complexity = estimate_complexity_sql(source, match_start, end_byte);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "procedure", "complexity": complexity}),
        });
    }

    // -----------------------------------------------------------------------
    // 6. CREATE [OR REPLACE] [CONSTRAINT] TRIGGER name  →  NodeKind::Function
    //    with trigger=true attribute
    // -----------------------------------------------------------------------
    let trigger_re = Regex::new(
        r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:CONSTRAINT\s+)?TRIGGER\s+(?:\w+\.)?(\w+)",
    )
    .unwrap();
    for caps in trigger_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        let end_line = estimate_end_line_sql(source, match_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "trigger", "trigger": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 7. CTEs: WITH cte_name AS (  →  NodeKind::Function with cte=true
    //    Only matches top-level CTEs (at the start of a statement, not inside
    //    a subquery).  We match `WITH name AS (` where the WITH is at the
    //    beginning of a line (possibly preceded by whitespace).
    // -----------------------------------------------------------------------
    let cte_re = Regex::new(r"(?im)^\s*WITH\s+(\w+)\s+AS\s*\(").unwrap();
    for caps in cte_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let match_start = caps.get(0).unwrap().start();
        let line = source[..match_start].matches('\n').count() as u32 + 1;
        // CTEs end at the closing paren; use a small fixed offset as fallback.
        let end_line = estimate_end_line_sql(source, match_start, line, 5);
        nodes.push(Node {
            fqn: format!("{}::{}", file, name),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line: line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"sql_type": "cte", "cte": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 8. Foreign key references as DataFlow edges: REFERENCES [schema.]table
    // -----------------------------------------------------------------------
    let ref_re = Regex::new(r"(?im)REFERENCES\s+(?:\w+\.)?(\w+)").unwrap();
    for caps in ref_re.captures_iter(source) {
        let target_table = caps.get(1).unwrap().as_str();
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: format!("{}::{}", file, target_table),
            kind: EdgeKind::DataFlow,
            confidence: 0.9,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({"reference": true}),
        });
    }

    // -----------------------------------------------------------------------
    // 9. Simple function/procedure call extraction within bodies
    //    Matches: function_name(...) patterns inside procedure/function bodies
    // -----------------------------------------------------------------------
    let sql_call_re = Regex::new(r"(?im)\b([a-zA-Z_]\w*)\s*\(").unwrap();
    let sql_keywords: std::collections::HashSet<&str> = [
        "select",
        "from",
        "where",
        "insert",
        "update",
        "delete",
        "create",
        "alter",
        "drop",
        "table",
        "index",
        "view",
        "function",
        "procedure",
        "trigger",
        "schema",
        "if",
        "else",
        "elsif",
        "then",
        "end",
        "begin",
        "declare",
        "set",
        "into",
        "values",
        "returns",
        "return",
        "as",
        "is",
        "or",
        "and",
        "not",
        "in",
        "exists",
        "between",
        "like",
        "case",
        "when",
        "join",
        "on",
        "left",
        "right",
        "inner",
        "outer",
        "group",
        "order",
        "having",
        "limit",
        "offset",
        "union",
        "except",
        "intersect",
        "with",
        "recursive",
        "replace",
        "grant",
        "revoke",
        "commit",
        "rollback",
        "savepoint",
        "constraint",
        "primary",
        "foreign",
        "key",
        "references",
        "check",
        "unique",
        "default",
        "null",
        "cascade",
        "count",
        "sum",
        "avg",
        "min",
        "max",
        "coalesce",
        "cast",
        "convert",
        "trim",
        "substring",
        "upper",
        "lower",
        "length",
        "concat",
    ]
    .iter()
    .copied()
    .collect();

    // Collect declared function/procedure names
    let _declared_fns: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .filter_map(|n| n.fqn.rsplit("::").next().map(|s| s.to_lowercase()))
        .collect();

    // Find function/procedure ranges for enclosing context
    let fn_ranges: Vec<(&str, u32, u32)> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .map(|n| (n.fqn.as_str(), n.start_line, n.end_line))
        .collect();

    for caps in sql_call_re.captures_iter(source) {
        let name = caps.get(1).unwrap().as_str();
        let name_lower = name.to_lowercase();
        if sql_keywords.contains(name_lower.as_str()) {
            continue;
        }
        // Skip common SQL built-in functions
        if sql_keywords.contains(name_lower.as_str()) {
            continue;
        }

        let match_start = caps.get(0).unwrap().start();
        let call_line = source[..match_start].matches('\n').count() as u32 + 1;

        // Find enclosing function/procedure
        let source_fqn = fn_ranges
            .iter()
            .rev()
            .find(|(_, start, end)| call_line >= *start && call_line <= *end)
            .map(|(fqn, _, _)| fqn.to_string())
            .unwrap_or_else(|| file.to_string());

        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: name.to_string(),
            kind: EdgeKind::Calls,
            confidence: 0.0,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({"call_type": "function"}),
        });
    }

    ExtractionResult { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Baseline test (updated assertions to match new NodeKind mapping)
    // ------------------------------------------------------------------

    #[test]
    fn test_sql_extract_tables_functions_views_procedures() {
        let source = r#"
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL,
    total DECIMAL(10, 2),
    status VARCHAR(50) DEFAULT 'pending',
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE OR REPLACE FUNCTION get_user_orders(p_user_id INTEGER)
RETURNS TABLE(order_id INTEGER, total DECIMAL) AS $$
BEGIN
    RETURN QUERY SELECT id, total FROM orders WHERE user_id = p_user_id;
END;
$$ LANGUAGE plpgsql;

CREATE VIEW active_users AS
SELECT u.id, u.name, u.email
FROM users u
WHERE u.id IN (SELECT DISTINCT user_id FROM orders WHERE status = 'completed');

CREATE PROCEDURE archive_old_orders(p_days INTEGER)
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM orders WHERE created_at < NOW() - INTERVAL '1 day' * p_days;
END;
$$;

CREATE TRIGGER update_timestamp
BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION update_modified_column();
"#;
        let result = extract_sql("migrations/001_schema.sql", source);

        // Tables → NodeKind::Type
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "migrations/001_schema.sql::users" && n.kind == NodeKind::Type)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "migrations/001_schema.sql::orders" && n.kind == NodeKind::Type)
        );

        // Function → NodeKind::Function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "migrations/001_schema.sql::get_user_orders"
                    && n.kind == NodeKind::Function)
        );

        // View → NodeKind::Class (changed from Type)
        assert!(result.nodes.iter().any(
            |n| n.fqn == "migrations/001_schema.sql::active_users" && n.kind == NodeKind::Class
        ));

        // Procedure → NodeKind::Function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "migrations/001_schema.sql::archive_old_orders"
                    && n.kind == NodeKind::Function)
        );

        // Trigger → NodeKind::Function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "migrations/001_schema.sql::update_timestamp"
                    && n.kind == NodeKind::Function)
        );

        // Foreign key reference edge
        let refs: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .collect();
        assert!(
            refs.iter()
                .any(|e| e.target_fqn == "migrations/001_schema.sql::users")
        );
    }

    #[test]
    fn test_sql_empty_file() {
        let result = extract_sql("empty.sql", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // ------------------------------------------------------------------
    // New tests for enhanced constructs
    // ------------------------------------------------------------------

    #[test]
    fn test_sql_schema() {
        let source = r#"
CREATE SCHEMA myapp;
CREATE SCHEMA IF NOT EXISTS reporting;
"#;
        let result = extract_sql("db/schemas.sql", source);

        let myapp = result
            .nodes
            .iter()
            .find(|n| n.fqn == "db/schemas.sql::myapp");
        assert!(myapp.is_some(), "myapp schema not found");
        assert_eq!(myapp.unwrap().kind, NodeKind::Module);
        assert_eq!(myapp.unwrap().attributes["sql_type"], "schema");

        let reporting = result
            .nodes
            .iter()
            .find(|n| n.fqn == "db/schemas.sql::reporting");
        assert!(reporting.is_some(), "reporting schema not found");
        assert_eq!(reporting.unwrap().kind, NodeKind::Module);
    }

    #[test]
    fn test_sql_view_is_class() {
        let source = r#"
CREATE VIEW active_users AS
SELECT id, name FROM users WHERE active = true;

CREATE OR REPLACE VIEW user_summary AS
SELECT id, name, email FROM users;
"#;
        let result = extract_sql("views.sql", source);

        let active = result
            .nodes
            .iter()
            .find(|n| n.fqn == "views.sql::active_users");
        assert!(active.is_some(), "active_users view not found");
        assert_eq!(active.unwrap().kind, NodeKind::Class);
        assert_eq!(active.unwrap().attributes["sql_type"], "view");
        assert_eq!(active.unwrap().attributes["materialized"], false);

        let summary = result
            .nodes
            .iter()
            .find(|n| n.fqn == "views.sql::user_summary");
        assert!(summary.is_some(), "user_summary view not found");
        assert_eq!(summary.unwrap().kind, NodeKind::Class);
    }

    #[test]
    fn test_sql_materialized_view() {
        let source = r#"
CREATE MATERIALIZED VIEW monthly_sales AS
SELECT
    date_trunc('month', created_at) AS month,
    SUM(total) AS revenue
FROM orders
GROUP BY 1;

CREATE OR REPLACE MATERIALIZED VIEW product_stats AS
SELECT product_id, COUNT(*) AS order_count FROM order_items GROUP BY 1;
"#;
        let result = extract_sql("materialized.sql", source);

        let monthly = result
            .nodes
            .iter()
            .find(|n| n.fqn == "materialized.sql::monthly_sales");
        assert!(
            monthly.is_some(),
            "monthly_sales materialized view not found"
        );
        assert_eq!(monthly.unwrap().kind, NodeKind::Class);
        assert_eq!(monthly.unwrap().attributes["materialized"], true);
        assert_eq!(monthly.unwrap().attributes["sql_type"], "view");

        let stats = result
            .nodes
            .iter()
            .find(|n| n.fqn == "materialized.sql::product_stats");
        assert!(stats.is_some(), "product_stats materialized view not found");
        assert_eq!(stats.unwrap().attributes["materialized"], true);
    }

    #[test]
    fn test_sql_stored_procedure() {
        let source = r#"
CREATE PROCEDURE transfer_funds(
    sender_id INTEGER,
    receiver_id INTEGER,
    amount DECIMAL
)
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE accounts SET balance = balance - amount WHERE id = sender_id;
    UPDATE accounts SET balance = balance + amount WHERE id = receiver_id;
    COMMIT;
END;
$$;

CREATE OR REPLACE PROCEDURE cleanup_sessions()
LANGUAGE sql AS $$
    DELETE FROM sessions WHERE expires_at < NOW();
$$;
"#;
        let result = extract_sql("procedures.sql", source);

        let transfer = result
            .nodes
            .iter()
            .find(|n| n.fqn == "procedures.sql::transfer_funds");
        assert!(transfer.is_some(), "transfer_funds procedure not found");
        assert_eq!(transfer.unwrap().kind, NodeKind::Function);
        assert_eq!(transfer.unwrap().attributes["sql_type"], "procedure");

        let cleanup = result
            .nodes
            .iter()
            .find(|n| n.fqn == "procedures.sql::cleanup_sessions");
        assert!(cleanup.is_some(), "cleanup_sessions procedure not found");
        assert_eq!(cleanup.unwrap().kind, NodeKind::Function);
    }

    #[test]
    fn test_sql_trigger_with_attribute() {
        let source = r#"
CREATE TRIGGER update_timestamp
BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION update_modified_column();

CREATE OR REPLACE TRIGGER audit_log
AFTER INSERT OR UPDATE OR DELETE ON orders
FOR EACH ROW EXECUTE PROCEDURE log_changes();

CREATE CONSTRAINT TRIGGER check_balance
AFTER INSERT ON transactions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_balance();
"#;
        let result = extract_sql("triggers.sql", source);

        let ts_trigger = result
            .nodes
            .iter()
            .find(|n| n.fqn == "triggers.sql::update_timestamp");
        assert!(ts_trigger.is_some(), "update_timestamp trigger not found");
        assert_eq!(ts_trigger.unwrap().kind, NodeKind::Function);
        assert_eq!(ts_trigger.unwrap().attributes["sql_type"], "trigger");
        assert_eq!(ts_trigger.unwrap().attributes["trigger"], true);

        let audit = result
            .nodes
            .iter()
            .find(|n| n.fqn == "triggers.sql::audit_log");
        assert!(audit.is_some(), "audit_log trigger not found");
        assert_eq!(audit.unwrap().attributes["trigger"], true);

        let constraint_trigger = result
            .nodes
            .iter()
            .find(|n| n.fqn == "triggers.sql::check_balance");
        assert!(
            constraint_trigger.is_some(),
            "check_balance constraint trigger not found"
        );
        assert_eq!(constraint_trigger.unwrap().kind, NodeKind::Function);
        assert_eq!(constraint_trigger.unwrap().attributes["trigger"], true);
    }

    #[test]
    fn test_sql_cte() {
        let source = r#"
WITH active_orders AS (
    SELECT * FROM orders WHERE status = 'active'
),
revenue_by_user AS (
    SELECT user_id, SUM(total) AS total_revenue
    FROM active_orders
    GROUP BY user_id
)
SELECT u.name, r.total_revenue
FROM users u
JOIN revenue_by_user r ON u.id = r.user_id;
"#;
        let result = extract_sql("queries/report.sql", source);

        let active_orders = result
            .nodes
            .iter()
            .find(|n| n.fqn == "queries/report.sql::active_orders");
        assert!(active_orders.is_some(), "active_orders CTE not found");
        assert_eq!(active_orders.unwrap().kind, NodeKind::Function);
        assert_eq!(active_orders.unwrap().attributes["sql_type"], "cte");
        assert_eq!(active_orders.unwrap().attributes["cte"], true);

        // Note: only the first WITH is at line start; subsequent CTEs in the
        // same WITH clause are comma-separated and not at line start, so only
        // active_orders is captured by the top-level CTE pattern.
        // revenue_by_user starts with a comma, not WITH, so it is not matched.
    }

    #[test]
    fn test_sql_cte_multiple_statements() {
        let source = r#"
WITH top_customers AS (
    SELECT user_id, COUNT(*) AS order_count
    FROM orders
    GROUP BY user_id
    HAVING COUNT(*) > 10
)
SELECT * FROM top_customers;

WITH recent_activity AS (
    SELECT * FROM events WHERE created_at > NOW() - INTERVAL '7 days'
)
SELECT COUNT(*) FROM recent_activity;
"#;
        let result = extract_sql("queries/analytics.sql", source);

        // Both top-level CTEs should be captured (each starts a new WITH statement)
        assert!(result.nodes.iter().any(
            |n| n.fqn == "queries/analytics.sql::top_customers" && n.attributes["cte"] == true
        ));
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "queries/analytics.sql::recent_activity"
                    && n.attributes["cte"] == true)
        );
    }

    #[test]
    fn test_sql_end_line_begin_end() {
        let source = r#"
CREATE OR REPLACE FUNCTION calculate_tax(amount DECIMAL)
RETURNS DECIMAL AS $$
BEGIN
    IF amount > 1000 THEN
        RETURN amount * 0.2;
    ELSE
        RETURN amount * 0.1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql;
"#;
        let result = extract_sql("functions.sql", source);

        let func = result
            .nodes
            .iter()
            .find(|n| n.fqn == "functions.sql::calculate_tax");
        assert!(func.is_some(), "calculate_tax function not found");
        let f = func.unwrap();
        // The function spans multiple lines; end_line should be > start_line
        assert!(
            f.end_line > f.start_line,
            "end_line ({}) should be greater than start_line ({}) for multi-line function",
            f.end_line,
            f.start_line
        );
    }

    #[test]
    fn test_sql_schema_qualified_names() {
        let source = r#"
CREATE TABLE public.users (
    id SERIAL PRIMARY KEY
);

CREATE FUNCTION app.get_user(p_id INTEGER)
RETURNS VOID AS $$ BEGIN END; $$ LANGUAGE plpgsql;

CREATE VIEW reporting.user_stats AS
SELECT id, name FROM public.users;
"#;
        let result = extract_sql("schema_qualified.sql", source);

        // Schema-qualified names: only the unqualified name is captured
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "schema_qualified.sql::users" && n.kind == NodeKind::Type)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "schema_qualified.sql::get_user" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "schema_qualified.sql::user_stats" && n.kind == NodeKind::Class)
        );
    }

    #[test]
    fn test_sql_mixed_ddl() {
        let source = r#"
CREATE SCHEMA analytics;

CREATE TABLE analytics.events (
    id BIGSERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    event_type VARCHAR(100),
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE OR REPLACE FUNCTION analytics.track_event(
    p_user_id INTEGER,
    p_event_type VARCHAR
) RETURNS VOID AS $$
BEGIN
    INSERT INTO analytics.events (user_id, event_type) VALUES (p_user_id, p_event_type);
END;
$$ LANGUAGE plpgsql;

CREATE MATERIALIZED VIEW analytics.daily_events AS
SELECT
    DATE(created_at) AS day,
    event_type,
    COUNT(*) AS event_count
FROM analytics.events
GROUP BY 1, 2;

CREATE TRIGGER events_audit
AFTER INSERT ON analytics.events
FOR EACH ROW EXECUTE FUNCTION log_event_insert();

WITH event_summary AS (
    SELECT user_id, COUNT(*) AS total_events FROM analytics.events GROUP BY 1
)
SELECT * FROM event_summary WHERE total_events > 100;
"#;
        let result = extract_sql("analytics/setup.sql", source);

        // Schema
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "analytics/setup.sql::analytics" && n.kind == NodeKind::Module)
        );

        // Table
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "analytics/setup.sql::events" && n.kind == NodeKind::Type)
        );

        // Function
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "analytics/setup.sql::track_event" && n.kind == NodeKind::Function
            )
        );

        // Materialized view
        let mv = result
            .nodes
            .iter()
            .find(|n| n.fqn == "analytics/setup.sql::daily_events");
        assert!(mv.is_some(), "daily_events materialized view not found");
        assert_eq!(mv.unwrap().kind, NodeKind::Class);
        assert_eq!(mv.unwrap().attributes["materialized"], true);

        // Trigger
        let trigger = result
            .nodes
            .iter()
            .find(|n| n.fqn == "analytics/setup.sql::events_audit");
        assert!(trigger.is_some(), "events_audit trigger not found");
        assert_eq!(trigger.unwrap().kind, NodeKind::Function);
        assert_eq!(trigger.unwrap().attributes["trigger"], true);

        // CTE
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "analytics/setup.sql::event_summary"
                    && n.attributes["cte"] == true)
        );

        // Foreign key reference edge
        let refs: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .collect();
        assert!(
            refs.iter()
                .any(|e| e.target_fqn == "analytics/setup.sql::users")
        );
    }
}
