//! Dart AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (classes, mixins, enums, functions, extensions,
//! constants, type aliases) and edges (imports, calls, inheritance, implements)
//! from a tree-sitter Dart parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Dart file.
///
/// Handles:
/// - Class definitions (regular, abstract)
/// - Mixin declarations (NodeKind::Trait)
/// - Enum declarations
/// - Function/method definitions (top-level, class methods, constructors)
/// - Extension declarations (NodeKind::Module)
/// - Top-level constants (const, final)
/// - Type alias declarations (typedef)
/// - Import declarations
/// - Intra-file call expressions resolved to definitions in the same file
/// - Inheritance (extends) and implements edges
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
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_dart::LANGUAGE.into())
        .expect("Dart grammar should load");
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
        if node.kind == NodeKind::Function
            && let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_signature")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "method_signature"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "function_body"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "method_definition"))
        {
            let c = complexity::compute_full_complexity(ast_node, source, "dart");
            if let Some(attrs) = node.attributes.as_object_mut() {
                attrs.insert("complexity".to_string(), serde_json::json!(c));
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
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "class_declaration" => {
                extract_class(child, file, source, nodes, defined_fqns, edges);
            }
            "mixin_declaration" => {
                extract_mixin(child, file, source, nodes, defined_fqns, edges);
            }
            "enum_declaration" => {
                extract_enum(child, file, source, nodes, defined_fqns);
            }
            "extension_declaration" => {
                extract_extension(child, file, source, nodes, defined_fqns, edges);
            }
            "function_signature" | "method_signature" => {
                extract_function(child, file, source, parent_name, nodes, defined_fqns);
            }
            "function_definition" | "method_definition" | "function_declaration" => {
                extract_function(child, file, source, parent_name, nodes, defined_fqns);
            }
            "constructor_signature" => {
                extract_constructor(child, file, source, parent_name, nodes, defined_fqns);
            }
            "type_alias" => {
                extract_type_alias(child, file, source, parent_name, nodes, defined_fqns);
            }
            // Top-level constants: const or final declarations
            "top_level_definition" | "top_level_variable_declaration" => {
                // Recurse into top-level definitions to find static_final_declaration
                if parent_name.is_none() {
                    extract_top_level_constant(child, file, source, nodes, defined_fqns);
                }
            }
            "initialized_variable_definition" | "static_final_declaration" => {
                if parent_name.is_none() {
                    extract_constant(child, file, source, nodes, defined_fqns);
                }
            }
            "final_builtin" | "const_builtin" => {
                // These are keywords, skip
            }
            "program" | "source_file" => {
                collect_definitions(child, file, source, parent_name, nodes, defined_fqns, edges);
            }
            _ => {
                if child.child_count() > 0 && !is_leaf_kind(child.kind()) {
                    collect_definitions(
                        child,
                        file,
                        source,
                        parent_name,
                        nodes,
                        defined_fqns,
                        edges,
                    );
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
            | "type_identifier"
            | "string_literal"
            | "integer_literal"
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "decimal_floating_point_literal"
            | "true"
            | "false"
            | "null_literal"
            | "comment"
            | "documentation_comment"
            | "import_or_export"
            | "import_specification"
            | "library_import"
            | "class_body"
            | "extension_body"
            | "mixin_body"
            | "function_body"
            | "block"
    )
}

/// Extract a class definition.
fn extract_class(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_identifier_child(node, source);
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

    // Extract inheritance (extends) and implements edges
    extract_superclass(node, source, &fqn, edges);
    extract_interfaces(node, source, &fqn, edges);
    extract_mixins_clause(node, source, &fqn, edges);

    // Recurse into class body for methods - walk all descendants
    if let Some(body) = find_child_by_kind(node, "class_body") {
        collect_class_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a mixin declaration (uses NodeKind::Trait per design).
fn extract_mixin(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_identifier_child(node, source);
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
        attributes: json!({"mixin": true}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Mixins can have `on` clause and `implements`
    extract_interfaces(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) =
        find_child_by_kind(node, "class_body").or_else(|| find_child_by_kind(node, "mixin_body"))
    {
        collect_class_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract an enum declaration.
fn extract_enum(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_identifier_child(node, source);
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

/// Extract an extension declaration (uses NodeKind::Module).
fn extract_extension(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    // Extension name: `extension Name on Type`
    let name = get_extension_name(node, source);
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"extension": true}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "extension_body")
        .or_else(|| find_child_by_kind(node, "class_body"))
    {
        collect_class_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a function or method definition.
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // For function_declaration/method_declaration, look inside the signature child
    let sig_node = if node.kind() == "function_declaration" || node.kind() == "method_declaration" {
        find_child_by_kind(node, "function_signature")
            .or_else(|| find_child_by_kind(node, "method_signature"))
            .unwrap_or(node)
    } else {
        node
    };

    let name = get_function_name(sig_node, source);
    if name.is_empty() {
        return;
    }

    let fqn = match parent_name {
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

/// Extract a constructor.
fn extract_constructor(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // Constructor name is typically the class name or a named constructor like ClassName.named
    let name = get_constructor_name(node, source, parent_name);
    if name.is_empty() {
        return;
    }

    let fqn = match parent_name {
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
        attributes: json!({"constructor": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract a type alias (typedef).
fn extract_type_alias(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_alias_name(node, source);
    if name.is_empty() {
        return;
    }

    let fqn = match parent_name {
        Some(cls) => format!("{file}::{cls}::{name}"),
        None => format!("{file}::{name}"),
    };
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::TypeAlias,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract a top-level constant (const or final).
fn extract_constant(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_constant_name(node, source);
    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

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

/// Extract constants from a top_level_variable_declaration node.
/// Structure: top_level_variable_declaration > const/final + static_final_declaration_list > static_final_declaration > identifier
fn extract_top_level_constant(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // Find static_final_declaration children (possibly nested in static_final_declaration_list)
    let mut found = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "static_final_declaration_list" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "static_final_declaration" {
                    extract_constant(inner, file, source, nodes, defined_fqns);
                    found = true;
                }
            }
        } else if child.kind() == "static_final_declaration" {
            extract_constant(child, file, source, nodes, defined_fqns);
            found = true;
        }
    }

    // If we didn't find structured declarations, try text-based extraction
    if !found {
        extract_constant(node, file, source, nodes, defined_fqns);
    }
}

/// Collect members (methods, constructors, type aliases) inside a class/mixin/extension body.
/// Uses a recursive descent to find method_signature and constructor_signature nodes
/// regardless of how deeply they're nested in wrapper nodes.
fn collect_class_members(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    _edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_signature" | "function_signature" | "getter_signature" | "setter_signature" => {
                extract_function(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "constructor_signature" => {
                extract_constructor(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "method_declaration" | "function_declaration" => {
                // These wrap a signature + body. Extract from the signature.
                if let Some(sig) = find_child_by_kind(child, "method_signature")
                    .or_else(|| find_child_by_kind(child, "function_signature"))
                {
                    extract_function(sig, file, source, Some(parent_name), nodes, defined_fqns);
                }
            }
            // Don't recurse into nested class/mixin/enum bodies
            "class_declaration"
            | "class_definition"
            | "mixin_declaration"
            | "enum_declaration"
            | "extension_declaration" => {}
            // Don't recurse into function bodies (we only want signatures)
            "function_body" | "block" => {}
            _ => {
                if child.child_count() > 0 {
                    collect_class_members(
                        child,
                        file,
                        source,
                        parent_name,
                        nodes,
                        defined_fqns,
                        _edges,
                    );
                }
            }
        }
    }
}

/// Extract the superclass from an `extends` clause and emit an Inherits edge.
fn extract_superclass(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "superclass" {
            // The superclass node contains a type_identifier
            let type_name = get_type_from_clause(child, source);
            if !type_name.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.to_string(),
                    target_fqn: type_name,
                    kind: EdgeKind::Inherits,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({}),
                });
            }
            return;
        }
    }

    // Fallback: look for extends keyword followed by type
    let text = node.utf8_text(source).unwrap_or("");
    if let Some(extends_pos) = text.find("extends ") {
        let after_extends = &text[extends_pos + 8..];
        let type_name: String = after_extends
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !type_name.is_empty() {
            edges.push(Edge {
                id: None,
                source_fqn: source_fqn.to_string(),
                target_fqn: type_name,
                kind: EdgeKind::Inherits,
                confidence: 1.0,
                edge_source: crate::store::confidence::EdgeSource::AstDirect,
                attributes: json!({}),
            });
        }
    }
}

/// Extract interfaces from an `implements` clause and emit Implements edges.
fn extract_interfaces(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interfaces" {
            extract_type_list(child, source, source_fqn, EdgeKind::Implements, edges);
            return;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("");
    if let Some(impl_pos) = text.find("implements ") {
        let after_impl = &text[impl_pos + 11..];
        // Take until '{' or 'with' or end
        let clause: String = after_impl
            .chars()
            .take_while(|c| *c != '{' && *c != ';')
            .collect();
        for type_name in clause.split(',') {
            let name = type_name.trim();
            let name: String = name
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && name != "with" {
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.to_string(),
                    target_fqn: name,
                    kind: EdgeKind::Implements,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({}),
                });
            }
        }
    }
}

/// Extract mixins from a `with` clause and emit Inherits edges.
fn extract_mixins_clause(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "mixins" {
            extract_type_list(child, source, source_fqn, EdgeKind::Inherits, edges);
            return;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("");
    if let Some(with_pos) = text.find(" with ") {
        let after_with = &text[with_pos + 6..];
        // Take until '{' or 'implements' or end
        let clause: String = after_with
            .chars()
            .take_while(|c| *c != '{' && *c != ';')
            .collect();
        // Stop at "implements"
        let clause = clause.split("implements").next().unwrap_or("");
        for type_name in clause.split(',') {
            let name = type_name.trim();
            let name: String = name
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.to_string(),
                    target_fqn: name,
                    kind: EdgeKind::Inherits,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({}),
                });
            }
        }
    }
}

/// Extract type names from a type list node (interfaces, mixins).
fn extract_type_list(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edge_kind: EdgeKind,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !name.is_empty() {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.to_string(),
                        target_fqn: name,
                        kind: edge_kind.clone(),
                        confidence: 1.0,
                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                        attributes: json!({}),
                    });
                }
            }
            _ => {
                // Recurse to find type identifiers
                if child.child_count() > 0 {
                    extract_type_list(child, source, source_fqn, edge_kind.clone(), edges);
                }
            }
        }
    }
}

