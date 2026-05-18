//! R AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (functions, S4 classes, R6 classes)
//! and edges (library/require/source imports, intra-file calls) from a tree-sitter R parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed R file.
///
/// Handles:
/// - Function assignments (`name <- function(args) { body }`, `name = function(...)`)
/// - S4 class definitions (`setClass("ClassName", ...)`)
/// - R6 class definitions (`ClassName <- R6Class("ClassName", ...)`)
/// - Library imports (`library(pkg)`, `require(pkg)`)
/// - Source imports (`source("file.R")`)
/// - Namespace-qualified calls (`pkg::func()`, `pkg:::func()`)
/// - Standard function calls
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(root, file, source_bytes, &mut nodes, &mut defined_fqns);

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
        .set_language(&tree_sitter_r::LANGUAGE.into())
        .expect("R grammar should load");
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
            // In tree-sitter-r, function definitions appear within binary_operator
            // (left_assignment) or equals_assignment nodes. The function body is a
            // `function_definition` node.
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_definition")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "binary_operator"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "equals_assignment"))
                    .or_else(|| {
                        find_ast_node_at_line(root, node.start_line, "left_assignment")
                    })
            {
                let c = complexity::compute_full_complexity(ast_node, source, "r");
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

/// Recursively collect definitions from the AST.
///
/// In tree-sitter-r, top-level constructs include:
/// - `left_assignment` / `equals_assignment` with a `function_definition` on the RHS
///   (e.g., `name <- function(args) { body }`)
/// - `call` nodes for `setClass(...)` and `R6Class(...)`
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // `name <- function(...)` or `name <- R6Class(...)`
            "left_assignment" | "binary_operator" => {
                try_extract_assignment(child, file, source, nodes, defined_fqns);
            }
            // `name = function(...)`
            "equals_assignment" => {
                try_extract_assignment(child, file, source, nodes, defined_fqns);
            }
            // Standalone call: `setClass("Name", ...)` without assignment
            "call" => {
                try_extract_standalone_class(child, file, source, nodes, defined_fqns);
            }
            // Recurse into braced blocks and other compound nodes
            "braced_expression" | "program" => {
                collect_definitions(child, file, source, nodes, defined_fqns);
            }
            _ => {
                // Recurse into other compound nodes that might contain definitions
                if child.child_count() > 0 && !is_leaf_kind(child.kind()) {
                    collect_definitions(child, file, source, nodes, defined_fqns);
                }
            }
        }
    }
}

/// Returns true for node kinds that should not be recursed into for definitions.
fn is_leaf_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "string"
            | "string_content"
            | "integer"
            | "float"
            | "complex"
            | "comment"
            | "true"
            | "false"
            | "null"
            | "na"
            | "inf"
            | "nan"
            | "dots"
            | "dot_dot_i"
    )
}

/// Try to extract a function or class definition from an assignment node.
///
/// Patterns:
/// - `name <- function(args) { body }` → Function
/// - `name = function(args) { body }` → Function
/// - `Name <- R6Class("Name", ...)` → Class (R6)
/// - `name <- setClass("Name", ...)` → Class (S4, though usually standalone)
fn try_extract_assignment(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // In tree-sitter-r, assignment nodes have structure:
    // (left_assignment name: (identifier) value: ...)
    // or (equals_assignment name: (identifier) value: ...)
    // or (binary_operator lhs rhs) with "<-" operator

    let (name_node, value_node) = match get_assignment_parts(node, source) {
        Some(parts) => parts,
        None => return,
    };

    let name = name_node.utf8_text(source).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Check if RHS is a function_definition
    if value_node.kind() == "function_definition" {
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
                attributes: json!({}),
            });
            defined_fqns.push((name, fqn));
        }
        return;
    }

    // Check if RHS is a call to R6Class or setClass
    if value_node.kind() == "call" {
        let callee = get_call_function_name(value_node, source);
        if callee == "R6Class" {
            // R6 class: use the LHS name as the class name
            let fqn = format!("{file}::{name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"class_system": "R6"}),
                });
                defined_fqns.push((name, fqn));
            }
            return;
        }
        if callee == "setClass" {
            // S4 class via assignment: extract class name from first string argument
            let class_name = get_first_string_arg(value_node, source)
                .unwrap_or_else(|| name.clone());
            let fqn = format!("{file}::{class_name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"class_system": "S4"}),
                });
                defined_fqns.push((class_name, fqn));
            }
        }
    }
}

