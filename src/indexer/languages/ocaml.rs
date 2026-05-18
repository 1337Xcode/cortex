//! OCaml AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (modules, functions, types, functors)
//! and edges (imports, intra-file calls) from a tree-sitter OCaml parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed OCaml file.
///
/// Handles:
/// - Module declarations (`module Name = struct ... end`)
/// - Module types/signatures (`module type Name = sig ... end`)
/// - Functions (`let name args = expr`, `let rec name args = expr`)
/// - Type declarations (variants, records, aliases)
/// - Functors (`module Name = functor (Arg : SIG) -> struct ... end`)
/// - Imports (`open Module`)
/// - Includes (`include Module`)
/// - Intra-file function calls
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
        .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
        .expect("OCaml grammar should load");
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
            if let Some(ast_node) = find_ast_node_at_line(root, node.start_line, "value_definition")
                .or_else(|| find_ast_node_at_line(root, node.start_line, "let_binding"))
                .or_else(|| find_ast_node_at_line(root, node.start_line, "binding"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "ocaml");
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
/// In tree-sitter-ocaml, top-level structures include:
/// - `value_definition` (let bindings: `let name = ...`, `let rec name = ...`)
/// - `module_definition` (module declarations: `module Name = ...`)
/// - `module_type_definition` (module type: `module type Name = ...`)
/// - `type_definition` (type declarations: `type name = ...`)
/// - `structure` (top-level structure wrapping items)
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
            // Let bindings: let name = expr, let rec name = expr
            "value_definition" => {
                extract_let_binding(child, file, source, nodes, defined_fqns);
            }
            // Module definition: module Name = struct ... end
            "module_definition" => {
                extract_module_definition(child, file, source, nodes, defined_fqns);
            }
            // Module type definition: module type Name = sig ... end
            "module_type_definition" => {
                extract_module_type_definition(child, file, source, nodes, defined_fqns);
            }
            // Type definition: type name = ...
            "type_definition" => {
                extract_type_definition(child, file, source, nodes, defined_fqns);
            }
            // Structure items wrapper
            "structure" | "structure_item" | "compilation_unit" => {
                collect_definitions(child, file, source, nodes, defined_fqns);
            }
            _ => {
                // Recurse into other node types that might contain definitions
                if child.child_count() > 0
                    && !matches!(
                        child.kind(),
                        "open_statement" | "include_statement" | "expression"
                    )
                {
                    collect_definitions(child, file, source, nodes, defined_fqns);
                }
            }
        }
    }
}

/// Extract a let binding (function or value).
///
/// In tree-sitter-ocaml, `value_definition` contains:
/// - `let` keyword
/// - optional `rec` keyword
/// - one or more `let_binding` children with pattern and body
///
/// We treat bindings with parameters as functions.
fn extract_let_binding(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let is_rec = node_text_contains(node, source, "rec");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Find let_binding children (there may be multiple with `and`)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "let_binding" {
            if let Some(name) = get_let_binding_name(child, source) {
                if name == "_" || is_ocaml_keyword(&name) {
                    continue;
                }

                let binding_start = child.start_position().row as u32 + 1;
                let binding_end = child.end_position().row as u32 + 1;

                let fqn = format!("{file}::{name}");

                // Avoid duplicates
                if nodes.iter().any(|n| n.fqn == fqn) {
                    continue;
                }

                let mut attributes = json!({});
                if is_rec {
                    if let Some(attrs) = attributes.as_object_mut() {
                        attrs.insert("recursive".to_string(), json!(true));
                    }
                }

                nodes.push(Node {
                    fqn: fqn.clone(),
                    kind: NodeKind::Function,
                    file: file.to_string(),
                    start_line: binding_start,
                    end_line: binding_end,
                    file_hash: String::new(),
                    indexed_at: 0,
                    attributes,
                });

                defined_fqns.push((name, fqn));
            }
        }
    }

    // Fallback: if no let_binding children found, try to extract name directly
    if !node.children(&mut node.walk()).any(|c| c.kind() == "let_binding") {
        if let Some(name) = get_value_def_name(node, source) {
            if name != "_" && !is_ocaml_keyword(&name) {
                let fqn = format!("{file}::{name}");
                if !nodes.iter().any(|n| n.fqn == fqn) {
                    let mut attributes = json!({});
                    if is_rec {
                        if let Some(attrs) = attributes.as_object_mut() {
                            attrs.insert("recursive".to_string(), json!(true));
                        }
                    }

                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Function,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes,
                    });

                    defined_fqns.push((name, fqn));
                }
            }
        }
    }
}

