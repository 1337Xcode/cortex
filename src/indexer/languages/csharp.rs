//! C# AST extractor.
//!
//! Extracts structural nodes (classes, interfaces, methods, namespaces) and edges
//! (using statements) from a tree-sitter C# parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed C# file.
///
/// Handles:
/// - Class declarations
/// - Interface declarations
/// - Method declarations (inside classes/interfaces)
/// - Namespace declarations
/// - Using directives (imports)
/// - Member access calls (obj.Method()) with receiver info
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut defined_fqns: Vec<(String, String)> = Vec::new();
    collect_definitions_with_fqns(
        root,
        file,
        source_bytes,
        None,
        &mut nodes,
        &mut defined_fqns,
    );
    collect_usings(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Recursively collect namespace, class, interface, and method definitions (with FQN tracking).
fn collect_definitions_with_fqns(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let ns_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match parent_name {
                        Some(p) => format!("{file}::{p}::{ns_name}"),
                        None => format!("{file}::{ns_name}"),
                    };
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;
                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Module,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({}),
                    });
                    defined_fqns.push((ns_name.to_string(), fqn));
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions_with_fqns(
                            body,
                            file,
                            source,
                            Some(ns_name),
                            nodes,
                            defined_fqns,
                        );
                    }
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match parent_name {
                        Some(p) => format!("{file}::{p}::{cls_name}"),
                        None => format!("{file}::{cls_name}"),
                    };
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
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions_with_fqns(
                            body,
                            file,
                            source,
                            Some(cls_name),
                            nodes,
                            defined_fqns,
                        );
                    }
                }
            }
            "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let iface_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match parent_name {
                        Some(p) => format!("{file}::{p}::{iface_name}"),
                        None => format!("{file}::{iface_name}"),
                    };
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
                    defined_fqns.push((iface_name.to_string(), fqn));
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions_with_fqns(
                            body,
                            file,
                            source,
                            Some(iface_name),
                            nodes,
                            defined_fqns,
                        );
                    }
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match parent_name {
                        Some(p) => format!("{file}::{p}::{method_name}"),
                        None => format!("{file}::{method_name}"),
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
                    defined_fqns.push((method_name.to_string(), fqn));
                }
            }
            _ => {
                collect_definitions_with_fqns(
                    child,
                    file,
                    source,
                    parent_name,
                    nodes,
                    defined_fqns,
                );
            }
        }
    }
}

/// Collect member access calls (obj.Method()) and simple invocations.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "invocation_expression"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let caller_fqn =
                find_enclosing_method_cs(child, file, source).unwrap_or_else(|| file.to_string());
            match func_node.kind() {
                "identifier" => {
                    let call_name = func_node.utf8_text(source).unwrap_or("");
                    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(n, _)| n == call_name)
                        && caller_fqn != *target_fqn
                    {
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
                "qualified_name" => {
                    // Namespace.Class.Method() qualified call
                    let full_text = func_node.utf8_text(source).unwrap_or("");
                    if let Some(dot_pos) = full_text.rfind('.') {
                        let qualifier = &full_text[..dot_pos];
                        let method_name = &full_text[dot_pos + 1..];
                        if !method_name.is_empty() {
                            let (target_fqn, confidence) = if let Some((_, fqn)) =
                                defined_fqns.iter().find(|(n, _)| n == method_name)
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
                "member_access_expression" => {
                    let full_text = func_node.utf8_text(source).unwrap_or("");
                    if let Some(dot_pos) = full_text.rfind('.') {
                        let receiver = &full_text[..dot_pos];
                        let method_name = &full_text[dot_pos + 1..];
                        if !method_name.is_empty() {
                            let (target_fqn, confidence) = if let Some((_, fqn)) =
                                defined_fqns.iter().find(|(n, _)| n == method_name)
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
                                attributes: json!({"receiver": receiver, "call_type": "method"}),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

fn find_enclosing_method_cs(node: tree_sitter::Node, file: &str, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_declaration"
            && let Some(name_node) = parent.child_by_field_name("name")
        {
            let method_name = name_node.utf8_text(source).unwrap_or("");
            let class_name = find_enclosing_class_cs(parent, source);
            return match class_name {
                Some(cls) => Some(format!("{file}::{cls}::{method_name}")),
                None => Some(format!("{file}::{method_name}")),
            };
        }
        current = parent.parent();
    }
    Some(file.to_string())
}

fn find_enclosing_class_cs(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "class_declaration"
            && let Some(name_node) = parent.child_by_field_name("name")
        {
            return Some(name_node.utf8_text(source).unwrap_or("").to_string());
        }
        current = parent.parent();
    }
    None
}

/// Collect using directives and create Imports edges.
fn collect_usings(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "using_directive" {
            // Extract the namespace from the using directive text
            let text = child.utf8_text(source).unwrap_or("");
            let ns = text
                .trim_start_matches("using ")
                .trim_start_matches("static ")
                .trim_end_matches(';')
                .trim();
            if !ns.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: file.to_string(),
                    target_fqn: ns.to_string(),
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    attributes: json!({}),
                });
            }
        } else if child.child_count() > 0 {
            // Recurse to find using directives inside namespace declarations etc.
            collect_usings(child, file, source, edges);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_csharp(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_classes_interfaces_methods_usings() {
        let source = r#"using System;
using System.Collections.Generic;

namespace MyApp.Services
{
    public interface IOrderService
    {
        void ProcessOrder(Order order);
        Order GetOrder(string id);
    }

    public class OrderService : IOrderService
    {
        public void ProcessOrder(Order order)
        {
            Console.WriteLine("Processing");
        }

        public Order GetOrder(string id)
        {
            return null;
        }
    }
}
"#;
        let tree = parse_csharp(source);
        let result = extract(&tree, "Services/OrderService.cs", source);

        // Check namespace
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Services/OrderService.cs::MyApp.Services"
                    && n.kind == NodeKind::Module)
        );

        // Check interface
        assert!(result.nodes.iter().any(|n| n.fqn
            == "Services/OrderService.cs::MyApp.Services::IOrderService"
            && n.kind == NodeKind::Interface));

        // Check class
        assert!(result.nodes.iter().any(|n| n.fqn
            == "Services/OrderService.cs::MyApp.Services::OrderService"
            && n.kind == NodeKind::Class));

        // Check methods
        assert!(result.nodes.iter().any(|n| n.fqn
            == "Services/OrderService.cs::OrderService::ProcessOrder"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "Services/OrderService.cs::OrderService::GetOrder"
            && n.kind == NodeKind::Function));

        // Check using edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "System"));
        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "System.Collections.Generic")
        );
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
namespace MyNamespace
{
    public class MyClass
    {
        public void MyMethod() {}
    }
}
"#;
        let tree = parse_csharp(source);
        let result = extract(&tree, "src/App.cs", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/App.cs::MyNamespace"));
        assert!(fqns.contains(&"src/App.cs::MyNamespace::MyClass"));
        assert!(fqns.contains(&"src/App.cs::MyClass::MyMethod"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
using System;

public class ValidClass
{
    public void ValidMethod() {}
}

public class Broken {{{ invalid !!!
"#;
        let tree = parse_csharp(source);
        let result = extract(&tree, "Broken.cs", source);

        assert!(!result.nodes.is_empty());
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Broken.cs::ValidClass")
        );
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_csharp(source);
        let result = extract(&tree, "Empty.cs", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
