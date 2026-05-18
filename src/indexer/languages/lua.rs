//! Lua AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (functions, local functions, tables-as-classes)
//! and edges (require imports, intra-file calls) from a tree-sitter Lua parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Lua file.
///
/// Handles:
/// - Global function definitions (`function name(...) ... end`)
/// - Local function definitions (`local function name(...) ... end`)
/// - Method-style functions (`function Class:method(...) ... end` or `function Class.method(...)`)
/// - Tables-as-classes (`ClassName = {}` followed by method assignments)
/// - Require imports (`require("module")`, `require 'module'`)
/// - Intra-file function calls (simple calls matching defined functions)
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)
    let mut table_classes: Vec<String> = Vec::new(); // table names that act as classes

    collect_definitions(
        root,
        file,
        source_bytes,
        &mut nodes,
        &mut defined_fqns,
        &mut table_classes,
    );

    // Compute cyclomatic complexity for Function nodes
    compute_node_complexities(&mut nodes, root, source_bytes);

    // Second pass: collect imports and calls
    collect_imports(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Deprecated wrapper for backward compatibility with the regex-based pipeline.
/// New code should use `extract()` with a pre-parsed tree.
#[deprecated(note = "Use extract() with a tree-sitter Tree instead")]
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_lua::LANGUAGE.into())
        .expect("Lua grammar should load");
    match parser.parse(source, None) {
        Some(tree) => extract(&tree, file, source),
        None => ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        },
    }
}

/// Compute cyclomatic complexity for all Function nodes.
fn compute_node_complexities(nodes: &mut [Node], root: tree_sitter::Node, source: &[u8]) {
    for node in nodes.iter_mut() {
        if node.kind == NodeKind::Function {
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_declaration")
                    .or_else(|| {
                        find_ast_node_at_line(root, node.start_line, "local_function_declaration")
                    })
                    .or_else(|| {
                        find_ast_node_at_line(root, node.start_line, "function_definition_statement")
                    })
                    .or_else(|| {
                        find_ast_node_at_line(root, node.start_line, "function_statement")
                    })
            {
                let c = complexity::compute_full_complexity(ast_node, source, "lua");
                if let Some(attrs) = node.attributes.as_object_mut() {
                    attrs.insert("complexity".to_string(), serde_json::json!(c));
                }
            }
        }
    }
}

/// Find an AST node of a given kind at a specific start line (1-indexed).
fn find_ast_node_at_line<'a>(
    node: tree_sitter::Node<'a>,
    target_line: u32,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let node_start_line = node.start_position().row as u32 + 1;
    if node.kind() == kind && node_start_line == target_line {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_ast_node_at_line(child, target_line, kind) {
            return Some(found);
        }
    }
    None
}

/// Recursively collect function definitions and table-as-class declarations from the AST.
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    table_classes: &mut Vec<String>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // Function declarations (both global and local in tree-sitter-lua)
            // tree-sitter-lua uses `function_declaration` for both, with a `local` child token
            "function_declaration" | "function_definition_statement" | "function_statement" => {
                extract_function(child, file, source, nodes, defined_fqns, table_classes);
            }
            // Some grammars may use a separate node kind for local functions
            "local_function_declaration" | "local_function_declaration_statement" => {
                extract_local_function(child, file, source, nodes, defined_fqns);
            }
            // Variable declarations: check for table-as-class patterns and local function assignments
            "variable_declaration" | "local_variable_declaration" => {
                extract_table_or_local_assignment(
                    child,
                    file,
                    source,
                    nodes,
                    defined_fqns,
                    table_classes,
                );
            }
            // Assignment statements: check for table-as-class and function assignments
            "assignment_statement" | "assignment" => {
                extract_assignment(child, file, source, nodes, defined_fqns, table_classes);
            }
            _ => {
                // Recurse into compound statements
                if child.child_count() > 0 {
                    collect_definitions(child, file, source, nodes, defined_fqns, table_classes);
                }
            }
        }
    }
}