/// Extract a module definition.
///
/// Patterns:
/// - `module Name = struct ... end`
/// - `module Name = functor (Arg : SIG) -> struct ... end`
/// - `module Name (Arg : SIG) = struct ... end` (sugar for functor)
fn extract_module_definition(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_module_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    // Check if this is a functor
    let is_functor = is_functor_definition(node, source);

    let mut attributes = json!({});
    if is_functor {
        if let Some(attrs) = attributes.as_object_mut() {
            attrs.insert("functor".to_string(), json!(true));
        }
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes,
    });

    defined_fqns.push((name, fqn));
}

/// Extract a module type definition.
///
/// Pattern: `module type Name = sig ... end`
fn extract_module_type_definition(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_module_type_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"module_type": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract a type definition.
///
/// Patterns:
/// - `type name = variant1 | variant2` → NodeKind::Class (variant type)
/// - `type name = { field1: type; ... }` → NodeKind::Class (record type)
/// - `type name = existing_type` → NodeKind::TypeAlias
fn extract_type_definition(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // A type_definition may contain multiple type_binding children (with `and`)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_binding" {
            extract_single_type_binding(child, file, source, nodes, defined_fqns);
        }
    }

    // Fallback: try to extract directly from the node if no type_binding children
    if !node.children(&mut node.walk()).any(|c| c.kind() == "type_binding") {
        extract_single_type_binding(node, file, source, nodes, defined_fqns);
    }
}

/// Extract a single type binding (part of a type definition).
fn extract_single_type_binding(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_name(node, source);
    if name.is_empty() || is_ocaml_keyword(&name) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    // Determine if this is a variant/record type or a simple alias
    let type_kind = classify_type_definition(node, source);

    let (kind, attributes) = match type_kind {
        TypeDefKind::Variant => (NodeKind::Class, json!({"type_kind": "variant"})),
        TypeDefKind::Record => (NodeKind::Class, json!({"type_kind": "record"})),
        TypeDefKind::Alias => (NodeKind::TypeAlias, json!({})),
        TypeDefKind::Abstract => (NodeKind::TypeAlias, json!({"abstract": true})),
    };

    nodes.push(Node {
        fqn: fqn.clone(),
        kind,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes,
    });

    defined_fqns.push((name, fqn));
}

/// Classification of OCaml type definitions.
enum TypeDefKind {
    Variant,  // type t = A | B | C
    Record,   // type t = { field: type; ... }
    Alias,    // type t = existing_type
    Abstract, // type t (no definition body)
}

/// Classify a type definition based on its body.
fn classify_type_definition(node: tree_sitter::Node, source: &[u8]) -> TypeDefKind {
    let text = node.utf8_text(source).unwrap_or("");

    // Check for variant constructors (lines starting with | or uppercase after =)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variant_declaration" | "constructor_declaration" => return TypeDefKind::Variant,
            "record_declaration" | "record_type" => return TypeDefKind::Record,
            _ => {}
        }
    }

    // Fallback: check text patterns
    if let Some(after_eq) = text.split_once('=') {
        let body = after_eq.1.trim();
        if body.is_empty() {
            return TypeDefKind::Abstract;
        }
        if body.starts_with('{') {
            return TypeDefKind::Record;
        }
        if body.contains('|') || body.chars().next().map_or(false, |c| c.is_uppercase()) {
            return TypeDefKind::Variant;
        }
        return TypeDefKind::Alias;
    }

    TypeDefKind::Abstract
}

