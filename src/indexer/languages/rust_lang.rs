//! Rust AST extractor.
//!
//! Extracts structural nodes (functions, methods, structs, traits, enums) and edges
//! (calls, use statements) from a tree-sitter Rust parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Rust file.
///
/// Handles:
/// - Function items (`fn foo() {}`)
/// - Impl blocks with methods (`impl Struct { fn method() {} }`)
/// - Struct items
/// - Trait items
/// - Enum items
/// - Use declarations
/// - Intra-file call expressions resolved to definitions in the same file
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(root, file, source_bytes, None, &mut nodes, &mut defined_fqns);

    // Compute cyclomatic complexity for Function nodes
    compute_node_complexities(&mut nodes, root, source_bytes);

    // Second pass: collect use statements and calls
    collect_use_statements(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Compute cyclomatic complexity for all Function nodes by finding their
/// corresponding AST nodes and walking the subtree.
fn compute_node_complexities(
    nodes: &mut [Node],
    root: tree_sitter::Node,
    source: &[u8],
) {
    for node in nodes.iter_mut() {
        if node.kind == NodeKind::Function {
            // Find the AST node matching this function's line range
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_item")
            {
                let c = complexity::compute_full_complexity(ast_node, source, "rust");
                if let Some(attrs) = node.attributes.as_object_mut() {
                    attrs.insert("complexity".to_string(), serde_json::json!(c));
                }
            }
        }
    }
}

/// Find an AST node of a given kind at a specific start line (1-indexed).
fn find_ast_node_at_line<'a>(node: tree_sitter::Node<'a>, target_line: u32, kind: &str) -> Option<tree_sitter::Node<'a>> {
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

/// Recursively collect function, struct, trait, enum, and impl definitions.
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    impl_type: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match impl_type {
                        Some(type_name) => format!("{file}::{type_name}::{name}"),
                        None => format!("{file}::{name}"),
                    };
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

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

                    defined_fqns.push((name.to_string(), fqn));
                }
            }
            "struct_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{name}");
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

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

                    defined_fqns.push((name.to_string(), fqn));
                }
            }
            "trait_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{name}");
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

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

                    defined_fqns.push((name.to_string(), fqn));
                }
            }
            "enum_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{name}");
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Type,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({}),
                    });

                    defined_fqns.push((name.to_string(), fqn));
                }
            }
            "impl_item" => {
                // Extract the type being implemented
                let type_name = extract_impl_type(child, source);
                if let Some(ref type_name) = type_name {
                    // Recurse into impl body to find methods
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions(
                            body,
                            file,
                            source,
                            Some(type_name),
                            nodes,
                            defined_fqns,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract the type name from an impl_item node.
fn extract_impl_type(impl_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // The "type" field of impl_item contains the type being implemented
    if let Some(type_node) = impl_node.child_by_field_name("type") {
        let text = type_node.utf8_text(source).unwrap_or("");
        // Strip generic parameters if present (e.g., "MyStruct<T>" -> "MyStruct")
        let base_name = text.split('<').next().unwrap_or(text).trim();
        if !base_name.is_empty() {
            return Some(base_name.to_string());
        }
    }
    None
}

/// Collect use declarations and create Imports edges.
/// Detects `pub use` re-exports and marks them with `reexport: true`.
fn collect_use_statements(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "use_declaration" {
            if let Some(arg_node) = child.child_by_field_name("argument") {
                let use_path = arg_node.utf8_text(source).unwrap_or("");
                if !use_path.is_empty() {
                    // Check if this is a `pub use` re-export
                    let full_text = child.utf8_text(source).unwrap_or("");
                    let is_reexport = full_text.trim_start().starts_with("pub use");
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: use_path.to_string(),
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: if is_reexport {
                            json!({"reexport": true})
                        } else {
                            json!({})
                        },
                    });
                }
            }
        }
    }
}

