//! PHP AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (classes, interfaces, traits, enums, functions,
//! constants, namespaces) and edges (imports, calls, inheritance, implementation)
//! from a tree-sitter PHP parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed PHP file.
///
/// Handles:
/// - Class declarations (including abstract/final)
/// - Interface declarations
/// - Trait declarations
/// - Enum declarations (PHP 8.1+)
/// - Function definitions (top-level)
/// - Method declarations (inside class/interface/trait)
/// - Namespace definitions
/// - Constant declarations
/// - Use/require/include import statements
/// - Intra-file call expressions resolved to definitions in the same file
/// - Inheritance (extends) and implementation (implements) edges
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(
        root,
        file,
        source_bytes,
        None,
        &mut nodes,
        &mut defined_fqns,
        &mut edges,
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
pub fn extract_php(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .expect("PHP grammar should load");
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
            // Try method_declaration first, then function_definition
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "method_declaration")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "function_definition"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "php");
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


/// Recursively collect definitions: classes, interfaces, traits, enums,
/// functions, methods, constants, and namespaces.
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                extract_class(child, file, source, nodes, defined_fqns, edges);
            }
            "interface_declaration" => {
                extract_interface(child, file, source, nodes, defined_fqns, edges);
            }
            "trait_declaration" => {
                extract_trait(child, file, source, nodes, defined_fqns, edges);
            }
            "enum_declaration" => {
                extract_enum(child, file, source, nodes, defined_fqns);
            }
            "function_definition" => {
                if class_name.is_none() {
                    extract_function(child, file, source, None, nodes, defined_fqns);
                }
            }
            "method_declaration" => {
                extract_function(child, file, source, class_name, nodes, defined_fqns);
            }
            "namespace_definition" => {
                extract_namespace(child, file, source, nodes, defined_fqns, edges);
            }
            "const_declaration" => {
                extract_constants(child, file, source, class_name, nodes, defined_fqns);
            }
            // The PHP grammar wraps the actual content in a "program" node
            "program" => {
                collect_definitions(child, file, source, class_name, nodes, defined_fqns, edges);
            }
            // PHP text content is wrapped in php_tag/text nodes
            "php_tag" | "text_interpolation" | "text" => {}
            _ => {
                // Recurse into compound statements and other containers
                collect_definitions(child, file, source, class_name, nodes, defined_fqns, edges);
            }
        }
    }
}

/// Extract a class declaration node.
fn extract_class(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_name(node, source, "name");
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Class,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Walk children to find base_clause, class_interface_clause, and declaration_list
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "base_clause" => {
                // extends ParentClass
                extract_names_from_clause(child, source, &fqn, EdgeKind::Inherits, edges);
            }
            "class_interface_clause" => {
                // implements Interface1, Interface2
                extract_names_from_clause(child, source, &fqn, EdgeKind::Implements, edges);
            }
            "declaration_list" => {
                collect_class_members(child, file, source, &name, nodes, defined_fqns, edges);
            }
            _ => {}
        }
    }

    // Also check body field (some grammar versions)
    if let Some(body) = node.child_by_field_name("body") {
        if body.kind() == "declaration_list" {
            // Already handled above via children iteration
        } else {
            collect_class_members(body, file, source, &name, nodes, defined_fqns, edges);
        }
    }
}

/// Extract an interface declaration node.
fn extract_interface(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_name(node, source, "name");
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Interface,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Walk children to find base_clause and declaration_list
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "base_clause" => {
                // Interfaces can extend other interfaces
                extract_names_from_clause(child, source, &fqn, EdgeKind::Inherits, edges);
            }
            "declaration_list" => {
                collect_class_members(child, file, source, &name, nodes, defined_fqns, edges);
            }
            _ => {}
        }
    }
}

/// Extract a trait declaration node.
fn extract_trait(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_name(node, source, "name");
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Trait,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({}),
    });

    defined_fqns.push((name.clone(), fqn));

    // Walk children to find declaration_list
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            collect_class_members(child, file, source, &name, nodes, defined_fqns, edges);
        }
    }
}

