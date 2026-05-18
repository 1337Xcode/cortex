//! Scala AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (objects, classes, traits, enums, functions,
//! constants, type aliases) and edges (imports, calls, inheritance)
//! from a tree-sitter Scala parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Scala file.
///
/// Handles:
/// - Object definitions (singleton objects, case objects)
/// - Class definitions (regular, case, abstract)
/// - Trait definitions
/// - Enum definitions (Scala 3)
/// - Function/method definitions (`def`)
/// - Val definitions (top-level constants)
/// - Type alias definitions
/// - Import declarations
/// - Intra-file call expressions resolved to definitions in the same file
/// - Inheritance (extends) and mixin (with) edges
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
        .set_language(&tree_sitter_scala::LANGUAGE.into())
        .expect("Scala grammar should load");
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
                find_ast_node_at_line(root, node.start_line, "function_definition")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "function_declaration"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "scala");
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

/// Recursively collect definitions: objects, classes, traits, enums,
/// functions, vals, and type aliases.
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
            "object_definition" => {
                extract_object(child, file, source, nodes, defined_fqns, edges);
            }
            "class_definition" => {
                extract_class(child, file, source, nodes, defined_fqns, edges);
            }
            "trait_definition" => {
                extract_trait(child, file, source, nodes, defined_fqns, edges);
            }
            "enum_definition" => {
                extract_enum(child, file, source, nodes, defined_fqns);
            }
            "function_definition" | "function_declaration" => {
                extract_function(child, file, source, parent_name, nodes, defined_fqns);
            }
            "val_definition" | "val_declaration" => {
                // Only extract top-level vals as constants
                if parent_name.is_none() {
                    extract_val(child, file, source, nodes, defined_fqns);
                }
            }
            "type_definition" => {
                extract_type_alias(child, file, source, parent_name, nodes, defined_fqns);
            }
            // Recurse into template bodies and other containers
            "template_body" => {
                collect_definitions(child, file, source, parent_name, nodes, defined_fqns, edges);
            }
            "compilation_unit" | "package_clause" => {
                collect_definitions(child, file, source, parent_name, nodes, defined_fqns, edges);
            }
            _ => {
                // Recurse into other containers (e.g., block expressions)
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
            | "operator_identifier"
            | "string"
            | "integer_literal"
            | "floating_point_literal"
            | "boolean_literal"
            | "character_literal"
            | "symbol_literal"
            | "null_literal"
            | "comment"
            | "block_comment"
            | "type_identifier"
            | "import_declaration"
            | "call_expression"
    )
}