/// Collect import edges from the AST.
///
/// In OCaml, imports are:
/// - `open Module` (brings module contents into scope)
/// - `include Module` (includes module contents in current module)
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "open_statement" | "open_module" => {
                if let Some(target) = get_open_module_name(child, source) {
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
            }
            "include_statement" | "include_module" => {
                if let Some(target) = get_include_module_name(child, source) {
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
                            attributes: json!({"include": true}),
                        });
                    }
                }
            }
            _ => {
                // Recurse into children to find nested opens/includes
                if child.child_count() > 0 {
                    collect_imports(child, file, source, edges);
                }
            }
        }
    }
}

/// Collect intra-file function calls.
///
/// In OCaml, function application is juxtaposition: `f x` applies f to x.
/// In tree-sitter-ocaml, this is represented as `application` nodes.
/// We also look for qualified calls like `Module.function`.
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
    let kind = node.kind();

    match kind {
        // Function application: f x y
        "application" | "function_application" | "application_expression" => {
            // The first child is the function being called
            if let Some(func_node) = node.child(0) {
                let call_name = get_applied_function_name(func_node, source);
                if !call_name.is_empty() && !is_ocaml_keyword(&call_name) {
                    let source_fqn = find_enclosing_function_fqn(node, file, source)
                        .unwrap_or_else(|| file.to_string());

                    // Check for qualified call (Module.function)
                    if call_name.contains('.') {
                        let parts: Vec<&str> = call_name.rsplitn(2, '.').collect();
                        if parts.len() == 2 {
                            let method = parts[0];
                            let receiver = parts[1];

                            if let Some((_, target_fqn)) =
                                defined_fqns.iter().find(|(simple, _)| simple == method)
                            {
                                if source_fqn != *target_fqn {
                                    edges.push(Edge {
                                        id: None,
                                        source_fqn: source_fqn.clone(),
                                        target_fqn: target_fqn.clone(),
                                        kind: EdgeKind::Calls,
                                        confidence: 1.0,
                                        attributes: json!({"receiver": receiver, "call_type": "qualified"}),
                                    });
                                }
                            } else {
                                edges.push(Edge {
                                    id: None,
                                    source_fqn: source_fqn.clone(),
                                    target_fqn: call_name.to_string(),
                                    kind: EdgeKind::Calls,
                                    confidence: 0.0,
                                    attributes: json!({"receiver": receiver, "call_type": "qualified"}),
                                });
                            }
                            return;
                        }
                    }

                    // Simple function call
                    if let Some((_, target_fqn)) =
                        defined_fqns.iter().find(|(simple, _)| simple == &call_name)
                    {
                        if source_fqn != *target_fqn {
                            // Avoid duplicate edges
                            if !edges.iter().any(|e| {
                                e.kind == EdgeKind::Calls
                                    && e.source_fqn == source_fqn
                                    && e.target_fqn == *target_fqn
                            }) {
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
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Get the name from a let_binding node.
/// In tree-sitter-ocaml, a let_binding has a pattern (name) and a body.
fn get_let_binding_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // The pattern/name is typically the first identifier child
            "value_name" | "variable" | "identifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "=" && text != "let" && text != "rec" {
                    return Some(text);
                }
            }
            // In some grammar versions, the name is inside a value_pattern
            "value_pattern" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                // Extract just the function name (first word before parameters)
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            _ => {
                // Check if it's a lowercase identifier (OCaml value names are lowercase)
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && text.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
                    && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
                    && !matches!(text.as_str(), "let" | "rec" | "and" | "in" | "=" | "fun" | "function")
                {
                    return Some(text);
                }
            }
        }
        // Only check the first meaningful child (the pattern)
        if !matches!(child.kind(), "let" | "rec" | "comment" | "attribute") {
            break;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    extract_name_from_let_text(&text)
}

/// Get the name from a value_definition node directly (fallback).
fn get_value_def_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    // Pattern: let [rec] name ...
    let after_let = text.strip_prefix("let")?.trim_start();
    let after_rec = if after_let.starts_with("rec") {
        after_let.strip_prefix("rec")?.trim_start()
    } else {
        after_let
    };

    let name: String = after_rec
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
        .collect();

    if name.is_empty() || name == "_" || is_ocaml_keyword(&name) {
        None
    } else {
        Some(name)
    }
}

/// Extract a function/value name from let binding text.
fn extract_name_from_let_text(text: &str) -> Option<String> {
    // Skip "let" and optional "rec"
    let rest = text.strip_prefix("let").unwrap_or(text).trim_start();
    let rest = if rest.starts_with("rec") {
        rest.strip_prefix("rec").unwrap_or(rest).trim_start()
    } else {
        rest
    };

    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
        .collect();

    if name.is_empty() || name == "_" || is_ocaml_keyword(&name) {
        None
    } else {
        Some(name)
    }
}

/// Get the module name from a module_definition node.
fn get_module_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    let mut found_module_keyword = false;

    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip the "module" keyword
        if text == "module" || child.kind() == "module" {
            found_module_keyword = true;
            continue;
        }

        // Skip "type" keyword (for module type)
        if text == "type" {
            continue;
        }

        // The module name is the first uppercase identifier after "module"
        if found_module_keyword
            && child.kind() != "="
            && !text.is_empty()
            && text.chars().next().map_or(false, |c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return text;
        }

        // Also check for module_name or module_path nodes
        if matches!(child.kind(), "module_name" | "module_path" | "constructor_name") {
            if !text.is_empty() && text.chars().next().map_or(false, |c| c.is_uppercase()) {
                return text;
            }
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after_module) = text.strip_prefix("module") {
        let rest = after_module.trim_start();
        // Skip optional "type" keyword
        let rest = if rest.starts_with("type") {
            rest.strip_prefix("type").unwrap_or(rest).trim_start()
        } else {
            rest
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_uppercase()) {
            return name;
        }
    }

    String::new()
}

/// Get the module type name from a module_type_definition node.
fn get_module_type_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Module type names follow "module type" keywords
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Try AST-based extraction first
    let mut cursor = node.walk();
    let mut found_type_keyword = false;

    for child in node.children(&mut cursor) {
        let child_text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if child_text == "type" {
            found_type_keyword = true;
            continue;
        }

        if found_type_keyword
            && !child_text.is_empty()
            && child_text.chars().next().map_or(false, |c| c.is_uppercase())
            && child_text.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return child_text;
        }

        if matches!(child.kind(), "module_type_name" | "module_name") {
            if !child_text.is_empty() {
                return child_text;
            }
        }
    }

    // Fallback: parse from text
    if let Some(rest) = text.strip_prefix("module") {
        let rest = rest.trim_start();
        if let Some(rest) = rest.strip_prefix("type") {
            let rest = rest.trim_start();
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_uppercase()) {
                return name;
            }
        }
    }

    String::new()
}