/// Extract a global function definition.
///
/// Handles:
/// - `function name(...) ... end`
/// - `local function name(...) ... end` (detected via `local` child token)
/// - `function Module.name(...) ... end`
/// - `function Module:name(...) ... end`
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    table_classes: &mut Vec<String>,
) {
    let name = get_function_name(node, source);
    if name.is_empty() {
        return;
    }

    // Check if this is a local function (has a `local` child token)
    let is_local = has_local_keyword(node);

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Check if this is a method on a table (e.g., `function MyClass:method(...)` or `function MyClass.method(...)`)
    if name.contains(':') || name.contains('.') {
        let parts: Vec<&str> = name.splitn(2, |c| c == ':' || c == '.').collect();
        if parts.len() == 2 {
            let table_name = parts[0];
            let method_name = parts[1];

            // Register the table as a class if not already
            if !table_classes.contains(&table_name.to_string()) {
                table_classes.push(table_name.to_string());
                // Create a Class node for the table if we haven't already
                let class_fqn = format!("{file}::{table_name}");
                if !nodes.iter().any(|n| n.fqn == class_fqn) {
                    nodes.push(Node {
                        fqn: class_fqn.clone(),
                        kind: NodeKind::Class,
                        file: file.to_string(),
                        start_line,
                        end_line: start_line, // Will be updated as methods are found
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({"table_class": true}),
                    });
                    defined_fqns.push((table_name.to_string(), class_fqn));
                }
            }

            // Create the method node
            let fqn = format!("{file}::{table_name}::{method_name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                let is_method = name.contains(':');
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Function,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: if is_method {
                        json!({"method": true})
                    } else {
                        json!({})
                    },
                });
                defined_fqns.push((method_name.to_string(), fqn));
            }

            // Update the class end_line to encompass this method
            if let Some(class_node) = nodes
                .iter_mut()
                .find(|n| n.fqn == format!("{file}::{table_name}") && n.kind == NodeKind::Class)
            {
                if end_line > class_node.end_line {
                    class_node.end_line = end_line;
                }
            }

            return;
        }
    }

    // Simple global function
    let fqn = format!("{file}::{name}");
    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Function,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: if is_local {
            json!({"local": true})
        } else {
            json!({})
        },
    });

    defined_fqns.push((name, fqn));
}

/// Extract a local function definition (`local function name(...) ... end`).
fn extract_local_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_local_function_name(node, source);
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Function,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"local": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract table-as-class or local function assignment from variable declarations.
///
/// Handles:
/// - `local ClassName = {}` (table-as-class)
/// - `local func = function(...) ... end` (local function assignment)
fn extract_table_or_local_assignment(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    table_classes: &mut Vec<String>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Check for table-as-class: `local ClassName = {}`
    // Pattern: identifier starts with uppercase and is assigned an empty or non-empty table
    if let Some((name, is_table, is_func)) = parse_local_assignment(node, source) {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        if is_table && name.chars().next().map_or(false, |c| c.is_uppercase()) {
            // Table-as-class pattern
            let fqn = format!("{file}::{name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                table_classes.push(name.clone());
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"table_class": true}),
                });
                defined_fqns.push((name, fqn));
            }
        } else if is_func {
            // Local function assignment: `local func = function(...) ... end`
            let fqn = format!("{file}::{name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Function,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"local": true}),
                });
                defined_fqns.push((name, fqn));
            }
        } else if is_table && !text.contains("require") {
            // Non-uppercase table: still track it but don't create a Class node
            // unless methods are later assigned to it
        }
    }
}