/// Extract an enum declaration node (PHP 8.1+).
fn extract_enum(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_child_name(node, source, "name");
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Enum,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract a function or method definition.
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_child_name(node, source, "name");
    if name.is_empty() {
        return;
    }

    let fqn = match class_name {
        Some(cls) => format!("{file}::{cls}::{name}"),
        None => format!("{file}::{name}"),
    };
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

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

/// Extract a namespace definition.
fn extract_namespace(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    // Namespace name can be in a "name" field or a child "namespace_name" node
    let name = get_child_name(node, source, "name");
    let ns_name = if name.is_empty() {
        // Try to find a namespace_name or qualified_name child
        get_namespace_name(node, source)
    } else {
        name
    };

    if !ns_name.is_empty() {
        let fqn = format!("{file}::{}", ns_name.replace('\\', "::"));
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Namespace,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });

        defined_fqns.push((ns_name, fqn));
    }

    // Recurse into namespace body
    if let Some(body) = node.child_by_field_name("body") {
        collect_definitions(body, file, source, None, nodes, defined_fqns, edges);
    } else {
        // Some namespace definitions don't have a body field; recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "compound_statement" || child.kind() == "declaration_list" {
                collect_definitions(child, file, source, None, nodes, defined_fqns, edges);
            }
        }
    }
}

/// Extract constant declarations.
fn extract_constants(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // const declarations can have multiple declarators
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_element" {
            let name = get_child_name(child, source, "name");
            if name.is_empty() {
                // Try getting the first identifier child
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "name" || inner_child.kind() == "identifier" {
                        let text = inner_child.utf8_text(source).unwrap_or("").to_string();
                        if !text.is_empty() {
                            let fqn = match class_name {
                                Some(cls) => format!("{file}::{cls}::{text}"),
                                None => format!("{file}::{text}"),
                            };
                            let start_line = child.start_position().row as u32 + 1;
                            let end_line = child.end_position().row as u32 + 1;

                            nodes.push(Node {
                                fqn: fqn.clone(),
                                kind: NodeKind::Constant,
                                file: file.to_string(),
                                start_line,
                                end_line,
                                file_hash: String::new(),
                                indexed_at: 0,
                                attributes: json!({}),
                            });

                            defined_fqns.push((text, fqn));
                        }
                        break;
                    }
                }
            } else {
                let fqn = match class_name {
                    Some(cls) => format!("{file}::{cls}::{name}"),
                    None => format!("{file}::{name}"),
                };
                let start_line = child.start_position().row as u32 + 1;
                let end_line = child.end_position().row as u32 + 1;

                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Constant,
                    file: file.to_string(),
                    start_line,
                    end_line,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes: json!({}),
                });

                defined_fqns.push((name, fqn));
            }
        }
    }
}


/// Collect members (methods, constants) inside a class/interface/trait body.
fn collect_class_members(
    body: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                extract_function(child, file, source, Some(class_name), nodes, defined_fqns);
            }
            "function_definition" => {
                extract_function(child, file, source, Some(class_name), nodes, defined_fqns);
            }
            "const_declaration" => {
                extract_constants(child, file, source, Some(class_name), nodes, defined_fqns);
            }
            // Use statements inside class body (trait use)
            "use_declaration" => {
                extract_use_edge(child, file, source, edges);
            }
            _ => {}
        }
    }
}

/// Collect import statements (use declarations, require/include).
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_declaration" | "namespace_use_declaration" => {
                extract_use_edge(child, file, source, edges);
            }
            "expression_statement" => {
                // require/include are expression statements in PHP
                extract_require_include(child, file, source, edges);
            }
            "program" => {
                collect_imports(child, file, source, edges);
            }
            _ => {
                // Recurse to find nested imports
                collect_imports(child, file, source, edges);
            }
        }
    }
}

