//! Swift AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (classes, structs, protocols, enums, functions,
//! extensions, constants, type aliases) and edges (imports, calls, inheritance)
//! from a tree-sitter Swift parse tree.
//!
//! Note: The tree-sitter-swift grammar uses `class_declaration` for classes,
//! structs, enums, AND extensions - differentiated by the first keyword child
//! (`class`, `struct`, `enum`, `extension`). This extractor inspects the keyword
//! to determine the actual construct type.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Swift file.
///
/// Handles:
/// - Class declarations (regular, final)
/// - Struct declarations
/// - Protocol declarations
/// - Enum declarations
/// - Function/method declarations (`func`)
/// - Init/deinit declarations
/// - Extension declarations (treated as Module)
/// - Top-level let/var constants
/// - Type alias declarations
/// - Import declarations
/// - Intra-file call expressions resolved to definitions in the same file
/// - Inheritance (`:` clause) edges
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
pub fn extract_swift(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .expect("Swift grammar should load");
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
                find_ast_node_at_line(root, node.start_line, "function_declaration").or_else(|| {
                    find_ast_node_at_line(root, node.start_line, "protocol_function_declaration")
                })
        {
            let c = complexity::compute_full_complexity(ast_node, source, "swift");
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

/// Determine what kind of declaration a `class_declaration` node actually is
/// by inspecting its first keyword child.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SwiftDeclKind {
    Class,
    Struct,
    Enum,
    Extension,
}

/// Inspect a `class_declaration` node to determine its actual kind.
fn classify_class_declaration(node: tree_sitter::Node, source: &[u8]) -> SwiftDeclKind {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct" => return SwiftDeclKind::Struct,
            "enum" => return SwiftDeclKind::Enum,
            "extension" => return SwiftDeclKind::Extension,
            "class" => return SwiftDeclKind::Class,
            // Skip modifiers like public, final, etc.
            "modifiers" | "attribute" => continue,
            // If we hit the type_identifier before finding a keyword, check text
            "type_identifier" | "user_type" => break,
            _ => {
                // Check if this child's text is a keyword
                let text = child.utf8_text(source).unwrap_or("");
                match text {
                    "struct" => return SwiftDeclKind::Struct,
                    "enum" => return SwiftDeclKind::Enum,
                    "extension" => return SwiftDeclKind::Extension,
                    "class" => return SwiftDeclKind::Class,
                    _ => continue,
                }
            }
        }
    }
    SwiftDeclKind::Class
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
            "class_declaration" => {
                // In tree-sitter-swift, class_declaration is used for class, struct, enum, extension
                let decl_kind = classify_class_declaration(child, source);
                match decl_kind {
                    SwiftDeclKind::Class => {
                        extract_class(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Struct => {
                        extract_struct(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Enum => {
                        extract_enum(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Extension => {
                        extract_extension(child, file, source, nodes, defined_fqns, edges);
                    }
                }
            }
            "protocol_declaration" => {
                extract_protocol(child, file, source, nodes, defined_fqns, edges);
            }
            "function_declaration" => {
                extract_function(child, file, source, parent_name, nodes, defined_fqns);
            }
            "protocol_function_declaration" => {
                extract_function(child, file, source, parent_name, nodes, defined_fqns);
            }
            "init_declaration" => {
                extract_init(child, file, source, parent_name, nodes, defined_fqns);
            }
            "deinit_declaration" => {
                extract_deinit(child, file, source, parent_name, nodes, defined_fqns);
            }
            "property_declaration" => {
                // Only extract top-level let/var as constants
                if parent_name.is_none() {
                    extract_property(child, file, source, nodes, defined_fqns);
                }
            }
            "typealias_declaration" => {
                extract_typealias(child, file, source, parent_name, nodes, defined_fqns);
            }
            // Recurse into source_file and other containers
            "source_file" => {
                collect_definitions(child, file, source, parent_name, nodes, defined_fqns, edges);
            }
            _ => {
                // Recurse into other containers (but not leaf nodes)
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
        "simple_identifier"
            | "integer_literal"
            | "real_literal"
            | "boolean_literal"
            | "string_literal"
            | "nil_literal"
            | "comment"
            | "multiline_comment"
            | "type_identifier"
            | "import_declaration"
            | "call_expression"
            | "navigation_expression"
            | "enum_entry"
    )
}

/// Extract a class declaration.
fn extract_class(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_type_identifier(node, source);
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

    // Check for inheritance
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "class_body") {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a struct declaration (uses NodeKind::Class per design).
fn extract_struct(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_type_identifier(node, source);
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
        attributes: json!({"struct": true}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Check for inheritance/protocol conformance
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "class_body") {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a protocol declaration.
fn extract_protocol(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_type_identifier(node, source);
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

    // Check for protocol inheritance
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for required methods
    if let Some(body) = find_child_by_kind(node, "protocol_body") {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract an enum declaration.
fn extract_enum(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_type_identifier(node, source);
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

    defined_fqns.push((name.clone(), fqn.clone()));

    // Check for inheritance/raw value type
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods (enum uses enum_class_body)
    if let Some(body) = find_child_by_kind(node, "enum_class_body") {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a function declaration (`func`).
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_function_name(node, source);
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

/// Extract an init declaration.
fn extract_init(
    node: tree_sitter::Node,
    file: &str,
    _source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = "init".to_string();

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
        attributes: json!({"initializer": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract a deinit declaration.
fn extract_deinit(
    node: tree_sitter::Node,
    file: &str,
    _source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = "deinit".to_string();

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
        attributes: json!({"deinitializer": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract an extension declaration (treated as Module-like).
fn extract_extension(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_extension_type_name(node, source);
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

    // Check for protocol conformance in extension
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods (extension uses class_body)
    if let Some(body) = find_child_by_kind(node, "class_body") {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a top-level property declaration as a Constant.
fn extract_property(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_property_name(node, source);
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

/// Extract a typealias declaration.
fn extract_typealias(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_identifier(node, source);
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

/// Collect members (methods, type aliases, nested types) inside a body node.
fn collect_body_members(
    body: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                extract_function(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "protocol_function_declaration" => {
                extract_function(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "init_declaration" => {
                extract_init(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "deinit_declaration" => {
                extract_deinit(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "typealias_declaration" => {
                extract_typealias(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "class_declaration" => {
                // Nested class/struct/enum
                let decl_kind = classify_class_declaration(child, source);
                match decl_kind {
                    SwiftDeclKind::Class => {
                        extract_class(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Struct => {
                        extract_struct(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Enum => {
                        extract_enum(child, file, source, nodes, defined_fqns, edges);
                    }
                    SwiftDeclKind::Extension => {
                        extract_extension(child, file, source, nodes, defined_fqns, edges);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract inheritance edges from the inheritance_specifier children.
fn extract_inheritance(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "inheritance_specifier" {
            extract_type_names_from_inheritance(child, source, source_fqn, edges);
        }
    }
}

/// Extract type names from an inheritance specifier and emit Inherits edges.
fn extract_type_names_from_inheritance(
    clause: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            "user_type" => {
                let name = get_user_type_name(child, source);
                if !name.is_empty() {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.to_string(),
                        target_fqn: name,
                        kind: EdgeKind::Inherits,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
            "type_identifier" | "simple_identifier" => {
                let name = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !name.is_empty() {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.to_string(),
                        target_fqn: name,
                        kind: EdgeKind::Inherits,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
            _ => {
                // Recurse into other nodes that might contain type identifiers
                if child.child_count() > 0 {
                    extract_type_names_from_inheritance(child, source, source_fqn, edges);
                }
            }
        }
    }
}

/// Collect import declarations.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            extract_import(child, file, source, edges);
        } else if child.child_count() > 0 {
            collect_imports(child, file, source, edges);
        }
    }
}

/// Extract an import declaration into Imports edges.
fn extract_import(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return;
    }

    // Strip "import" keyword and optional kind specifier
    let import_path = text
        .trim_start_matches("import")
        .trim()
        .trim_start_matches("struct ")
        .trim_start_matches("class ")
        .trim_start_matches("enum ")
        .trim_start_matches("protocol ")
        .trim_start_matches("func ")
        .trim_start_matches("var ")
        .trim_start_matches("let ")
        .trim_start_matches("typealias ")
        .trim();

    if !import_path.is_empty() {
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: import_path.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({}),
        });
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
        if child.kind() == "call_expression" {
            extract_call_edge(child, file, source, defined_fqns, edges);
        }

        // Recurse into children
        if child.child_count() > 0 {
            collect_calls(child, file, source, defined_fqns, edges);
        }
    }
}

/// Extract a single call edge from a call_expression node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    // The function being called is typically the first child
    let func_node = node.child(0);

    if let Some(func) = func_node {
        let call_text = func.utf8_text(source).unwrap_or("").trim();

        // Only resolve simple identifier calls (not method calls like obj.method())
        if func.kind() == "simple_identifier"
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
        if parent.kind() == "function_declaration"
            || parent.kind() == "protocol_function_declaration"
            || parent.kind() == "init_declaration"
            || parent.kind() == "deinit_declaration"
        {
            let name = if parent.kind() == "init_declaration" {
                "init".to_string()
            } else if parent.kind() == "deinit_declaration" {
                "deinit".to_string()
            } else {
                get_function_name(parent, source)
            };

            if !name.is_empty() {
                // Check if this function is inside a class/struct/enum/protocol/extension
                let mut ancestor = parent.parent();
                while let Some(anc) = ancestor {
                    match anc.kind() {
                        "class_declaration" => {
                            let cls = get_type_identifier_or_extension_name(anc, source);
                            if !cls.is_empty() {
                                return Some(format!("{file}::{cls}::{name}"));
                            }
                            break;
                        }
                        "protocol_declaration" => {
                            let cls = get_type_identifier(anc, source);
                            if !cls.is_empty() {
                                return Some(format!("{file}::{cls}::{name}"));
                            }
                            break;
                        }
                        // Skip body nodes
                        "class_body" | "protocol_body" | "enum_class_body" => {}
                        _ => {}
                    }
                    ancestor = anc.parent();
                }
                return Some(format!("{file}::{name}"));
            }
        }
        current = parent.parent();
    }
    // Call at file level
    Some(file.to_string())
}

// ─── Helper functions ───────────────────────────────────────────────────────

/// Get the type_identifier from a declaration node (class, struct, enum, protocol, typealias).
fn get_type_identifier(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }

    // Look for type_identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    String::new()
}

/// Get the type name from a class_declaration that could be an extension.
/// For extensions, the type is in a `user_type` child.
fn get_type_identifier_or_extension_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let decl_kind = classify_class_declaration(node, source);
    if decl_kind == SwiftDeclKind::Extension {
        get_extension_type_name(node, source)
    } else {
        get_type_identifier(node, source)
    }
}

/// Get the type name from an extension declaration.
/// In tree-sitter-swift, extensions have a `user_type` child for the extended type.
fn get_extension_type_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "user_type" {
            return get_user_type_name(child, source);
        }
        if child.kind() == "type_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

/// Get the simple name from a user_type node.
fn get_user_type_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // user_type contains a type_identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            return child.utf8_text(source).unwrap_or("").trim().to_string();
        }
    }
    // Fallback: get full text
    node.utf8_text(source).unwrap_or("").trim().to_string()
}

/// Get the function name from a function_declaration or protocol_function_declaration.
fn get_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }

    // Look for simple_identifier child (function names use simple_identifier)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "simple_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() && text != "func" {
                return text;
            }
        }
    }

    String::new()
}

/// Get the name from a property declaration (let/var).
fn get_property_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Look for pattern or simple_identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "pattern" {
            // Look inside pattern for identifier
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "simple_identifier" {
                    let t = inner.utf8_text(source).unwrap_or("").trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            // Pattern itself might be the identifier
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() && text != "let" && text != "var" {
                return text;
            }
        }
        if child.kind() == "simple_identifier" {
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() && text != "let" && text != "var" {
                return text;
            }
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

    /// Helper to parse Swift source and run extraction.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .expect("Swift grammar should load");
        let tree = parser.parse(source, None).expect("Should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_classes_with_methods() {
        let source = r#"
class ViewController: UIViewController {
    func viewDidLoad() {
        super.viewDidLoad()
    }

    func fetchData() async throws -> [Item] {
        return []
    }

    private func handleError(_ error: Error) {
        print("Error: \(error)")
    }
}

final class APIClient {
    func request(_ endpoint: String) async throws -> Data {
        return Data()
    }
}
"#;
        let result = parse_and_extract("Sources/App.swift", source);

        // Classes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/App.swift::ViewController" && n.kind == NodeKind::Class),
            "Expected ViewController class, got nodes: {:?}",
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
                .any(|n| n.fqn == "Sources/App.swift::APIClient" && n.kind == NodeKind::Class),
            "Expected APIClient class"
        );

        // Methods inside class
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "Sources/App.swift::ViewController::viewDidLoad"
                    && n.kind == NodeKind::Function
            ),
            "Expected viewDidLoad method"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/App.swift::ViewController::fetchData"
                    && n.kind == NodeKind::Function),
            "Expected fetchData method"
        );
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "Sources/App.swift::ViewController::handleError"
                    && n.kind == NodeKind::Function
            ),
            "Expected handleError method"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/App.swift::APIClient::request"
                    && n.kind == NodeKind::Function),
            "Expected request method"
        );
    }

    #[test]
    fn test_structs_with_methods() {
        let source = r#"
struct Config {
    func validate() -> Bool {
        return true
    }
}

struct Point {
    func distance(to other: Point) -> Double {
        return 0.0
    }
}
"#;
        let result = parse_and_extract("Sources/Models.swift", source);

        // Structs (use Class kind with struct attribute)
        let config = result
            .nodes
            .iter()
            .find(|n| n.fqn == "Sources/Models.swift::Config");
        assert!(
            config.is_some(),
            "Expected Config struct, got: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(config.unwrap().kind, NodeKind::Class);
        assert_eq!(config.unwrap().attributes["struct"], true);

        let point = result
            .nodes
            .iter()
            .find(|n| n.fqn == "Sources/Models.swift::Point");
        assert!(point.is_some(), "Expected Point struct");
        assert_eq!(point.unwrap().kind, NodeKind::Class);
        assert_eq!(point.unwrap().attributes["struct"], true);

        // Methods
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Models.swift::Config::validate"
                    && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Models.swift::Point::distance"
                    && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_protocols_with_required_methods() {
        let source = r#"
protocol DataFetchable {
    func fetchData() async throws -> Data
    func cancel()
}

protocol Logging {
    func log(message: String)
}
"#;
        let result = parse_and_extract("Sources/Protocols.swift", source);

        // Protocols
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Protocols.swift::DataFetchable"
                    && n.kind == NodeKind::Interface),
            "Expected DataFetchable protocol, got: {:?}",
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
                .any(|n| n.fqn == "Sources/Protocols.swift::Logging"
                    && n.kind == NodeKind::Interface),
            "Expected Logging protocol"
        );

        // Required methods
        assert!(
            result.nodes.iter().any(|n| n.fqn
                == "Sources/Protocols.swift::DataFetchable::fetchData"
                && n.kind == NodeKind::Function),
            "Expected fetchData method in protocol, got: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(result.nodes.iter().any(|n| n.fqn
            == "Sources/Protocols.swift::DataFetchable::cancel"
            && n.kind == NodeKind::Function));
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Protocols.swift::Logging::log"
                    && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_enums_with_methods() {
        let source = r#"
enum NetworkError: Error {
    case timeout
    case invalidResponse
}

enum Direction {
    case north, south, east, west

    func opposite() -> Direction {
        return .north
    }
}
"#;
        let result = parse_and_extract("Sources/Enums.swift", source);

        // Enums
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Enums.swift::NetworkError" && n.kind == NodeKind::Enum),
            "Expected NetworkError enum, got: {:?}",
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
                .any(|n| n.fqn == "Sources/Enums.swift::Direction" && n.kind == NodeKind::Enum),
            "Expected Direction enum"
        );

        // Method in enum
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Enums.swift::Direction::opposite"
                    && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_extensions() {
        let source = r#"
extension String {
    func trimmed() -> String {
        return self.trimmingCharacters(in: .whitespaces)
    }
}

extension Array {
    func chunked(size: Int) -> [[Element]] {
        return []
    }
}
"#;
        let result = parse_and_extract("Sources/Extensions.swift", source);

        // Extensions (Module kind)
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Extensions.swift::String" && n.kind == NodeKind::Module),
            "Expected String extension, got: {:?}",
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
                .any(|n| n.fqn == "Sources/Extensions.swift::Array" && n.kind == NodeKind::Module),
            "Expected Array extension"
        );

        // Methods in extensions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Extensions.swift::String::trimmed"
                    && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Sources/Extensions.swift::Array::chunked"
                    && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_imports() {
        let source = r#"
import Foundation
import UIKit
import struct SwiftUI.Color
"#;
        let result = parse_and_extract("Sources/App.swift", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.len() >= 2,
            "Expected at least 2 imports, got {}",
            imports.len()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Foundation"),
            "Expected Foundation import, got: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "UIKit"),
            "Expected UIKit import"
        );
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
func greet() -> String {
    return "hello"
}

func main() {
    greet()
}
"#;
        let result = parse_and_extract("Sources/app.swift", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // main calls greet
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn.contains("main") && e.target_fqn.contains("greet")),
            "Expected a call from main to greet, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_inheritance_edges() {
        let source = r#"
protocol Animal {
    func speak() -> String
}

class Dog: Animal {
    func speak() -> String {
        return "Woof"
    }
}
"#;
        let result = parse_and_extract("Sources/Animals.swift", source);

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();

        // Dog inherits Animal
        assert!(
            inherits
                .iter()
                .any(|e| e.source_fqn == "Sources/Animals.swift::Dog" && e.target_fqn == "Animal"),
            "Expected Dog inherits Animal, got: {:?}",
            inherits
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_and_extract("empty.swift", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_deprecated_extract_swift_wrapper() {
        let source = r#"
class Hello {
    func world() {}
}
"#;
        #[allow(deprecated)]
        let result = extract_swift("test.swift", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "test.swift::Hello" && n.kind == NodeKind::Class)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "test.swift::Hello::world" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_line_numbers_are_accurate() {
        let source = "class Foo {\n    func bar() {}\n}\n";
        let result = parse_and_extract("test.swift", source);

        let foo = result.nodes.iter().find(|n| n.fqn == "test.swift::Foo");
        assert!(
            foo.is_some(),
            "Expected Foo node, got: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        let foo = foo.unwrap();
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 3);

        let bar = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.swift::Foo::bar");
        assert!(bar.is_some(), "Expected bar node");
        let bar = bar.unwrap();
        assert_eq!(bar.start_line, 2);
        assert_eq!(bar.end_line, 2);
    }
}