/// Get the type name from a superclass or type clause.
fn get_type_from_clause(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" || child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("").trim().to_string();
        }
        // Recurse into type nodes
        if child.child_count() > 0 {
            let result = get_type_from_clause(child, source);
            if !result.is_empty() {
                return result;
            }
        }
    }
    String::new()
}

/// Collect import declarations.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_or_export" | "import_specification" | "library_import" => {
                extract_import(child, file, source, edges);
            }
            _ => {
                if child.child_count() > 0 {
                    collect_imports(child, file, source, edges);
                }
            }
        }
    }
}

/// Extract an import declaration into Imports edges.
fn extract_import(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if text.is_empty() || !text.starts_with("import") {
        return;
    }

    // Extract the import path from quotes: import 'package:...' or import "..."
    if let Some(target) = extract_string_literal(&text) {
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target,
            kind: EdgeKind::Imports,
            confidence: 1.0,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({}),
        });
    }
}

/// Extract a string literal value from an import statement.
fn extract_string_literal(text: &str) -> Option<String> {
    // Find the first quoted string
    let start_single = text.find('\'');
    let start_double = text.find('"');

    let (start, quote_char) = match (start_single, start_double) {
        (Some(s), Some(d)) => {
            if s < d {
                (s, '\'')
            } else {
                (d, '"')
            }
        }
        (Some(s), None) => (s, '\''),
        (None, Some(d)) => (d, '"'),
        (None, None) => return None,
    };

    let after_quote = &text[start + 1..];
    if let Some(end) = after_quote.find(quote_char) {
        let path = &after_quote[..end];
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
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
        // Look for various call expression patterns in Dart
        match child.kind() {
            "call_expression" | "function_expression_invocation" => {
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

/// Extract a single call edge from a call expression node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    // The function being called is typically the first child
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.child(0));

    if let Some(func) = func_node {
        let call_text = func.utf8_text(source).unwrap_or("").trim();

        // Only resolve simple identifier calls (not method calls like obj.method())
        if func.kind() == "identifier"
            && let Some((_, target_fqn)) = defined_fqns.iter().find(|(name, _)| name == call_text)
        {
            let caller_fqn = find_enclosing_function(node, file, source);
            if let Some(caller) = caller_fqn
                && caller != *target_fqn
            {
                edges.push(Edge {
                    id: None,
                    source_fqn: caller,
                    target_fqn: target_fqn.clone(),
                    kind: EdgeKind::Calls,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({}),
                });
            }
        }
    }
}

/// Find the enclosing function/method for a given node to determine the caller FQN.
fn find_enclosing_function(node: tree_sitter::Node, file: &str, source: &[u8]) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "function_signature"
            | "method_signature"
            | "function_definition"
            | "method_definition" => {
                let name = get_function_name(parent, source);
                if !name.is_empty() {
                    // Check if this function is inside a class/mixin/extension
                    let mut ancestor = parent.parent();
                    while let Some(anc) = ancestor {
                        match anc.kind() {
                            "class_definition"
                            | "class_declaration"
                            | "mixin_declaration"
                            | "extension_declaration" => {
                                let cls = get_identifier_child(anc, source);
                                if cls.is_empty() {
                                    let cls = get_extension_name(anc, source);
                                    if !cls.is_empty() {
                                        return Some(format!("{file}::{cls}::{name}"));
                                    }
                                } else {
                                    return Some(format!("{file}::{cls}::{name}"));
                                }
                                break;
                            }
                            "class_body" | "extension_body" | "mixin_body" | "class_member"
                            | "method_declaration" => {}
                            _ => {}
                        }
                        ancestor = anc.parent();
                    }
                    return Some(format!("{file}::{name}"));
                }
            }
            "method_declaration" => {
                // method_declaration wraps method_signature + function_body
                if let Some(sig) = find_child_by_kind(parent, "method_signature") {
                    let name = get_function_name(sig, source);
                    if !name.is_empty() {
                        let mut ancestor = parent.parent();
                        while let Some(anc) = ancestor {
                            match anc.kind() {
                                "class_definition"
                                | "class_declaration"
                                | "mixin_declaration"
                                | "extension_declaration" => {
                                    let cls = get_identifier_child(anc, source);
                                    if cls.is_empty() {
                                        let cls = get_extension_name(anc, source);
                                        if !cls.is_empty() {
                                            return Some(format!("{file}::{cls}::{name}"));
                                        }
                                    } else {
                                        return Some(format!("{file}::{cls}::{name}"));
                                    }
                                    break;
                                }
                                "class_body" | "extension_body" | "mixin_body" | "class_member" => {
                                }
                                _ => {}
                            }
                            ancestor = anc.parent();
                        }
                        return Some(format!("{file}::{name}"));
                    }
                }
            }
            "constructor_signature" => {
                // Inside a constructor
                let mut ancestor = parent.parent();
                while let Some(anc) = ancestor {
                    if anc.kind() == "class_definition" || anc.kind() == "class_declaration" {
                        let cls = get_identifier_child(anc, source);
                        if !cls.is_empty() {
                            return Some(format!("{file}::{cls}::{cls}"));
                        }
                        break;
                    }
                    ancestor = anc.parent();
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    // Call at file level
    Some(file.to_string())
}

// ─── Helper functions ───────────────────────────────────────────────────────

/// Get the first identifier child of a node (for class/mixin/enum names).
fn get_identifier_child(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }

    // Fallback: find first identifier or type_identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty()
                && text != "abstract"
                && text != "class"
                && text != "mixin"
                && text != "enum"
                && text != "extension"
            {
                return text;
            }
        }
    }

    String::new()
}