/// Collect intra-file call expressions and create Calls edges.
///
/// Handles simple calls (`foo()`) and method calls (`self.method()`, `obj.method()`).
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
            if let Some(func_node) = child.child_by_field_name("function") {
                let caller_fqn = find_enclosing_function(child, file, source)
                    .unwrap_or_else(|| file.to_string());

                match func_node.kind() {
                    "identifier" => {
                        let call_name = func_node.utf8_text(source).unwrap_or("");
                        if let Some((_, target_fqn)) =
                            defined_fqns.iter().find(|(name, _)| name == call_name)
                        {
                            if caller_fqn != *target_fqn {
                                edges.push(Edge {
                                    id: None,
                                    source_fqn: caller_fqn,
                                    target_fqn: target_fqn.clone(),
                                    kind: EdgeKind::Calls,
                                    confidence: 1.0,
                                    attributes: json!({}),
                                });
                            }
                        }
                    }
                    "scoped_identifier" | "path_expression" => {
                        // Qualified call: module::function() or Type::method()
                        let full_text = func_node.utf8_text(source).unwrap_or("");
                        if let Some(sep_pos) = full_text.rfind("::") {
                            let qualifier = &full_text[..sep_pos];
                            let func_name = &full_text[sep_pos + 2..];
                            if !func_name.is_empty() {
                                // Try exact FQN match first, then short name
                                let (target_fqn, confidence) =
                                    if let Some((_, fqn)) = defined_fqns
                                        .iter()
                                        .find(|(_, fqn)| fqn.ends_with(&format!("::{func_name}")))
                                    {
                                        (fqn.clone(), 1.0_f64)
                                    } else {
                                        (full_text.to_string(), 0.6_f64)
                                    };
                                edges.push(Edge {
                                    id: None,
                                    source_fqn: caller_fqn,
                                    target_fqn,
                                    kind: EdgeKind::Calls,
                                    confidence,
                                    attributes: json!({
                                        "receiver": qualifier,
                                        "call_type": "qualified"
                                    }),
                                });
                            }
                        }
                    }
                    "field_expression" => {
                        // self.method() or obj.method()
                        let full_text = func_node.utf8_text(source).unwrap_or("");
                        if let Some(dot_pos) = full_text.rfind('.') {
                            let receiver = &full_text[..dot_pos];
                            let method_name = &full_text[dot_pos + 1..];
                            if !method_name.is_empty() {
                                let chain_position = count_chain_depth_rs(func_node);
                                let (target_fqn, confidence) =
                                    if let Some((_, fqn)) = defined_fqns
                                        .iter()
                                        .find(|(name, _)| name == method_name)
                                    {
                                        (fqn.clone(), 1.0_f64)
                                    } else {
                                        (method_name.to_string(), 0.0_f64)
                                    };
                                edges.push(Edge {
                                    id: None,
                                    source_fqn: caller_fqn,
                                    target_fqn,
                                    kind: EdgeKind::Calls,
                                    confidence,
                                    attributes: json!({
                                        "receiver": receiver,
                                        "call_type": "method",
                                        "chain_position": chain_position
                                    }),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Recurse into children
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

/// Find the enclosing function for a given node to determine the caller FQN.
fn find_enclosing_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        if parent.kind() == "function_item" {
            if let Some(name_node) = parent.child_by_field_name("name") {
                let func_name = name_node.utf8_text(source).unwrap_or("");
                // Check if this function is inside an impl block
                let impl_type = find_enclosing_impl(parent, source);
                return match impl_type {
                    Some(type_name) => Some(format!("{file}::{type_name}::{func_name}")),
                    None => Some(format!("{file}::{func_name}")),
                };
            }
        }
        current = parent.parent();
    }
    // Call at module level
    Some(file.to_string())
}

/// Find the enclosing impl block's type name for a function node.
fn find_enclosing_impl(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "impl_item" {
            return extract_impl_type(parent, source);
        }
        current = parent.parent();
    }
    None
}

fn count_chain_depth_rs(func_node: tree_sitter::Node) -> u32 {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            return 1 + count_chain_depth_in_call_rs(child);
        }
    }
    0
}

fn count_chain_depth_in_call_rs(call_node: tree_sitter::Node) -> u32 {
    if let Some(func) = call_node.child_by_field_name("function") {
        if func.kind() == "field_expression" {
            return count_chain_depth_rs(func);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_functions_structs_traits_enums_use() {
        let source = r#"use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug)]
pub struct Config {
    pub name: String,
    pub values: HashMap<String, String>,
}

pub trait Configurable {
    fn configure(&self) -> Result<(), Error>;
}

pub enum Status {
    Active,
    Inactive,
    Pending,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Config {
            name: name.to_string(),
            values: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.values.get(key)
    }
}

pub fn create_config(name: &str) -> Config {
    Config::new(name)
}
"#;
        let tree = parse_rust(source);
        let result = extract(&tree, "src/config.rs", source);

        // Nodes: Config struct + Configurable trait + Status enum + new + get + create_config = 6
        assert_eq!(result.nodes.len(), 6);

        // Check struct
        let config_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/config.rs::Config")
            .unwrap();
        assert_eq!(config_node.kind, NodeKind::Class);

        // Check trait
        let trait_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/config.rs::Configurable")
            .unwrap();
        assert_eq!(trait_node.kind, NodeKind::Interface);

        // Check enum
        let enum_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/config.rs::Status")
            .unwrap();
        assert_eq!(enum_node.kind, NodeKind::Type);

        // Check impl methods
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/config.rs::Config::new"));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/config.rs::Config::get"));

        // Check top-level function
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/config.rs::create_config"));

        // Check use/import edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges
            .iter()
            .any(|e| e.target_fqn == "std::collections::HashMap"));
        assert!(import_edges
            .iter()
            .any(|e| e.target_fqn == "serde::{Serialize, Deserialize}"));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
struct MyStruct {}

impl MyStruct {
    fn do_work(&self) {}
}

fn standalone() {}
"#;
        let tree = parse_rust(source);
        let result = extract(&tree, "src/worker.rs", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/worker.rs::MyStruct"));
        assert!(fqns.contains(&"src/worker.rs::MyStruct::do_work"));
        assert!(fqns.contains(&"src/worker.rs::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
fn valid_function() {
    println!("hello");
}

fn broken( {{{ invalid syntax !!!

fn another_valid() {}
"#;
        let tree = parse_rust(source);
        let result = extract(&tree, "broken.rs", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "broken.rs::valid_function"));
    }

    #[test]
    fn test_call_edges() {
        let source = r#"
fn helper() -> i32 {
    42
}

fn main() {
    let x = helper();
    println!("{}", x);
}
"#;
        let tree = parse_rust(source);
        let result = extract(&tree, "src/main.rs", source);

        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(call_edges.iter().any(|e| {
            e.source_fqn == "src/main.rs::main" && e.target_fqn == "src/main.rs::helper"
        }));
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_rust(source);
        let result = extract(&tree, "empty.rs", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