/// Extract a use declaration into an Imports edge.
fn extract_use_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    // Walk children to find namespace names / qualified names
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_name" | "qualified_name" | "name" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "use" {
                    let normalized = text.replace('\\', ".");
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: normalized,
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
            "namespace_use_clause" | "use_clause" => {
                // Group use: use Foo\{Bar, Baz}
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    let normalized = text
                        .split_whitespace()
                        .next()
                        .unwrap_or(&text)
                        .replace('\\', ".");
                    if !normalized.is_empty() {
                        edges.push(Edge {
                            id: None,
                            source_fqn: file.to_string(),
                            target_fqn: normalized,
                            kind: EdgeKind::Imports,
                            confidence: 1.0,
                            attributes: json!({}),
                        });
                    }
                }
            }
            "namespace_use_group" => {
                // Recurse into group use clauses
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "namespace_use_clause" {
                        let text = inner_child.utf8_text(source).unwrap_or("").trim().to_string();
                        if !text.is_empty() {
                            let normalized = text
                                .split_whitespace()
                                .next()
                                .unwrap_or(&text)
                                .replace('\\', ".");
                            if !normalized.is_empty() {
                                edges.push(Edge {
                                    id: None,
                                    source_fqn: file.to_string(),
                                    target_fqn: normalized,
                                    kind: EdgeKind::Imports,
                                    confidence: 1.0,
                                    attributes: json!({}),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback: if no edges were added from children, try the full text
    // This handles cases where the grammar structure differs
    let full_text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if !full_text.is_empty() && !edges.iter().any(|e| e.source_fqn == file && full_text.contains(&e.target_fqn.replace('.', "\\"))) {
        // Parse "use Foo\Bar\Baz;" or "use Foo\Bar as Alias;"
        let stripped = full_text
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .trim();
        if !stripped.is_empty() && stripped.contains('\\') {
            let target = stripped.split_whitespace().next().unwrap_or(stripped);
            let normalized = target.replace('\\', ".");
            if !normalized.is_empty() && !edges.iter().any(|e| e.target_fqn == normalized) {
                edges.push(Edge {
                    id: None,
                    source_fqn: file.to_string(),
                    target_fqn: normalized,
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    attributes: json!({}),
                });
            }
        }
    }
}

/// Extract require/include statements.
fn extract_require_include(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "require_expression" | "require_once_expression"
            | "include_expression" | "include_once_expression" => {
                // The argument is typically a string child
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "string"
                        || inner_child.kind() == "encapsed_string"
                        || inner_child.kind() == "string_value"
                    {
                        let text = inner_child.utf8_text(source).unwrap_or("");
                        let path = text.trim_matches('\'').trim_matches('"');
                        if !path.is_empty() {
                            edges.push(Edge {
                                id: None,
                                source_fqn: file.to_string(),
                                target_fqn: path.to_string(),
                                kind: EdgeKind::Imports,
                                confidence: 0.9,
                                attributes: json!({"require": true}),
                            });
                        }
                    }
                }
            }
            _ => {
                // Recurse
                extract_require_include(child, file, source, edges);
            }
        }
    }
}

/// Collect intra-file call expressions and create Calls edges.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_call_expression" {
            extract_call_edge(child, file, source, defined_fqns, edges);
        }

        // Recurse into children
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

/// Extract a single call edge from a function_call_expression node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    // The function being called is typically the first child or "function" field
    let func_node = node.child_by_field_name("function").or_else(|| node.child(0));

    if let Some(func) = func_node {
        let call_text = func.utf8_text(source).unwrap_or("");

        // Only resolve simple function calls (identifiers), not method calls
        if func.kind() == "name" || func.kind() == "identifier" {
            if let Some((_, target_fqn)) = defined_fqns.iter().find(|(name, _)| name == call_text) {
                let caller_fqn = find_enclosing_function(node, file, source);
                if let Some(caller) = caller_fqn {
                    if caller != *target_fqn {
                        edges.push(Edge {
                            id: None,
                            source_fqn: caller,
                            target_fqn: target_fqn.clone(),
                            kind: EdgeKind::Calls,
                            confidence: 1.0,
                            attributes: json!({}),
                        });
                    }
                }
            }
        }
    }
}