/// Get the type name from a type_binding or type_definition node.
fn get_type_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    let mut found_type_keyword = false;

    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip "type" keyword and "nonrec" keyword
        if text == "type" || text == "nonrec" || text == "and" || child.kind() == "type" {
            found_type_keyword = true;
            continue;
        }

        // Skip type parameters (start with ')
        if text.starts_with('\'') || text.starts_with('(') {
            continue;
        }

        // The type name is a lowercase identifier
        if found_type_keyword
            && matches!(child.kind(), "type_constructor" | "type_variable" | "identifier" | "type_name")
        {
            if !text.is_empty()
                && text.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
                && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
            {
                return text;
            }
        }

        // Also accept lowercase identifiers directly
        if found_type_keyword
            && !text.is_empty()
            && text.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
            && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
            && !matches!(text.as_str(), "type" | "nonrec" | "and" | "of" | "mutable")
        {
            return text;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let rest = text.strip_prefix("type").unwrap_or(&text).trim_start();
    // Skip "nonrec" if present
    let rest = rest.strip_prefix("nonrec").unwrap_or(rest).trim_start();
    // Skip type parameters like 'a or ('a, 'b)
    let rest = if rest.starts_with('\'') {
        // Skip single type param
        let after_param: &str = rest
            .split_whitespace()
            .nth(1)
            .unwrap_or("");
        after_param
    } else if rest.starts_with('(') {
        // Skip parenthesized type params
        if let Some(close) = rest.find(')') {
            rest[close + 1..].trim_start()
        } else {
            rest
        }
    } else {
        rest
    };

    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
        .collect();

    if !name.is_empty()
        && name.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
        && !is_ocaml_keyword(&name)
    {
        name
    } else {
        String::new()
    }
}