/// Extract assignments that define table-as-class or method assignments.
///
/// Handles:
/// - `ClassName = {}` (global table-as-class)
/// - `ClassName.method = function(...) ... end` (method assignment)
fn extract_assignment(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    table_classes: &mut Vec<String>,
) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Check for table-as-class: `ClassName = {}`
    if let Some((name, rhs_is_table, rhs_is_func)) = parse_assignment_statement(node, source) {
        if name.contains('.') || name.contains(':') {
            // Method assignment: `Table.method = function(...) ... end`
            let parts: Vec<&str> = name.splitn(2, |c| c == '.' || c == ':').collect();
            if parts.len() == 2 && rhs_is_func {
                let table_name = parts[0];
                let method_name = parts[1];

                // Register the table as a class if not already
                if !table_classes.contains(&table_name.to_string()) {
                    table_classes.push(table_name.to_string());
                    let class_fqn = format!("{file}::{table_name}");
                    if !nodes.iter().any(|n| n.fqn == class_fqn) {
                        nodes.push(Node {
                            fqn: class_fqn.clone(),
                            kind: NodeKind::Class,
                            file: file.to_string(),
                            start_line,
                            end_line: start_line,
                            file_hash: String::new(),
                            indexed_at: 0,
                            attributes: json!({"table_class": true}),
                        });
                        defined_fqns.push((table_name.to_string(), class_fqn));
                    }
                }

                // Create the method node
                let fqn = format!("{file}::{table_name}::{method_name}");
                if !nodes.iter().any(|n| n.fqn == fqn) {
                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Function,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({"method": true}),
                    });
                    defined_fqns.push((method_name.to_string(), fqn));
                }

                // Update class end_line
                if let Some(class_node) = nodes.iter_mut().find(|n| {
                    n.fqn == format!("{file}::{table_name}") && n.kind == NodeKind::Class
                }) {
                    if end_line > class_node.end_line {
                        class_node.end_line = end_line;
                    }
                }
            }
        } else if rhs_is_table && name.chars().next().map_or(false, |c| c.is_uppercase()) {
            // Global table-as-class: `ClassName = {}`
            let fqn = format!("{file}::{name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                table_classes.push(name.clone());
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"table_class": true}),
                });
                defined_fqns.push((name, fqn));
            }
        }
    }
}

/// Collect require imports from the AST.
///
/// Handles:
/// - `require("module")`
/// - `require 'module'`
/// - `local x = require("module")`
/// - `local x = require 'module'`
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_call" | "call_expression" => {
                if let Some(target) = extract_require_target(child, source) {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: target,
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
            "variable_declaration" | "local_variable_declaration" | "assignment_statement"
            | "assignment" => {
                // Check for `local x = require("module")` patterns
                collect_require_from_declaration(child, file, source, edges);
            }
            _ => {}
        }

        // Recurse into children (but not into function bodies for top-level imports)
        if child.child_count() > 0 {
            collect_imports(child, file, source, edges);
        }
    }
}

/// Check if a function_call node is a `require(...)` call and extract the module name.
fn extract_require_target(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Quick check: does this look like a require call?
    if !text.starts_with("require") {
        return None;
    }

    // Extract the string argument from require("module") or require 'module'
    extract_string_from_text(&text)
}

/// Extract require calls from variable declarations.
/// Handles: `local json = require("cjson")`
fn collect_require_from_declaration(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Look for require in the text
    if !text.contains("require") {
        return;
    }

    // Find all require("...") or require '...' patterns in the text
    let mut search_from = 0;
    while let Some(req_pos) = text[search_from..].find("require") {
        let abs_pos = search_from + req_pos;
        let after_require = &text[abs_pos + 7..]; // skip "require"

        if let Some(target) = extract_string_from_text(&format!("require{after_require}")) {
            // Avoid duplicate imports
            if !edges.iter().any(|e| {
                e.kind == EdgeKind::Imports
                    && e.source_fqn == file
                    && e.target_fqn == target
            }) {
                edges.push(Edge {
                    id: None,
                    source_fqn: file.to_string(),
                    target_fqn: target,
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    attributes: json!({}),
                });
            }
        }

        search_from = abs_pos + 7;
    }
}

