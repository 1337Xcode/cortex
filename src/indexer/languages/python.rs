//! Python AST extractor.
//!
//! Extracts structural nodes (functions, methods, classes) and edges (calls, imports)
//! from a tree-sitter Python parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Python file.
///
/// Handles:
/// - Top-level function definitions (`def`)
/// - Class definitions
/// - Methods (functions defined inside a class)
/// - Import statements (`import` and `from ... import`)
/// - Intra-file call expressions resolved to definitions in the same file
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_functions: Vec<String> = Vec::new();
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(
        root,
        file,
        source_bytes,
        None,
        &mut nodes,
        &mut defined_functions,
        &mut defined_fqns,
    );

    // Compute cyclomatic complexity for Function nodes
    compute_node_complexities(&mut nodes, root, source_bytes);

    // Second pass: collect imports and calls
    collect_imports(root, file, source_bytes, &mut edges);
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
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_definition")
            {
                let c = complexity::compute_full_complexity(ast_node, source, "python");
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

/// Recursively collect function, method, and class definitions.
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_functions: &mut Vec<String>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match class_name {
                        Some(cls) => format!("{file}::{cls}::{name}"),
                        None => format!("{file}::{name}"),
                    };
                    let kind = match class_name {
                        Some(_) => NodeKind::Function, // methods are still Function kind
                        None => NodeKind::Function,
                    };
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({}),
                    });

                    defined_functions.push(name.to_string());
                    defined_fqns.push((name.to_string(), fqn));
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{cls_name}");
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

                    defined_fqns.push((cls_name.to_string(), fqn));

                    // Recurse into class body to find methods
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions(
                            body,
                            file,
                            source,
                            Some(cls_name),
                            nodes,
                            defined_functions,
                            defined_fqns,
                        );
                    }
                }
            }
            // Recurse into decorated definitions
            "decorated_definition" => {
                collect_definitions(
                    child,
                    file,
                    source,
                    class_name,
                    nodes,
                    defined_functions,
                    defined_fqns,
                );
            }
            _ => {}
        }
    }
}