/// Check if a module definition is a functor.
fn is_functor_definition(node: tree_sitter::Node, source: &[u8]) -> bool {
    let text = node.utf8_text(source).unwrap_or("");
    // Check for functor keyword
    if text.contains("functor") {
        return true;
    }
    // Check for module_parameter in children or grandchildren (module_binding)
    has_descendant_kind(node, "module_parameter")
        || has_descendant_kind(node, "functor_parameter")
}

/// Check if a node has a descendant of a given kind (up to depth 2).
fn has_descendant_kind(node: tree_sitter::Node, target_kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == target_kind {
            return true;
        }
        // Check grandchildren (e.g., module_parameter inside module_binding)
        let mut inner_cursor = child.walk();
        for grandchild in child.children(&mut inner_cursor) {
            if grandchild.kind() == target_kind {
                return true;
            }
        }
    }
    false
}

/// Get the module name from an open statement.
fn get_open_module_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip "open" keyword and "!" (for open!)
        if text == "open" || text == "!" || child.kind() == "open" {
            continue;
        }

        // Module names start with uppercase
        if !text.is_empty()
            && text.chars().next().map_or(false, |c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Some(text);
        }

        // Check for module_path nodes
        if matches!(child.kind(), "module_path" | "module_name" | "extended_module_path") {
            if !text.is_empty() && text.chars().next().map_or(false, |c| c.is_uppercase()) {
                return Some(text);
            }
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let rest = text.strip_prefix("open")?.trim_start();
    // Skip optional "!" for open!
    let rest = rest.strip_prefix('!').unwrap_or(rest).trim_start();

    let module_name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();

    if module_name.is_empty() || !module_name.chars().next().map_or(false, |c| c.is_uppercase()) {
        None
    } else {
        Some(module_name)
    }
}

/// Get the module name from an include statement.
fn get_include_module_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip "include" keyword
        if text == "include" || child.kind() == "include" {
            continue;
        }

        // Module names start with uppercase
        if !text.is_empty()
            && text.chars().next().map_or(false, |c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Some(text);
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let rest = text.strip_prefix("include")?.trim_start();

    let module_name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();

    if module_name.is_empty() || !module_name.chars().next().map_or(false, |c| c.is_uppercase()) {
        None
    } else {
        Some(module_name)
    }
}