/// Extract a string literal value from a require-like expression.
/// Handles: `require("module")`, `require 'module'`, `require "module"`
fn extract_string_from_text(text: &str) -> Option<String> {
    // Skip "require" prefix and any whitespace/parens
    let after_require = text.strip_prefix("require")?.trim_start();

    // Handle both `require("mod")` and `require "mod"` and `require 'mod'`
    let content = if after_require.starts_with('(') {
        // require("mod") or require('mod')
        let inner = after_require.strip_prefix('(')?;
        inner.split(')').next()?
    } else {
        // require "mod" or require 'mod'
        after_require
    };

    let content = content.trim();

    // Extract the string value
    if content.starts_with('"') {
        let end = content[1..].find('"')?;
        let value = &content[1..1 + end];
        if !value.is_empty() {
            return Some(value.to_string());
        }
    } else if content.starts_with('\'') {
        let end = content[1..].find('\'')?;
        let value = &content[1..1 + end];
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    None
}

/// Collect intra-file function calls.
/// A call is a function_call node whose target matches a defined function.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_call" | "call_expression" => {
                extract_call_edge(child, file, source, defined_fqns, edges);
            }
            _ => {}
        }

        // Recurse into children
        if child.child_count() > 0 {
            collect_calls(child, file, source, defined_fqns, edges);
        }
    }
}

/// Extract a call edge from a function_call node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Skip require calls (already handled as imports)
    if text.starts_with("require") {
        return;
    }

    // Get the function name being called
    let call_name = get_call_target_name(node, source);
    if call_name.is_empty() {
        return;
    }

    // Determine the source FQN (enclosing function or file-level)
    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Check for method calls (e.g., `obj:method()` or `obj.method()`)
    if call_name.contains(':') || call_name.contains('.') {
        let parts: Vec<&str> = call_name.splitn(2, |c| c == ':' || c == '.').collect();
        if parts.len() == 2 {
            let receiver = parts[0];
            let method = parts[1];

            // Try to resolve to a defined method
            if let Some((_, target_fqn)) = defined_fqns
                .iter()
                .find(|(simple, _)| simple == method)
            {
                if source_fqn != *target_fqn {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.clone(),
                        target_fqn: target_fqn.clone(),
                        kind: EdgeKind::Calls,
                        confidence: 1.0,
                        attributes: json!({"receiver": receiver, "call_type": "method"}),
                    });
                }
            } else {
                // Emit unresolved method call edge
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.clone(),
                    target_fqn: method.to_string(),
                    kind: EdgeKind::Calls,
                    confidence: 0.0,
                    attributes: json!({"receiver": receiver, "call_type": "method"}),
                });
            }
            return;
        }
    }

    // Simple function call - try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == &call_name) {
        if source_fqn != *target_fqn {
            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn: target_fqn.clone(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: json!({}),
            });
        }
    }
}

/// Check if a function_declaration node has a `local` keyword child.
fn has_local_keyword(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "local" {
            return true;
        }
    }
    false
}

/// Get the function name from a function_declaration node.
/// Handles dotted and colon-separated names.
fn get_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).unwrap_or("").trim().to_string();
    }

    // Look for identifier, dot_index_expression, or method_index_expression children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "dot_index_expression" | "method_index_expression" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "function_name" | "function_name_field" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {}
        }
    }

    // Fallback: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if text.starts_with("function ") {
        let after_fn = text.strip_prefix("function ").unwrap_or("");
        let name: String = after_fn
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
            .collect();
        return name;
    }

    String::new()
}

/// Get the name from a local function declaration.
fn get_local_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).unwrap_or("").trim().to_string();
    }

    // Look for identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("").trim().to_string();
        }
    }

    // Fallback: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after) = text.strip_prefix("local function ") {
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return name;
    }

    String::new()
}