/// Collect import statements and create Imports edges.
/// Detects `from module import *` as a re-export pattern.
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                let mut name_cursor = child.walk();
                for name_child in child.children(&mut name_cursor) {
                    if name_child.kind() == "dotted_name" {
                        let module_path = name_child.utf8_text(source).unwrap_or("");
                        if !module_path.is_empty() {
                            edges.push(Edge {
                                id: None,
                                source_fqn: file.to_string(),
                                target_fqn: module_path.to_string(),
                                kind: EdgeKind::Imports,
                                confidence: 1.0,
                                attributes: json!({}),
                            });
                        }
                    }
                }
            }
            "import_from_statement" => {
                if let Some(module_node) = child.child_by_field_name("module_name") {
                    let module_path = module_node.utf8_text(source).unwrap_or("");
                    if !module_path.is_empty() {
                        // Detect `from module import *` as a re-export
                        let full_text = child.utf8_text(source).unwrap_or("");
                        let is_reexport = full_text.contains("import *");
                        edges.push(Edge {
                            id: None,
                            source_fqn: file.to_string(),
                            target_fqn: module_path.to_string(),
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
            _ => {}
        }
    }
}

/// Collect intra-file call expressions and create Calls edges.
///
/// Handles both simple calls (`foo()`) and method calls (`obj.method()`).
/// Method calls emit a Calls edge with `receiver` and `call_type` attributes.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let caller_fqn = find_enclosing_function(child, file, source)
                    .unwrap_or_else(|| file.to_string());

                match func_node.kind() {
                    "identifier" => {
                        // Simple call: foo()
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
                    "attribute" => {
                        // Method call: obj.method() - possibly chained
                        let full_text = func_node.utf8_text(source).unwrap_or("");
                        if let Some(dot_pos) = full_text.rfind('.') {
                            let receiver = &full_text[..dot_pos];
                            let method_name = &full_text[dot_pos + 1..];
                            if !method_name.is_empty() {
                                // Detect chain position: count how many call nodes are
                                // nested in the receiver expression
                                let chain_position = count_chain_depth(func_node);
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

/// Find the enclosing function/method for a given node to determine the caller FQN.
fn find_enclosing_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    let mut class_name: Option<String> = None;

    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            if let Some(name_node) = parent.child_by_field_name("name") {
                let func_name = name_node.utf8_text(source).unwrap_or("");
                // Check if this function is inside a class
                let mut ancestor = parent.parent();
                while let Some(anc) = ancestor {
                    if anc.kind() == "class_definition" {
                        if let Some(cls_name_node) = anc.child_by_field_name("name") {
                            class_name =
                                Some(cls_name_node.utf8_text(source).unwrap_or("").to_string());
                        }
                        break;
                    }
                    ancestor = anc.parent();
                }
                return match class_name {
                    Some(cls) => Some(format!("{file}::{cls}::{func_name}")),
                    None => Some(format!("{file}::{func_name}")),
                };
            }
        }
        current = parent.parent();
    }
    // Call at module level - use file as the caller
    Some(file.to_string())
}

/// Count the chain depth of a method call by checking if the receiver
/// is itself a call expression (indicating a chained call).
/// Returns 0 for `obj.method()`, 1 for `obj.a().b()`, etc.
fn count_chain_depth(func_node: tree_sitter::Node) -> u32 {
    // The func_node is an attribute node: object.attribute
    // Check if the object part contains a call expression
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "call" {
            // The receiver is itself a call - this is a chained call
            return 1 + count_chain_depth_in_call(child);
        }
    }
    0
}

fn count_chain_depth_in_call(call_node: tree_sitter::Node) -> u32 {
    if let Some(func) = call_node.child_by_field_name("function") {
        if func.kind() == "attribute" {
            return count_chain_depth(func);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_python(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_class_methods_function_imports() {
        let source = r#"
import os
from pathlib import Path

class OrderProcessor:
    def __init__(self, db):
        self.db = db

    def process(self, order):
        validate(order)
        return self.db.save(order)

def validate(order):
    if not order.items:
        raise ValueError("Empty order")

def main():
    processor = OrderProcessor(None)
    processor.process({"items": [1]})
    validate({"items": []})
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "src/orders/processor.py", source);

        // Check nodes: class + 2 methods + 2 functions = 5
        assert_eq!(result.nodes.len(), 5);

        // Check class node
        let class_node = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class)
            .unwrap();
        assert_eq!(class_node.fqn, "src/orders/processor.py::OrderProcessor");

        // Check method nodes
        let init_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/orders/processor.py::OrderProcessor::__init__")
            .unwrap();
        assert_eq!(init_node.kind, NodeKind::Function);

        let process_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/orders/processor.py::OrderProcessor::process")
            .unwrap();
        assert_eq!(process_node.kind, NodeKind::Function);

        // Check top-level functions
        let validate_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/orders/processor.py::validate")
            .unwrap();
        assert_eq!(validate_node.kind, NodeKind::Function);

        let main_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/orders/processor.py::main")
            .unwrap();
        assert_eq!(main_node.kind, NodeKind::Function);

        // Check import edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "os"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "pathlib"));

        // Check call edges: validate() called from process and main
        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(call_edges.len() >= 2);
        assert!(call_edges.iter().any(|e| {
            e.source_fqn == "src/orders/processor.py::OrderProcessor::process"
                && e.target_fqn == "src/orders/processor.py::validate"
        }));
        assert!(call_edges.iter().any(|e| {
            e.source_fqn == "src/orders/processor.py::main"
                && e.target_fqn == "src/orders/processor.py::validate"
        }));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
class MyClass:
    def my_method(self):
        pass

def standalone():
    pass
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "src/app.py", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/app.py::MyClass"));
        assert!(fqns.contains(&"src/app.py::MyClass::my_method"));
        assert!(fqns.contains(&"src/app.py::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
def valid_function():
    pass

class Broken(:
    def method(self
        pass pass pass @@@ !!!

def another_valid():
    pass
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "broken.py", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        // The valid function should be extracted
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "broken.py::valid_function"));
    }

    #[test]
    fn test_node_line_numbers() {
        let source = "def foo():\n    pass\n\ndef bar():\n    pass\n";
        let tree = parse_python(source);
        let result = extract(&tree, "test.py", source);

        let foo = result.nodes.iter().find(|n| n.fqn == "test.py::foo").unwrap();
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 2);

        let bar = result.nodes.iter().find(|n| n.fqn == "test.py::bar").unwrap();
        assert_eq!(bar.start_line, 4);
        assert_eq!(bar.end_line, 5);
    }

    #[test]
    fn test_method_call_extraction() {
        let source = r#"
class Processor:
    def run(self):
        self.validate()
        self.save()

    def validate(self):
        pass

    def save(self):
        pass

def main():
    p = Processor()
    p.run()
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "src/proc.py", source);

        let method_calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| {
                e.kind == EdgeKind::Calls
                    && e.attributes.get("call_type").and_then(|v| v.as_str()) == Some("method")
            })
            .collect();

        // self.validate() and self.save() from run(), p.run() from main()
        assert!(!method_calls.is_empty(), "Should extract method calls");

        // Check receiver info is present
        for edge in &method_calls {
            assert!(
                edge.attributes.get("receiver").is_some(),
                "Method call edge should have receiver attribute"
            );
        }
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_python(source);
        let result = extract(&tree, "empty.py", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