/// Get the name of a function/method from its node.
fn get_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }

    // For method_signature: it may contain function_signature which has the identifier
    // For function_signature: it directly has the identifier
    // Walk children looking for identifier, also recurse into function_signature
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty()
                && text != "static"
                && text != "async"
                && text != "abstract"
                && text != "external"
                && text != "override"
            {
                return text;
            }
        }
        // Recurse into function_signature to find the identifier
        if child.kind() == "function_signature" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "identifier" {
                    let text = inner.utf8_text(source).unwrap_or("").trim().to_string();
                    if !text.is_empty()
                        && text != "static"
                        && text != "async"
                        && text != "abstract"
                        && text != "external"
                        && text != "override"
                    {
                        return text;
                    }
                }
            }
        }
    }

    String::new()
}

/// Get the name of a constructor.
fn get_constructor_name(
    node: tree_sitter::Node,
    source: &[u8],
    parent_name: Option<&str>,
) -> String {
    // Try to find the constructor name (could be ClassName or ClassName.named)
    // Look for identifier children
    let mut cursor = node.walk();
    let mut names: Vec<String> = Vec::new();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            let name = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !name.is_empty() && name != "const" && name != "factory" && name != "external" {
                names.push(name);
            }
        }
    }

    if names.is_empty() {
        // Use parent name as constructor name
        return parent_name.unwrap_or("").to_string();
    }

    // If there's a dot-separated name (named constructor), use the part after the dot
    if names.len() >= 2 {
        // e.g., ClassName.named -> "named"
        return names[1].clone();
    }

    // Single name - it's the class name constructor
    if let Some(parent) = parent_name
        && names[0] == parent
    {
        return parent.to_string();
    }

    names[0].clone()
}