/// Parse a local variable declaration to determine if it's a table or function assignment.
/// Returns (name, is_table, is_function).
fn parse_local_assignment(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<(String, bool, bool)> {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Pattern: `local Name = {...}` or `local name = function(...) ... end`
    let after_local = text.strip_prefix("local ")?.trim_start();

    // Get the variable name
    let name: String = after_local
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if name.is_empty() {
        return None;
    }

    // Check what's on the right side of the assignment
    let after_name = after_local[name.len()..].trim_start();
    if !after_name.starts_with('=') {
        return None;
    }

    let rhs = after_name[1..].trim_start();

    let is_table = rhs.starts_with('{');
    let is_func = rhs.starts_with("function");

    Some((name, is_table, is_func))
}

/// Parse an assignment statement to determine if it's a table or function assignment.
/// Returns (name, is_table, is_function).
fn parse_assignment_statement(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<(String, bool, bool)> {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Get the left-hand side (variable name, possibly dotted)
    let eq_pos = text.find('=')?;

    // Make sure it's not == (comparison)
    if text.get(eq_pos + 1..eq_pos + 2) == Some("=") {
        return None;
    }

    let lhs = text[..eq_pos].trim();
    let rhs = text[eq_pos + 1..].trim_start();

    // The LHS should be a valid identifier or dotted name
    let name: String = lhs
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
        .collect();

    if name.is_empty() || name != lhs.trim() {
        return None;
    }

    let is_table = rhs.starts_with('{');
    let is_func = rhs.starts_with("function");

    Some((name.to_string(), is_table, is_func))
}

/// Get the call target name from a function_call node.
fn get_call_target_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).unwrap_or("").trim().to_string();
    }

    // Look for the function/prefix part (first child before arguments)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "dot_index_expression" | "method_index_expression" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "arguments" | "argument_list" | "string" | "table_constructor" => {
                // We've passed the function name, stop
                break;
            }
            // For prefix expressions that contain the call target
            "prefix" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                // If this child has text that looks like a function name, use it
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && !text.starts_with('(')
                    && !text.starts_with('{')
                    && !text.starts_with('"')
                    && !text.starts_with('\'')
                    && child.kind() != "comment"
                {
                    // Check if it's a simple identifier or dotted name
                    if text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
                    {
                        return text;
                    }
                }
            }
        }
    }

    // Fallback: extract from the full text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let name: String = text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
        .collect();

    // Don't return keywords
    if matches!(
        name.as_str(),
        "if" | "for" | "while" | "repeat" | "return" | "local" | "function" | "end" | "do"
            | "then" | "else" | "elseif"
    ) {
        return String::new();
    }

    name
}