/// Try to extract a standalone class definition from a call node.
///
/// Pattern: `setClass("ClassName", ...)` without assignment
fn try_extract_standalone_class(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let callee = get_call_function_name(node, source);
    if callee == "setClass" {
        if let Some(class_name) = get_first_string_arg(node, source) {
            let start_line = node.start_position().row as u32 + 1;
            let end_line = node.end_position().row as u32 + 1;
            let fqn = format!("{file}::{class_name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"class_system": "S4"}),
                });
                defined_fqns.push((class_name, fqn));
            }
        }
    } else if callee == "R6Class" {
        // Standalone R6Class call (rare but possible): R6Class("Name", ...)
        if let Some(class_name) = get_first_string_arg(node, source) {
            let start_line = node.start_position().row as u32 + 1;
            let end_line = node.end_position().row as u32 + 1;
            let fqn = format!("{file}::{class_name}");
            if !nodes.iter().any(|n| n.fqn == fqn) {
                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Class,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({"class_system": "R6"}),
                });
                defined_fqns.push((class_name, fqn));
            }
        }
    }
}

/// Get the LHS (name) and RHS (value) of an assignment node.
///
/// Handles:
/// - `left_assignment`: name <- value
/// - `equals_assignment`: name = value
/// - `binary_operator` with `<-` or `=`: name <- value
fn get_assignment_parts<'a>(
    node: tree_sitter::Node<'a>,
    source: &[u8],
) -> Option<(tree_sitter::Node<'a>, tree_sitter::Node<'a>)> {
    match node.kind() {
        "left_assignment" | "equals_assignment" => {
            // These have named fields: name and value
            let name_node = node.child_by_field_name("name")
                .or_else(|| node.child(0))?;
            let value_node = node.child_by_field_name("value")
                .or_else(|| {
                    // Fallback: last child is the value
                    let count = node.child_count();
                    if count >= 2 {
                        node.child(count - 1)
                    } else {
                        None
                    }
                })?;

            // Name must be an identifier
            if name_node.kind() == "identifier" {
                Some((name_node, value_node))
            } else {
                None
            }
        }
        "binary_operator" => {
            // binary_operator: lhs op rhs
            // Check if the operator is `<-`, `<<-`, or `=` (assignment)
            // We need to find the operator child to determine if this is an assignment
            let count = node.child_count();
            if count < 3 {
                return None;
            }

            let lhs = node.child(0)?;
            let op = node.child(1)?;
            let rhs = node.child(count - 1)?;

            let op_text = op.utf8_text(source).unwrap_or("").trim();
            if op_text != "<-" && op_text != "<<-" && op_text != "=" {
                return None;
            }

            if lhs.kind() == "identifier" {
                Some((lhs, rhs))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Get the function name from a call node.
///
/// In tree-sitter-r, a call node has structure:
/// (call function: (identifier) arguments: (arguments ...))
/// or (call function: (namespace_operator ...) arguments: ...)
fn get_call_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    if node.kind() != "call" {
        return String::new();
    }

    // Try field-based access
    if let Some(func_node) = node.child_by_field_name("function") {
        match func_node.kind() {
            "identifier" => {
                return func_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "namespace_operator" => {
                // pkg::func or pkg:::func
                return func_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                return func_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
        }
    }

    // Fallback: first child is typically the function being called
    if let Some(first_child) = node.child(0) {
        match first_child.kind() {
            "identifier" => {
                return first_child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "namespace_operator" => {
                return first_child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                let text = first_child.utf8_text(source).unwrap_or("").trim().to_string();
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
                    .collect();
                return name;
            }
        }
    }

    String::new()
}

/// Get the first string argument from a call node's arguments.
///
/// Used to extract class names from `setClass("ClassName", ...)` and `R6Class("ClassName", ...)`.
fn get_first_string_arg(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Find the arguments node
    let args_node = node.child_by_field_name("arguments")
        .or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "arguments")
        })?;

    // Look for the first string argument
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        match child.kind() {
            "string" => {
                // Extract the string content (without quotes)
                return extract_string_content(child, source);
            }
            "argument" => {
                // Named or positional argument: check its value
                let mut arg_cursor = child.walk();
                for arg_child in child.children(&mut arg_cursor) {
                    if arg_child.kind() == "string" {
                        return extract_string_content(arg_child, source);
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Extract the text content of a string node (without surrounding quotes).
fn extract_string_content(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Try to find a string_content child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_content" {
            let text = child.utf8_text(source).unwrap_or("").to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }

    // Fallback: strip quotes from the full text
    let text = node.utf8_text(source).unwrap_or("").trim();
    let stripped = text
        .strip_prefix('"')
        .or_else(|| text.strip_prefix('\''))
        .unwrap_or(text);
    let stripped = stripped
        .strip_suffix('"')
        .or_else(|| stripped.strip_suffix('\''))
        .unwrap_or(stripped);

    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

// ─── Import Collection ──────────────────────────────────────────────────────

/// Collect import edges from the AST.
///
/// In R, imports come from:
/// - `library(pkg)` - load a package
/// - `require(pkg)` - conditionally load a package
/// - `source("file.R")` - source a file
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    collect_imports_recursive(node, file, source, edges);
}

fn collect_imports_recursive(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    if node.kind() == "call" {
        let callee = get_call_function_name(node, source);
        match callee.as_str() {
            "library" | "require" => {
                if let Some(pkg_name) = get_first_identifier_or_string_arg(node, source) {
                    add_import_edge(file, &pkg_name, &callee, edges);
                }
            }
            "source" => {
                if let Some(file_path) = get_first_string_arg(node, source) {
                    add_import_edge(file, &file_path, "source", edges);
                }
            }
            _ => {}
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports_recursive(child, file, source, edges);
    }
}

/// Get the first argument from a call, accepting either an identifier or a string.
///
/// In R, `library(dplyr)` uses an unquoted identifier, while `library("dplyr")` uses a string.
fn get_first_identifier_or_string_arg(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let args_node = node.child_by_field_name("arguments")
        .or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "arguments")
        })?;

    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
            "string" => {
                return extract_string_content(child, source);
            }
            "argument" => {
                // Check value of named/positional argument
                let mut arg_cursor = child.walk();
                for arg_child in child.children(&mut arg_cursor) {
                    match arg_child.kind() {
                        "identifier" => {
                            let text = arg_child.utf8_text(source).unwrap_or("").trim().to_string();
                            if !text.is_empty() && text != "=" {
                                return Some(text);
                            }
                        }
                        "string" => {
                            return extract_string_content(arg_child, source);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Add an import edge, avoiding duplicates.
fn add_import_edge(file: &str, target: &str, import_type: &str, edges: &mut Vec<Edge>) {
    if !edges.iter().any(|e| {
        e.kind == EdgeKind::Imports && e.source_fqn == file && e.target_fqn == target
    }) {
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({"import_type": import_type}),
        });
    }
}

// ─── Call Collection ────────────────────────────────────────────────────────

/// Collect intra-file function calls.
///
/// In R, function calls use standard syntax: `name(args)`.
/// Also handles namespace-qualified calls: `pkg::func(args)` and `pkg:::func(args)`.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    collect_calls_recursive(node, file, source, defined_fqns, edges);
}

fn collect_calls_recursive(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    if node.kind() == "call" {
        extract_call_edge(node, file, source, defined_fqns, edges);
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

/// Extract a call edge from a call node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let call_name = get_call_function_name(node, source);
    if call_name.is_empty() || is_r_keyword(&call_name) {
        return;
    }

    // Skip import-related calls (already handled as imports)
    if matches!(call_name.as_str(), "library" | "require" | "source") {
        return;
    }

    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Handle namespace-qualified calls: pkg::func or pkg:::func
    if call_name.contains("::") {
        let parts: Vec<&str> = if call_name.contains(":::") {
            call_name.splitn(2, ":::").collect()
        } else {
            call_name.splitn(2, "::").collect()
        };

        if parts.len() == 2 {
            let _namespace = parts[0];
            let func_name = parts[1];

            // Try to resolve to a local definition
            if let Some((_, target_fqn)) =
                defined_fqns.iter().find(|(simple, _)| simple == func_name)
            {
                if source_fqn != *target_fqn {
                    add_call_edge(&source_fqn, target_fqn, Some(&call_name), edges);
                }
            } else {
                // Emit unresolved qualified call for cross-file resolution
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.clone(),
                    target_fqn: call_name.to_string(),
                    kind: EdgeKind::Calls,
                    confidence: 0.0,
                    attributes: json!({"call_type": "qualified"}),
                });
            }
            return;
        }
    }

    // Simple function call - try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == &call_name) {
        if source_fqn != *target_fqn {
            add_call_edge(&source_fqn, target_fqn, None, edges);
        }
    }
}

/// Add a call edge, avoiding duplicates.
fn add_call_edge(
    source_fqn: &str,
    target_fqn: &str,
    qualified_name: Option<&str>,
    edges: &mut Vec<Edge>,
) {
    if edges.iter().any(|e| {
        e.kind == EdgeKind::Calls && e.source_fqn == source_fqn && e.target_fqn == target_fqn
    }) {
        return;
    }

    let attributes = match qualified_name {
        Some(name) => json!({"call_type": "qualified", "qualified_name": name}),
        None => json!({}),
    };

    edges.push(Edge {
        id: None,
        source_fqn: source_fqn.to_string(),
        target_fqn: target_fqn.to_string(),
        kind: EdgeKind::Calls,
        confidence: 1.0,
        attributes,
    });
}

/// Find the enclosing function definition for a given node and return its FQN.
fn find_enclosing_function_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "left_assignment" | "equals_assignment" | "binary_operator" => {
                // Check if this assignment defines a function
                if let Some((name_node, value_node)) = get_assignment_parts(parent, source) {
                    if value_node.kind() == "function_definition" {
                        let name = name_node.utf8_text(source).unwrap_or("").trim().to_string();
                        if !name.is_empty() {
                            return Some(format!("{file}::{name}"));
                        }
                    }
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    None
}

/// Check if a name is an R keyword or built-in that should not be treated as a user call.
fn is_r_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "repeat"
            | "in"
            | "next"
            | "break"
            | "function"
            | "return"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_complex_"
            | "NA_character_"
            | "Inf"
            | "NaN"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse R source and extract.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_r::LANGUAGE.into())
            .expect("R grammar should load");
        let tree = parser.parse(source, None).expect("should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_r_extract_functions() {
        let source = r#"
clean_data <- function(df) {
  df[!is.na(df$value), ]
}

plot_results = function(data, title) {
  plot(data$x, data$y, main = title)
}

validate.input <- function(x) {
  stopifnot(is.numeric(x))
  x
}
"#;
        let result = parse_and_extract("R/analysis.R", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::clean_data"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::plot_results"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::validate.input"
            && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_r_extract_s4_class() {
        let source = r#"
setClass("Person",
  representation(
    name = "character",
    age = "numeric"
  )
)
"#;
        let result = parse_and_extract("R/classes.R", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "R/classes.R::Person"
            && n.kind == NodeKind::Class));

        let person = result.nodes.iter().find(|n| n.fqn == "R/classes.R::Person").unwrap();
        assert_eq!(person.attributes["class_system"], "S4");
    }

    #[test]
    fn test_r_extract_r6_class() {
        let source = r#"
Animal <- R6Class("Animal",
  public = list(
    name = NULL,
    initialize = function(name) {
      self$name <- name
    },
    speak = function() {
      cat(paste(self$name, "speaks\n"))
    }
  )
)
"#;
        let result = parse_and_extract("R/animal.R", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "R/animal.R::Animal"
            && n.kind == NodeKind::Class));

        let animal = result.nodes.iter().find(|n| n.fqn == "R/animal.R::Animal").unwrap();
        assert_eq!(animal.attributes["class_system"], "R6");
    }

    #[test]
    fn test_r_extract_library_imports() {
        let source = r#"
library(dplyr)
library(ggplot2)
require(tidyr)
source("utils/helpers.R")
"#;
        let result = parse_and_extract("R/analysis.R", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(imports.iter().any(|e| e.target_fqn == "dplyr"));
        assert!(imports.iter().any(|e| e.target_fqn == "ggplot2"));
        assert!(imports.iter().any(|e| e.target_fqn == "tidyr"));
        assert!(imports.iter().any(|e| e.target_fqn == "utils/helpers.R"));
    }

    #[test]
    fn test_r_extract_calls() {
        let source = r#"
helper <- function() {
  42
}

main <- function() {
  x <- helper()
  print(x)
}
"#;
        let result = parse_and_extract("R/main.R", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // main should call helper
        assert!(calls.iter().any(|e| e.source_fqn == "R/main.R::main"
            && e.target_fqn == "R/main.R::helper"));
    }

    #[test]
    fn test_r_extract_namespace_qualified_calls() {
        let source = r#"
process <- function(df) {
  dplyr::filter(df, x > 0)
}
"#;
        let result = parse_and_extract("R/process.R", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // Should have a qualified call to dplyr::filter
        assert!(calls.iter().any(|e| e.source_fqn == "R/process.R::process"
            && e.target_fqn == "dplyr::filter"));
    }

    #[test]
    fn test_r_full_file() {
        let source = r#"
library(dplyr)
library(ggplot2)
require(tidyr)
source("utils/helpers.R")

setClass("DataModel",
  representation(
    data = "data.frame",
    name = "character"
  )
)

Processor <- R6Class("Processor",
  public = list(
    model = NULL,
    initialize = function(model) {
      self$model <- model
    },
    run = function() {
      self$model
    }
  )
)

clean_data <- function(df) {
  df %>%
    filter(!is.na(value)) %>%
    mutate(normalized = value / max(value))
}

plot_results = function(data, title) {
  ggplot(data, aes(x = x, y = y)) +
    geom_point() +
    ggtitle(title)
}

main <- function() {
  df <- read.csv("data.csv")
  cleaned <- clean_data(df)
  plot_results(cleaned, "Results")
}
"#;
        let result = parse_and_extract("R/analysis.R", source);

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "dplyr"));
        assert!(imports.iter().any(|e| e.target_fqn == "ggplot2"));
        assert!(imports.iter().any(|e| e.target_fqn == "tidyr"));
        assert!(imports.iter().any(|e| e.target_fqn == "utils/helpers.R"));

        // Check S4 class
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::DataModel"
            && n.kind == NodeKind::Class));

        // Check R6 class
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::Processor"
            && n.kind == NodeKind::Class));

        // Check functions
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::clean_data"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::plot_results"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "R/analysis.R::main"
            && n.kind == NodeKind::Function));

        // Check calls from main
        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(calls.iter().any(|e| e.source_fqn == "R/analysis.R::main"
            && e.target_fqn == "R/analysis.R::clean_data"));
        assert!(calls.iter().any(|e| e.source_fqn == "R/analysis.R::main"
            && e.target_fqn == "R/analysis.R::plot_results"));
    }

    #[test]
    fn test_r_empty_file() {
        let result = parse_and_extract("empty.R", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_r_complexity_computed() {
        let source = r#"
complex_func <- function(x, y) {
  if (x > 0) {
    for (i in seq_len(y)) {
      if (i > 5) {
        return(i)
      }
    }
  } else {
    while (y > 0) {
      y <- y - 1
    }
  }
  0
}
"#;
        let result = parse_and_extract("R/complex.R", source);

        let func = result
            .nodes
            .iter()
            .find(|n| n.fqn == "R/complex.R::complex_func")
            .expect("should find complex_func");

        // Should have complexity attribute set (value > 1 due to branches)
        assert!(
            func.attributes.get("complexity").is_some(),
            "complexity should be computed for function nodes"
        );
        let complexity = func.attributes["complexity"].as_u64().unwrap();
        assert!(
            complexity >= 4,
            "complex function should have complexity >= 4, got {}",
            complexity
        );
    }

    #[test]
    fn test_r_backward_compat_extract_regex() {
        let source = r#"
library(dplyr)

clean_data <- function(df) {
  df
}
"#;
        #[allow(deprecated)]
        let result = extract_regex("R/test.R", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "R/test.R::clean_data"
            && n.kind == NodeKind::Function));
        assert!(result.edges.iter().any(|e| e.kind == EdgeKind::Imports
            && e.target_fqn == "dplyr"));
    }
}