/// Get the function name from an application node's function position.
fn get_applied_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    match node.kind() {
        "value_name" | "variable" | "identifier" => {
            node.utf8_text(source).unwrap_or("").trim().to_string()
        }
        // value_path contains module_path.value_name or just value_name
        "value_path" => {
            node.utf8_text(source).unwrap_or("").trim().to_string()
        }
        // Qualified name: Module.function
        "field_get_expression" | "dot_expression" | "module_path" => {
            node.utf8_text(source).unwrap_or("").trim().to_string()
        }
        _ => {
            // Try to get text if it looks like an identifier
            let text = node.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty()
                && text
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_lowercase() || c == '_' || c.is_uppercase())
                && text
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '\'' || c == '.')
            {
                text
            } else {
                String::new()
            }
        }
    }
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
            "let_binding" => {
                // Check if this is a top-level binding (parent is value_definition at top level)
                if let Some(name) = get_let_binding_name(parent, source) {
                    if !name.is_empty() && !is_ocaml_keyword(&name) && name != "_" {
                        // Verify this is a top-level definition (not a local let)
                        if let Some(grandparent) = parent.parent() {
                            if grandparent.kind() == "value_definition" {
                                if let Some(great_grandparent) = grandparent.parent() {
                                    // Top-level if parent is compilation_unit, structure, or similar
                                    if matches!(
                                        great_grandparent.kind(),
                                        "compilation_unit" | "structure" | "structure_item"
                                    ) || great_grandparent.parent().is_none()
                                    {
                                        return Some(format!("{file}::{name}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "value_definition" => {
                if let Some(name) = get_value_def_name(parent, source) {
                    if !name.is_empty() && !is_ocaml_keyword(&name) && name != "_" {
                        // Check if this is top-level
                        if let Some(grandparent) = parent.parent() {
                            if matches!(
                                grandparent.kind(),
                                "compilation_unit" | "structure" | "structure_item"
                            ) || grandparent.parent().is_none()
                            {
                                return Some(format!("{file}::{name}"));
                            }
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

/// Check if the node's text contains a specific keyword.
fn node_text_contains(node: tree_sitter::Node, source: &[u8], keyword: &str) -> bool {
    let text = node.utf8_text(source).unwrap_or("");
    // Check direct children for the keyword
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_text = child.utf8_text(source).unwrap_or("").trim();
        if child_text == keyword {
            return true;
        }
    }
    // Fallback: check if text starts with "let rec"
    text.trim_start().starts_with(&format!("let {keyword}"))
}

/// Check if a name is an OCaml keyword that should not be treated as a function/value.
fn is_ocaml_keyword(name: &str) -> bool {
    matches!(
        name,
        "let"
            | "rec"
            | "in"
            | "and"
            | "if"
            | "then"
            | "else"
            | "match"
            | "with"
            | "function"
            | "fun"
            | "type"
            | "module"
            | "struct"
            | "sig"
            | "end"
            | "open"
            | "include"
            | "val"
            | "external"
            | "exception"
            | "class"
            | "object"
            | "method"
            | "inherit"
            | "initializer"
            | "constraint"
            | "virtual"
            | "private"
            | "mutable"
            | "nonrec"
            | "of"
            | "begin"
            | "do"
            | "done"
            | "while"
            | "for"
            | "to"
            | "downto"
            | "true"
            | "false"
            | "assert"
            | "lazy"
            | "try"
            | "raise"
            | "when"
            | "as"
            | "new"
            | "functor"
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse OCaml source and run the extractor.
    fn parse_ocaml(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .expect("OCaml grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, "lib/server.ml", source)
    }

    #[test]
    fn test_let_bindings() {
        let source = r#"
let validate_user user =
  user.name <> "" && user.age > 0

let rec fibonacci n =
  if n <= 1 then n
  else fibonacci (n - 1) + fibonacci (n - 2)

let main () =
  print_endline "Hello, OCaml!"
"#;
        let result = parse_ocaml(source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::validate_user" && n.kind == NodeKind::Function),
            "Should find validate_user function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::fibonacci" && n.kind == NodeKind::Function),
            "Should find fibonacci function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::main" && n.kind == NodeKind::Function),
            "Should find main function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );

        // Check recursive attribute
        let fib = result.nodes.iter().find(|n| n.fqn.ends_with("::fibonacci")).unwrap();
        assert_eq!(
            fib.attributes.get("recursive").and_then(|v| v.as_bool()),
            Some(true),
            "fibonacci should be marked as recursive"
        );
    }

    #[test]
    fn test_module_extraction() {
        let source = r#"
module Config = struct
  let default_port = 8080
  let max_connections = 100
end

module type STORAGE = sig
  val get : string -> string option
  val set : string -> string -> unit
end
"#;
        let result = parse_ocaml(source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::Config" && n.kind == NodeKind::Module),
            "Should find Config module. Nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::STORAGE" && n.kind == NodeKind::Module),
            "Should find STORAGE module type. Nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_type_definitions() {
        let source = r#"
type color = Red | Green | Blue

type user = {
  name: string;
  age: int;
}

type id = int
"#;
        let result = parse_ocaml(source);

        // Variant type → Class
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::color" && n.kind == NodeKind::Class),
            "Should find color as Class (variant type). Nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );

        // Record type → Class
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::user" && n.kind == NodeKind::Class),
            "Should find user as Class (record type). Nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );

        // Type alias → TypeAlias
        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/server.ml::id" && n.kind == NodeKind::TypeAlias),
            "Should find id as TypeAlias. Nodes: {:?}",
            result.nodes.iter().map(|n| (&n.fqn, &n.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_open_imports() {
        let source = r#"
open Printf
open Lwt.Infix
open! Core
"#;
        let result = parse_ocaml(source);

        let imports: Vec<&Edge> = result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "Printf"),
            "Should find Printf import. Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Lwt.Infix"),
            "Should find Lwt.Infix import. Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Core"),
            "Should find Core import (open!). Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_include_imports() {
        let source = r#"
include Base
include Sexplib.Std
"#;
        let result = parse_ocaml(source);

        let imports: Vec<&Edge> = result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "Base" && e.attributes.get("include").and_then(|v| v.as_bool()) == Some(true)),
            "Should find Base include import. Imports: {:?}",
            imports.iter().map(|e| (&e.target_fqn, &e.attributes)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_functor_detection() {
        let source = r#"
module Make (S : STORAGE) = struct
  let create () = S.get "key"
end
"#;
        let result = parse_ocaml(source);

        let make_module = result.nodes.iter().find(|n| n.fqn == "lib/server.ml::Make");
        assert!(
            make_module.is_some(),
            "Should find Make module. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );

        if let Some(m) = make_module {
            assert_eq!(m.kind, NodeKind::Module);
            assert_eq!(
                m.attributes.get("functor").and_then(|v| v.as_bool()),
                Some(true),
                "Make should be marked as a functor"
            );
        }
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
let helper x = x + 1

let main () =
  let result = helper 42 in
  print_int result
"#;
        let result = parse_ocaml(source);

        let calls: Vec<&Edge> = result.edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();

        // main should call helper
        assert!(
            calls.iter().any(|e| e.source_fqn.ends_with("::main") && e.target_fqn.ends_with("::helper")),
            "main should call helper. Calls found: {:?}",
            calls.iter().map(|e| format!("{} -> {}", e.source_fqn, e.target_fqn)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_ocaml("");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_combined_extraction() {
        let source = r#"
open Printf
open Lwt.Infix

module Config = struct
  let default_port = 8080
  let max_connections = 100
end

type user = {
  name: string;
  age: int;
}

type color = Red | Green | Blue

type id = int

let validate_user user =
  user.name <> "" && user.age > 0

let rec fibonacci n =
  if n <= 1 then n
  else fibonacci (n - 1) + fibonacci (n - 2)

let main () =
  printf "Hello, OCaml!\n"
"#;
        let result = parse_ocaml(source);

        // Check imports
        let imports: Vec<&Edge> = result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).collect();
        assert!(imports.iter().any(|e| e.target_fqn == "Printf"));
        assert!(imports.iter().any(|e| e.target_fqn == "Lwt.Infix"));

        // Check module
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::Config" && n.kind == NodeKind::Module));

        // Check types
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::user" && n.kind == NodeKind::Class));
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::color" && n.kind == NodeKind::Class));
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::id" && n.kind == NodeKind::TypeAlias));

        // Check functions
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::validate_user" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::fibonacci" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "lib/server.ml::main" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_complexity_computed() {
        let source = r#"
let complex x =
  if x > 0 then
    match x with
    | 1 -> "one"
    | 2 -> "two"
    | _ -> "other"
  else "negative"
"#;
        let result = parse_ocaml(source);

        let func = result.nodes.iter().find(|n| n.fqn.ends_with("::complex"));
        assert!(func.is_some(), "Should find complex function");
        if let Some(f) = func {
            let complexity = f.attributes.get("complexity");
            assert!(
                complexity.is_some(),
                "Should have complexity attribute computed"
            );
            if let Some(c) = complexity {
                assert!(
                    c.as_u64().unwrap_or(0) >= 1,
                    "Complexity should be at least 1"
                );
            }
        }
    }

    #[test]
    fn test_extract_regex_backward_compat() {
        let source = r#"
let greet name = "Hello, " ^ name

let main () =
  print_endline (greet "World")
"#;
        #[allow(deprecated)]
        let result = extract_regex("lib/main.ml", source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "lib/main.ml::greet" && n.kind == NodeKind::Function),
            "extract_regex should still work for backward compatibility. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_multiple_let_and() {
        let source = r#"
let even n = n mod 2 = 0
and odd n = n mod 2 = 1
"#;
        let result = parse_ocaml(source);

        assert!(
            result.nodes.iter().any(|n| n.fqn.ends_with("::even") && n.kind == NodeKind::Function),
            "Should find even function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn.ends_with("::odd") && n.kind == NodeKind::Function),
            "Should find odd function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }
}