/// Get the name of an extension.
fn get_extension_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Extension can be: `extension Name on Type` or `extension on Type` (unnamed)
    let mut cursor = node.walk();
    let mut found_extension_keyword = false;

    for child in node.children(&mut cursor) {
        if child.kind() == "extension" || child.utf8_text(source).unwrap_or("") == "extension" {
            found_extension_keyword = true;
            continue;
        }
        if found_extension_keyword
            && (child.kind() == "identifier" || child.kind() == "type_identifier")
        {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if text != "on" && !text.is_empty() {
                return text;
            }
        }
    }

    // Fallback: try to get name from text
    let text = node.utf8_text(source).unwrap_or("");
    if let Some(rest) = text.strip_prefix("extension") {
        let rest = rest.trim();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && name != "on" {
            return name;
        }
    }

    // Unnamed extension - use a generated name
    let line = node.start_position().row + 1;
    format!("_extension_L{}", line)
}

/// Get the name of a type alias.
fn get_type_alias_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // type_alias structure: typedef + type_identifier (name) + = + type
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    // Fallback: look for identifier after typedef keyword
    let mut cursor2 = node.walk();
    let mut found_typedef = false;
    for child in node.children(&mut cursor2) {
        let text = child.utf8_text(source).unwrap_or("").trim();
        if text == "typedef" || child.kind() == "typedef" {
            found_typedef = true;
            continue;
        }
        if found_typedef && child.kind() == "identifier" && !text.is_empty() {
            return text.to_string();
        }
    }

    // Last resort: parse from text
    let text = node.utf8_text(source).unwrap_or("");
    if let Some(rest) = text.strip_prefix("typedef") {
        let rest = rest.trim();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return name;
        }
    }

    String::new()
}