/// Extract an object definition (singleton object).
fn extract_object(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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
        attributes: json!({"object": true}),
    });

    defined_fqns.push((name.clone(), fqn.clone()));

    // Check for extends/with clauses
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "template_body")
        .or_else(|| node.child_by_field_name("body"))
    {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a class definition (regular, case, abstract).
fn extract_class(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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

    // Check for extends/with clauses
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "template_body")
        .or_else(|| node.child_by_field_name("body"))
    {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract a trait definition.
fn extract_trait(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    edges: &mut Vec<Edge>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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

    defined_fqns.push((name.clone(), fqn.clone()));

    // Check for extends/with clauses
    extract_inheritance(node, source, &fqn, edges);

    // Recurse into body for methods
    if let Some(body) = find_child_by_kind(node, "template_body")
        .or_else(|| node.child_by_field_name("body"))
    {
        collect_body_members(body, file, source, &name, nodes, defined_fqns, edges);
    }
}

/// Extract an enum definition (Scala 3).
fn extract_enum(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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

/// Extract a function/method definition (`def`).
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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

/// Extract a val definition as a Constant node.
fn extract_val(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // Val pattern can be a simple identifier or a destructuring pattern
    let name = get_val_name(node, source);
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

/// Extract a type alias definition.
fn extract_type_alias(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_child_by_field_or_first_identifier(node, source, "name");
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

/// Collect members (methods, type aliases, vals) inside a class/object/trait body.
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
            "function_definition" | "function_declaration" => {
                extract_function(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "type_definition" => {
                extract_type_alias(child, file, source, Some(parent_name), nodes, defined_fqns);
            }
            "object_definition" => {
                // Nested objects (companion objects, etc.)
                extract_object(child, file, source, nodes, defined_fqns, edges);
            }
            "class_definition" => {
                extract_class(child, file, source, nodes, defined_fqns, edges);
            }
            "trait_definition" => {
                extract_trait(child, file, source, nodes, defined_fqns, edges);
            }
            _ => {}
        }
    }
}

/// Extract inheritance edges from extends/with clauses.
fn extract_inheritance(
    node: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "extends_clause" => {
                // The extends clause contains the parent type(s)
                extract_type_names_from_clause(child, source, source_fqn, edges);
            }
            _ => {}
        }
    }
}

/// Extract type names from an extends clause and emit Inherits edges.
/// Handles `extends Base with Trait1 with Trait2` patterns.
fn extract_type_names_from_clause(
    clause: tree_sitter::Node,
    source: &[u8],
    source_fqn: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            // Type identifiers are the actual type names
            "type_identifier" => {
                let name = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !name.is_empty() && name != "extends" && name != "with" {
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
            // Generic types like Foo[T] - extract the base type name
            "generic_type" => {
                if let Some(type_id) = find_first_child_by_kind(child, "type_identifier") {
                    let name = type_id.utf8_text(source).unwrap_or("").trim().to_string();
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
            }
            // Recurse into compound types (e.g., `A with B`)
            "compound_type" => {
                extract_type_names_from_clause(child, source, source_fqn, edges);
            }
            _ => {
                // Recurse into other nodes that might contain type identifiers
                if child.child_count() > 0 {
                    extract_type_names_from_clause(child, source, source_fqn, edges);
                }
            }
        }
    }
}

/// Collect import declarations.
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
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
fn extract_import(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    // Get the full import text and parse it
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return;
    }

    // Strip the "import " prefix
    let import_path = text
        .trim_start_matches("import")
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
    // The function being called is typically the first child (the callee)
    let func_node = node.child_by_field_name("function").or_else(|| node.child(0));

    if let Some(func) = func_node {
        let call_text = func.utf8_text(source).unwrap_or("").trim();

        // Only resolve simple identifier calls (not method calls like obj.method())
        if func.kind() == "identifier" {
            if let Some((_, target_fqn)) =
                defined_fqns.iter().find(|(name, _)| name == call_text)
            {
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

    while let Some(parent) = current {
        if parent.kind() == "function_definition" || parent.kind() == "function_declaration" {
            let name = get_child_by_field_or_first_identifier(parent, source, "name");
            if !name.is_empty() {
                // Check if this function is inside a class/object/trait
                let mut ancestor = parent.parent();
                while let Some(anc) = ancestor {
                    match anc.kind() {
                        "object_definition" | "class_definition" | "trait_definition"
                        | "enum_definition" => {
                            let cls =
                                get_child_by_field_or_first_identifier(anc, source, "name");
                            if !cls.is_empty() {
                                return Some(format!("{file}::{cls}::{name}"));
                            }
                            break;
                        }
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

/// Get the name of a node by trying the "name" field first, then looking for
/// the first `identifier` or `type_identifier` child.
fn get_child_by_field_or_first_identifier(
    node: tree_sitter::Node,
    source: &[u8],
    field: &str,
) -> String {
    // Try field name first
    if let Some(name_node) = node.child_by_field_name(field) {
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
            if !text.is_empty() {
                return text;
            }
        }
    }

    String::new()
}

/// Get the name from a val definition. Looks for a pattern child with an identifier.
fn get_val_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try "pattern" field
    if let Some(pattern) = node.child_by_field_name("pattern") {
        if pattern.kind() == "identifier" {
            return pattern.utf8_text(source).unwrap_or("").trim().to_string();
        }
        // Look for identifier inside pattern
        let mut cursor = pattern.walk();
        for child in pattern.children(&mut cursor) {
            if child.kind() == "identifier" {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
        }
    }

    // Fallback: look for first identifier child directly
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("").trim().to_string();
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
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Find the first child of a given kind (non-recursive, first level only).
fn find_first_child_by_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Scala source and run extraction.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_scala::LANGUAGE.into())
            .expect("Scala grammar should load");
        let tree = parser.parse(source, None).expect("Should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_objects_with_methods() {
        let source = r#"
object Main {
  def main(args: Array[String]): Unit = {
    println("Hello")
  }

  def helper(): Int = 42
}

case object Singleton
"#;
        let result = parse_and_extract("src/app.scala", source);

        // Objects
        assert!(result.nodes.iter().any(|n| n.fqn == "src/app.scala::Main" && n.kind == NodeKind::Module));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/app.scala::Singleton" && n.kind == NodeKind::Module));

        // Methods inside object
        assert!(result.nodes.iter().any(|n| n.fqn == "src/app.scala::Main::main" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/app.scala::Main::helper" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_classes_regular_and_case() {
        let source = r#"
class OrderService {
  def processOrder(order: Order): Unit = {}
  def validate(order: Order): Boolean = true
}

case class Point(x: Int, y: Int)

abstract class BaseService {
  def initialize(): Unit
}
"#;
        let result = parse_and_extract("src/service.scala", source);

        // Classes
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::OrderService" && n.kind == NodeKind::Class));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::Point" && n.kind == NodeKind::Class));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::BaseService" && n.kind == NodeKind::Class));

        // Methods
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::OrderService::processOrder" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::OrderService::validate" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/service.scala::BaseService::initialize" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_traits_with_methods() {
        let source = r#"
trait Serializable {
  def serialize(): String
  def deserialize(data: String): Unit = {}
}

trait Logging {
  def log(msg: String): Unit = println(msg)
}
"#;
        let result = parse_and_extract("src/traits.scala", source);

        // Traits
        assert!(result.nodes.iter().any(|n| n.fqn == "src/traits.scala::Serializable" && n.kind == NodeKind::Trait));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/traits.scala::Logging" && n.kind == NodeKind::Trait));

        // Methods in traits
        assert!(result.nodes.iter().any(|n| n.fqn == "src/traits.scala::Serializable::serialize" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/traits.scala::Serializable::deserialize" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/traits.scala::Logging::log" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_enum_scala3() {
        let source = r#"
enum Color {
  case Red, Green, Blue
}

enum Planet(mass: Double, radius: Double) {
  case Mercury extends Planet(3.303e+23, 2.4397e6)
  case Venus extends Planet(4.869e+24, 6.0518e6)
}
"#;
        let result = parse_and_extract("src/enums.scala", source);

        // Enums
        assert!(result.nodes.iter().any(|n| n.fqn == "src/enums.scala::Color" && n.kind == NodeKind::Enum));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/enums.scala::Planet" && n.kind == NodeKind::Enum));
    }

    #[test]
    fn test_imports() {
        let source = r#"
import scala.collection.mutable
import akka.actor.{Actor, ActorSystem}
import java.util._
"#;
        let result = parse_and_extract("src/imports.scala", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(imports.len() >= 3, "Expected at least 3 imports, got {}", imports.len());
        assert!(imports.iter().any(|e| e.target_fqn.contains("scala.collection.mutable")));
        assert!(imports.iter().any(|e| e.target_fqn.contains("akka.actor")));
        assert!(imports.iter().any(|e| e.target_fqn.contains("java.util")));
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
object App {
  def greet(): String = "hello"

  def main(): Unit = {
    greet()
  }
}
"#;
        let result = parse_and_extract("src/app.scala", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // main calls greet
        assert!(
            calls.iter().any(|e| e.source_fqn.contains("main") && e.target_fqn.contains("greet")),
            "Expected a call from main to greet, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_inheritance_edges() {
        let source = r#"
trait Animal {
  def speak(): String
}

trait Domestic

class Dog extends Animal with Domestic {
  def speak(): String = "Woof"
}
"#;
        let result = parse_and_extract("src/animals.scala", source);

        let inherits: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect();

        // Dog extends Animal
        assert!(
            inherits.iter().any(|e| e.source_fqn == "src/animals.scala::Dog" && e.target_fqn == "Animal"),
            "Expected Dog inherits Animal, got: {:?}",
            inherits
        );
        // Dog with Domestic
        assert!(
            inherits.iter().any(|e| e.source_fqn == "src/animals.scala::Dog" && e.target_fqn == "Domestic"),
            "Expected Dog inherits Domestic, got: {:?}",
            inherits
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_and_extract("empty.scala", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_type_alias() {
        let source = r#"
type StringMap = Map[String, String]
type Callback = () => Unit
"#;
        let result = parse_and_extract("src/types.scala", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "src/types.scala::StringMap" && n.kind == NodeKind::TypeAlias));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/types.scala::Callback" && n.kind == NodeKind::TypeAlias));
    }

    #[test]
    fn test_top_level_val_as_constant() {
        let source = r#"
val MAX_RETRIES = 3
val DEFAULT_TIMEOUT = 5000
"#;
        let result = parse_and_extract("src/config.scala", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "src/config.scala::MAX_RETRIES" && n.kind == NodeKind::Constant));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/config.scala::DEFAULT_TIMEOUT" && n.kind == NodeKind::Constant));
    }

    #[test]
    fn test_deprecated_extract_regex_wrapper() {
        let source = r#"
object Hello {
  def world(): Unit = {}
}
"#;
        #[allow(deprecated)]
        let result = extract_regex("test.scala", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "test.scala::Hello" && n.kind == NodeKind::Module));
        assert!(result.nodes.iter().any(|n| n.fqn == "test.scala::Hello::world" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_line_numbers_are_accurate() {
        let source = "object Foo {\n  def bar(): Unit = {}\n}\n";
        let result = parse_and_extract("test.scala", source);

        let foo = result.nodes.iter().find(|n| n.fqn == "test.scala::Foo").unwrap();
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 3);

        let bar = result.nodes.iter().find(|n| n.fqn == "test.scala::Foo::bar").unwrap();
        assert_eq!(bar.start_line, 2);
        assert_eq!(bar.end_line, 2);
    }
}