/// Find the enclosing function for a given node and return its FQN.
fn find_enclosing_function_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration" | "function_definition_statement" | "function_statement" => {
                let name = get_function_name(parent, source);
                if !name.is_empty() {
                    // Handle dotted/colon names
                    if name.contains(':') || name.contains('.') {
                        let parts: Vec<&str> =
                            name.splitn(2, |c| c == ':' || c == '.').collect();
                        if parts.len() == 2 {
                            return Some(format!("{file}::{}::{}", parts[0], parts[1]));
                        }
                    }
                    return Some(format!("{file}::{name}"));
                }
            }
            "local_function_declaration" | "local_function_declaration_statement" => {
                let name = get_local_function_name(parent, source);
                if !name.is_empty() {
                    return Some(format!("{file}::{name}"));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Lua source and run the extractor.
    fn parse_lua(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_lua::LANGUAGE.into())
            .expect("Lua grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, "src/main.lua", source)
    }

    #[test]
    fn test_global_functions() {
        let source = r#"
function greet(name)
  print("Hello, " .. name)
end

function calculate(a, b)
  return a + b
end
"#;
        let result = parse_lua(source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::greet"
                && n.kind == NodeKind::Function),
            "Should find greet function"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::calculate"
                && n.kind == NodeKind::Function),
            "Should find calculate function"
        );
    }

    #[test]
    fn test_local_functions() {
        let source = r#"
local function validate(data)
  return data ~= nil
end

local function helper()
  return true
end
"#;
        let result = parse_lua(source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::validate"
                && n.kind == NodeKind::Function
                && n.attributes.get("local") == Some(&json!(true))),
            "Should find local validate function"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::helper"
                && n.kind == NodeKind::Function
                && n.attributes.get("local") == Some(&json!(true))),
            "Should find local helper function"
        );
    }

    #[test]
    fn test_table_as_class_with_methods() {
        let source = r#"
local MyClass = {}

function MyClass:new(name)
  local self = setmetatable({}, { __index = MyClass })
  self.name = name
  return self
end

function MyClass:getName()
  return self.name
end

function MyClass.staticMethod()
  return "static"
end
"#;
        let result = parse_lua(source);

        // Should find the class node
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::MyClass"
                && n.kind == NodeKind::Class),
            "Should find MyClass as a class node"
        );

        // Should find methods
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::MyClass::new"
                && n.kind == NodeKind::Function),
            "Should find MyClass:new method"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::MyClass::getName"
                && n.kind == NodeKind::Function),
            "Should find MyClass:getName method"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::MyClass::staticMethod"
                && n.kind == NodeKind::Function),
            "Should find MyClass.staticMethod"
        );
    }

    #[test]
    fn test_require_imports() {
        let source = r#"
local json = require("cjson")
local utils = require 'utils.helpers'
local http = require("socket.http")
"#;
        let result = parse_lua(source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "cjson"),
            "Should find cjson import"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "utils.helpers"),
            "Should find utils.helpers import"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "socket.http"),
            "Should find socket.http import"
        );
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
local function validate(data)
  return data ~= nil
end

function process(input)
  if validate(input) then
    return input
  end
end

function main()
  process("hello")
  validate("test")
end
"#;
        let result = parse_lua(source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // process calls validate
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.lua::process"
                && e.target_fqn == "src/main.lua::validate"),
            "process should call validate"
        );

        // main calls process
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.lua::main"
                && e.target_fqn == "src/main.lua::process"),
            "main should call process"
        );

        // main calls validate
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.lua::main"
                && e.target_fqn == "src/main.lua::validate"),
            "main should call validate"
        );
    }

    #[test]
    fn test_method_calls() {
        let source = r#"
local MyClass = {}

function MyClass:new(name)
  local self = setmetatable({}, { __index = MyClass })
  self.name = name
  return self
end

function MyClass:greet()
  print("Hello, " .. self.name)
end

function main()
  local obj = MyClass:new("World")
  obj:greet()
end
"#;
        let result = parse_lua(source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // main should have a call to MyClass:new (resolved to the method)
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.lua::main"
                && e.target_fqn == "src/main.lua::MyClass::new"),
            "main should call MyClass:new"
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_lua("");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_combined_extraction() {
        let source = r#"
local json = require("cjson")
local utils = require 'utils.helpers'

local function validate(data)
  return data ~= nil
end

function MyModule.process(input)
  local result = json.decode(input)
  return validate(result)
end

function greet(name)
  print("Hello, " .. name)
end
"#;
        let result = parse_lua(source);

        // Check requires
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "cjson"));
        assert!(imports.iter().any(|e| e.target_fqn == "utils.helpers"));

        // Check local function
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.lua::validate" && n.kind == NodeKind::Function));

        // Check global functions - MyModule.process creates a class + method
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.lua::MyModule::process" && n.kind == NodeKind::Function));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.lua::greet" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_function_assignment_method() {
        let source = r#"
local Handler = {}

Handler.process = function(self, data)
  return data
end

Handler.validate = function(input)
  return input ~= nil
end
"#;
        let result = parse_lua(source);

        // Should find Handler as a class
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::Handler"
                && n.kind == NodeKind::Class),
            "Should find Handler as a class"
        );

        // Should find methods
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::Handler::process"
                && n.kind == NodeKind::Function),
            "Should find Handler.process method"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.lua::Handler::validate"
                && n.kind == NodeKind::Function),
            "Should find Handler.validate method"
        );
    }
}