/// Get the name of a constant from a variable declaration.
fn get_constant_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // For static_final_declaration: identifier is a direct child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim();
    let rest = text
        .strip_prefix("const ")
        .or_else(|| text.strip_prefix("final "))
        .unwrap_or(text);

    // The name is the identifier before '=' or ';'
    let parts: Vec<&str> = rest.splitn(2, '=').collect();
    let decl = parts[0].trim();

    // Last word before '=' is the variable name
    let words: Vec<&str> = decl.split_whitespace().collect();
    if let Some(last) = words.last() {
        let name: String = last
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return name;
        }
    }

    String::new()
}

/// Find the first child of a given kind.
fn find_child_by_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|&child| child.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Dart source and run extraction.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_dart::LANGUAGE.into())
            .expect("Dart grammar should load");
        let tree = parser.parse(source, None).expect("Should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_classes_with_methods_and_constructors() {
        let source = r#"
class MyWidget extends StatelessWidget {
  final String title;

  MyWidget(this.title);

  MyWidget.named(String t) : title = t;

  void build() {
    print("Building $title");
  }

  Future<void> fetchData() async {
    await Future.delayed(Duration(seconds: 1));
  }
}

abstract class BaseWidget {
  void build();
}
"#;
        let result = parse_and_extract("lib/widget.dart", source);

        // Check classes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/widget.dart::MyWidget" && n.kind == NodeKind::Class),
            "Should find MyWidget class. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/widget.dart::BaseWidget" && n.kind == NodeKind::Class),
            "Should find BaseWidget class"
        );

        // Check methods
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "lib/widget.dart::MyWidget::build" && n.kind == NodeKind::Function
            ),
            "Should find build method. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );

        // Check inheritance
        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits
                .iter()
                .any(|e| e.source_fqn == "lib/widget.dart::MyWidget"
                    && e.target_fqn == "StatelessWidget"),
            "Should have extends edge. Inherits edges: {:?}",
            inherits
        );
    }

    #[test]
    fn test_mixins() {
        let source = r#"
mixin Logging {
  void log(String msg) {
    print(msg);
  }
}

mixin Serializable {
  String serialize();
}
"#;
        let result = parse_and_extract("lib/mixins.dart", source);

        // Mixins use NodeKind::Trait
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/mixins.dart::Logging" && n.kind == NodeKind::Trait),
            "Should find Logging mixin as Trait. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/mixins.dart::Serializable" && n.kind == NodeKind::Trait),
            "Should find Serializable mixin as Trait"
        );

        // Methods inside mixin
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/mixins.dart::Logging::log" && n.kind == NodeKind::Function),
            "Should find log method in Logging mixin. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_enums() {
        let source = r#"