/// Find the enclosing function/method for a given node to determine the caller FQN.
fn find_enclosing_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    let mut class_name: Option<String> = None;

    while let Some(parent) = current {
        if parent.kind() == "function_definition" || parent.kind() == "method_declaration" {
            let name = get_child_name(parent, source, "name");
            if !name.is_empty() {
                // Check if this function is inside a class/interface/trait
                let mut ancestor = parent.parent();
                while let Some(anc) = ancestor {
                    match anc.kind() {
                        "class_declaration" | "interface_declaration"
                        | "trait_declaration" | "enum_declaration" => {
                            let cls = get_child_name(anc, source, "name");
                            if !cls.is_empty() {
                                class_name = Some(cls);
                            }
                            break;
                        }
                        _ => {}
                    }
                    ancestor = anc.parent();
                }
                return match class_name {
                    Some(cls) => Some(format!("{file}::{cls}::{name}")),
                    None => Some(format!("{file}::{name}")),
                };
            }
        }
        current = parent.parent();
    }
    // Call at file level
    Some(file.to_string())
}

// ─── Helper functions ───────────────────────────────────────────────────────

/// Get the text of a named child field (typically "name").
fn get_child_name(node: tree_sitter::Node, source: &[u8], field: &str) -> String {
    if let Some(name_node) = node.child_by_field_name(field) {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    String::new()
}

/// Get namespace name from a namespace_definition node by looking for
/// namespace_name or qualified_name children.
fn get_namespace_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_name" | "qualified_name" | "name" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "namespace" {
                    return text;
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// Extract type names from a base_clause or class_interface_clause node.
/// These clauses contain keyword(s) like "extends"/"implements" followed by name nodes.
fn extract_names_from_clause(
    clause: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edge_kind: EdgeKind,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            "name" | "qualified_name" | "namespace_name" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "extends" && text != "implements" {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.to_string(),
                        target_fqn: text,
                        kind: edge_kind.clone(),
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
            // Skip keywords and punctuation
            "extends" | "implements" | "," => {}
            _ => {
                // Recurse into nested structures (e.g., name_list)
                extract_names_from_clause(child, source, source_fqn, edge_kind.clone(), edges);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn parse_php(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_class_with_methods() {
        let source = r#"<?php

class UserController {
    public function index(): Response {
        $users = User::all();
        return response()->json($users);
    }

    public function store(Request $request): Response {
        $user = User::create($request->validated());
        return response()->json($user, 201);
    }

    protected function validate(): bool {
        return true;
    }
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Controllers/UserController.php", source);

        // Check class node
        let class_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class)
            .expect("Should find a class node");
        assert_eq!(
            class_node.fqn,
            "app/Controllers/UserController.php::UserController"
        );

        // Check method nodes
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Controllers/UserController.php::UserController::index"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Controllers/UserController.php::UserController::store"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Controllers/UserController.php::UserController::validate"
            && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"<?php

interface Authenticatable {
    public function authenticate(): bool;
    public function getToken(): string;
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Contracts/Authenticatable.php", source);

        let iface = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Interface)
            .expect("Should find an interface node");
        assert_eq!(
            iface.fqn,
            "app/Contracts/Authenticatable.php::Authenticatable"
        );

        // Interface methods
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Contracts/Authenticatable.php::Authenticatable::authenticate"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Contracts/Authenticatable.php::Authenticatable::getToken"
            && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_extract_trait() {
        let source = r#"<?php

trait Loggable {
    public function log(string $message): void {
        echo $message;
    }

    public function error(string $message): void {
        echo "ERROR: " . $message;
    }
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Traits/Loggable.php", source);

        let trait_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Trait)
            .expect("Should find a trait node");
        assert_eq!(trait_node.fqn, "app/Traits/Loggable.php::Loggable");

        // Trait methods
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Traits/Loggable.php::Loggable::log"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "app/Traits/Loggable.php::Loggable::error"
            && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"<?php

enum Status {
    case Active;
    case Inactive;
    case Pending;
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Enums/Status.php", source);

        let enum_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Enum)
            .expect("Should find an enum node");
        assert_eq!(enum_node.fqn, "app/Enums/Status.php::Status");
    }

    #[test]
    fn test_extract_namespace() {
        let source = r#"<?php

namespace App\Controllers;

class HomeController {
    public function index(): void {}
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Controllers/HomeController.php", source);

        // Should find namespace node
        let ns_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Namespace);
        assert!(
            ns_node.is_some(),
            "Should find a namespace node. Found nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );

        // Should also find the class
        assert!(result
            .nodes
            .iter()
            .any(|n| n.kind == NodeKind::Class && n.fqn.contains("HomeController")));
    }

    #[test]
    fn test_extract_use_imports() {
        let source = r#"<?php

namespace App\Controllers;

use App\Models\User;
use App\Services\AuthService;

class UserController {}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Controllers/UserController.php", source);

        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            import_edges.iter().any(|e| e.target_fqn == "App.Models.User"),
            "Should import App.Models.User. Found: {:?}",
            import_edges.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "App.Services.AuthService"),
            "Should import App.Services.AuthService. Found: {:?}",
            import_edges.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_require_include() {
        let source = r#"<?php

require_once 'vendor/autoload.php';
include 'config/database.php';

function main() {}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "index.php", source);

        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports && e.attributes.get("require").is_some())
            .collect();

        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "vendor/autoload.php"),
            "Should find require for vendor/autoload.php. Found: {:?}",
            import_edges.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "config/database.php"),
            "Should find include for config/database.php. Found: {:?}",
            import_edges.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_intra_file_calls() {
        let source = r#"<?php

function validate($data) {
    return !empty($data);
}

function process($data) {
    validate($data);
    return transform($data);
}

function transform($data) {
    return $data;
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/helpers.php", source);

        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // process calls validate
        assert!(
            call_edges.iter().any(|e| e.source_fqn == "app/helpers.php::process"
                && e.target_fqn == "app/helpers.php::validate"),
            "process should call validate. Found calls: {:?}",
            call_edges
                .iter()
                .map(|e| (&e.source_fqn, &e.target_fqn))
                .collect::<Vec<_>>()
        );

        // process calls transform
        assert!(
            call_edges.iter().any(|e| e.source_fqn == "app/helpers.php::process"
                && e.target_fqn == "app/helpers.php::transform"),
            "process should call transform. Found calls: {:?}",
            call_edges
                .iter()
                .map(|e| (&e.source_fqn, &e.target_fqn))
                .collect::<Vec<_>>()
        );
    }



    #[test]
    fn test_extract_inheritance() {
        let source = r#"<?php

abstract class BaseController {
    abstract protected function validate(): bool;
}

class UserController extends BaseController {
    protected function validate(): bool {
        return true;
    }
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Controllers.php", source);

        let inherits_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();

        assert!(
            inherits_edges.iter().any(|e| e.source_fqn == "app/Controllers.php::UserController"
                && e.target_fqn == "BaseController"),
            "UserController should inherit from BaseController. Found: {:?}",
            inherits_edges
                .iter()
                .map(|e| (&e.source_fqn, &e.target_fqn))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_implements() {
        let source = r#"<?php

interface Serializable {
    public function serialize(): string;
}

interface Countable {
    public function count(): int;
}

class Collection implements Serializable, Countable {
    public function serialize(): string { return ""; }
    public function count(): int { return 0; }
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Collection.php", source);

        let impl_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();

        assert!(
            impl_edges.iter().any(|e| e.source_fqn == "app/Collection.php::Collection"
                && e.target_fqn == "Serializable"),
            "Collection should implement Serializable. Found: {:?}",
            impl_edges
                .iter()
                .map(|e| (&e.source_fqn, &e.target_fqn))
                .collect::<Vec<_>>()
        );
        assert!(
            impl_edges.iter().any(|e| e.source_fqn == "app/Collection.php::Collection"
                && e.target_fqn == "Countable"),
            "Collection should implement Countable. Found: {:?}",
            impl_edges
                .iter()
                .map(|e| (&e.source_fqn, &e.target_fqn))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_php(source);
        let result = extract(&tree, "empty.php", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_php_only_tag() {
        let source = "<?php\n";
        let tree = parse_php(source);
        let result = extract(&tree, "empty.php", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"<?php

class MyClass {
    public function myMethod(): void {}
}

function standalone(): void {}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "src/app.php", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/app.php::MyClass"));
        assert!(fqns.contains(&"src/app.php::MyClass::myMethod"));
        assert!(fqns.contains(&"src/app.php::standalone"));
    }

    #[test]
    fn test_node_line_numbers() {
        let source = "<?php\nfunction foo(): void {\n}\n\nfunction bar(): void {\n}\n";
        let tree = parse_php(source);
        let result = extract(&tree, "test.php", source);

        let foo = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.php::foo")
            .expect("Should find foo");
        assert_eq!(foo.start_line, 2);
        assert_eq!(foo.end_line, 3);

        let bar = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.php::bar")
            .expect("Should find bar");
        assert_eq!(bar.start_line, 5);
        assert_eq!(bar.end_line, 6);
    }

    #[test]
    fn test_deprecated_extract_php_wrapper() {
        #[allow(deprecated)]
        let result = extract_php("test.php", "<?php\nfunction hello(): void {}\n");
        assert!(result.nodes.iter().any(|n| n.fqn == "test.php::hello"));
    }

    #[test]
    fn test_constants() {
        let source = r#"<?php

const MAX_RETRIES = 3;
const DEFAULT_TIMEOUT = 30;

class Config {
    const VERSION = "1.0.0";
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/config.php", source);

        let constants: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constant)
            .collect();

        // Should find at least the top-level constants
        let const_fqns: Vec<&str> = constants.iter().map(|n| n.fqn.as_str()).collect();
        assert!(
            const_fqns.iter().any(|f| f.contains("MAX_RETRIES")),
            "Should find MAX_RETRIES constant. Found: {:?}",
            const_fqns
        );
        assert!(
            const_fqns.iter().any(|f| f.contains("DEFAULT_TIMEOUT")),
            "Should find DEFAULT_TIMEOUT constant. Found: {:?}",
            const_fqns
        );
    }

    #[test]
    fn test_invalid_syntax_partial_extraction() {
        let source = r#"<?php

function valid_function(): void {
    echo "hello";
}

class Broken {{{
    invalid syntax here @@@
}

function another_valid(): void {}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "broken.php", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "broken.php::valid_function"));
    }

    #[test]
    fn test_full_example() {
        let source = r#"<?php

namespace App\Controllers;

use App\Models\User;
use App\Services\AuthService;

abstract class BaseController {
    abstract protected function validate(): bool;
}

class UserController extends BaseController {
    public function index(): Response {
        $users = getUsers();
        return response()->json($users);
    }

    protected function validate(): bool {
        return true;
    }
}

function getUsers(): array {
    return [];
}
"#;
        let tree = parse_php(source);
        let result = extract(&tree, "app/Controllers/UserController.php", source);

        // Verify we have classes, functions, namespace, imports, inheritance, and calls
        assert!(result.nodes.iter().any(|n| n.kind == NodeKind::Class));
        assert!(result.nodes.iter().any(|n| n.kind == NodeKind::Function));
        assert!(result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Imports));
        assert!(result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Inherits));
    }
}