enum Color {
  red,
  green,
  blue
}

enum Status {
  active,
  inactive,
  pending
}
"#;
        let result = parse_and_extract("lib/enums.dart", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/enums.dart::Color" && n.kind == NodeKind::Enum),
            "Should find Color enum. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/enums.dart::Status" && n.kind == NodeKind::Enum),
            "Should find Status enum"
        );
    }

    #[test]
    fn test_extensions() {
        let source = r#"
extension StringExt on String {
  String capitalize() {
    return '${this[0].toUpperCase()}${substring(1)}';
  }
}

extension NumberParsing on String {
  int parseInt() {
    return int.parse(this);
  }
}
"#;
        let result = parse_and_extract("lib/extensions.dart", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/extensions.dart::StringExt" && n.kind == NodeKind::Module),
            "Should find StringExt extension as Module. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "lib/extensions.dart::NumberParsing" && n.kind == NodeKind::Module
            ),
            "Should find NumberParsing extension as Module"
        );
    }

    #[test]
    fn test_imports() {
        let source = r#"
import 'package:flutter/material.dart';
import 'dart:async';
import 'package:http/http.dart' as http;
"#;
        let result = parse_and_extract("lib/app.dart", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "package:flutter/material.dart"),
            "Should import flutter/material.dart. Imports: {:?}",
            imports
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "dart:async"),
            "Should import dart:async. Imports: {:?}",
            imports
        );
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "package:http/http.dart"),
            "Should import http/http.dart. Imports: {:?}",
            imports
        );
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
void helper() {
  print("helping");
}

void main() {
  helper();
}
"#;
        let result = parse_and_extract("lib/main.dart", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn.contains("main") && e.target_fqn.contains("helper")),
            "Should have call from main to helper. Calls: {:?}",
            calls
        );
    }

    #[test]
    fn test_inheritance_and_implements() {
        let source = r#"
abstract class Animal {
  void speak();
}

abstract class Movable {
  void move();
}

class Dog extends Animal implements Movable {
  void speak() => print("Woof");
  void move() => print("Run");
}
"#;
        let result = parse_and_extract("lib/animals.dart", source);

        // Check inheritance
        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            inherits
                .iter()
                .any(|e| e.source_fqn == "lib/animals.dart::Dog" && e.target_fqn == "Animal"),
            "Dog should extend Animal. Inherits: {:?}",
            inherits
        );

        // Check implements
        let implements: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();
        assert!(
            implements
                .iter()
                .any(|e| e.source_fqn == "lib/animals.dart::Dog" && e.target_fqn == "Movable"),
            "Dog should implement Movable. Implements: {:?}",
            implements
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_and_extract("empty.dart", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_top_level_functions() {
        let source = r#"
void main() {
  runApp(MyApp());
}

Future<String> fetchData(String url) async {
  return '';
}

int add(int a, int b) => a + b;
"#;
        let result = parse_and_extract("lib/main.dart", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/main.dart::main" && n.kind == NodeKind::Function),
            "Should find main function. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/main.dart::fetchData" && n.kind == NodeKind::Function),
            "Should find fetchData function. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/main.dart::add" && n.kind == NodeKind::Function),
            "Should find add function. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }
}
