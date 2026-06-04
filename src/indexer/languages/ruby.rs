//! Ruby AST extractor.
//!
//! Extracts structural nodes (methods, classes, modules) and edges
//! (require statements) from a tree-sitter Ruby parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Ruby file.
///
/// Handles:
/// - Method definitions (`def`)
/// - Class definitions
/// - Module definitions
/// - require/require_relative statements
/// - Method calls with receiver info (obj.method())
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
    collect_requires(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Recursively collect method, class, and module definitions (with FQN tracking).
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
            "method" => {
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
            "class" => {
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
            "module" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let mod_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match parent_name {
                        Some(p) => format!("{file}::{p}::{mod_name}"),
                        None => format!("{file}::{mod_name}"),
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
                    defined_fqns.push((mod_name.to_string(), fqn));
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions_with_fqns(
                            body,
                            file,
                            source,
                            Some(mod_name),
                            nodes,
                            defined_fqns,
                        );
                    }
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

/// Collect method calls with receiver info (obj.method()).
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
            let caller_fqn =
                find_enclosing_method_rb(child, file, source).unwrap_or_else(|| file.to_string());

            // Check for receiver (obj.method)
            let receiver_node = child.child_by_field_name("receiver");
            let method_node = child.child_by_field_name("method");

            if let (Some(recv), Some(meth)) = (receiver_node, method_node) {
                let receiver = recv.utf8_text(source).unwrap_or("");
                let method_name = meth.utf8_text(source).unwrap_or("");
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
                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                        attributes: json!({"receiver": receiver, "call_type": "method"}),
                    });
                }
            } else if let Some(meth) = method_node {
                // Simple call without receiver
                let call_name = meth.utf8_text(source).unwrap_or("");
                if let Some((_, target_fqn)) = defined_fqns.iter().find(|(n, _)| n == call_name)
                    && caller_fqn != *target_fqn
                {
                    edges.push(Edge {
                        id: None,
                        source_fqn: caller_fqn,
                        target_fqn: target_fqn.clone(),
                        kind: EdgeKind::Calls,
                        confidence: 1.0,
                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                        attributes: json!({}),
                    });
                }
            }
        }
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

fn find_enclosing_method_rb(node: tree_sitter::Node, file: &str, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method"
            && let Some(name_node) = parent.child_by_field_name("name")
        {
            let method_name = name_node.utf8_text(source).unwrap_or("");
            let class_name = find_enclosing_class_rb(parent, source);
            return match class_name {
                Some(cls) => Some(format!("{file}::{cls}::{method_name}")),
                None => Some(format!("{file}::{method_name}")),
            };
        }
        current = parent.parent();
    }
    Some(file.to_string())
}

fn find_enclosing_class_rb(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if (parent.kind() == "class" || parent.kind() == "module")
            && let Some(name_node) = parent.child_by_field_name("name")
        {
            return Some(name_node.utf8_text(source).unwrap_or("").to_string());
        }
        current = parent.parent();
    }
    None
}

/// Collect require/require_relative calls and create Imports edges.
fn collect_requires(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // Check if this is a require or require_relative call
            if let Some(method_node) = child.child_by_field_name("method") {
                let method_name = method_node.utf8_text(source).unwrap_or("");
                if method_name == "require" || method_name == "require_relative" {
                    // Extract the argument (the required path)
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let arg_text = extract_first_string_arg(args, source);
                        if let Some(path) = arg_text {
                            edges.push(Edge {
                                id: None,
                                source_fqn: file.to_string(),
                                target_fqn: path,
                                kind: EdgeKind::Imports,
                                confidence: 1.0,
                                edge_source: crate::store::confidence::EdgeSource::AstDirect,
                                attributes: json!({}),
                            });
                        }
                    }
                }
            }
        } else if child.child_count() > 0 {
            collect_requires(child, file, source, edges);
        }
    }
}

/// Extract the first string argument from an argument_list node.
fn extract_first_string_arg(args_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        if child.kind() == "argument_list" {
            // Recurse into argument_list
            return extract_first_string_arg(child, source);
        }
        if child.kind() == "string" || child.kind() == "string_literal" {
            let raw = child.utf8_text(source).unwrap_or("");
            // Strip quotes
            let path = raw.trim_matches('"').trim_matches('\'');
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ruby(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_classes_modules_methods_requires() {
        let source = r#"require 'json'
require_relative 'helpers/utils'

module Validators
  def self.validate(data)
    data.is_a?(Hash)
  end
end

class OrderProcessor
  def initialize(db)
    @db = db
  end

  def process(order)
    validate(order)
    @db.save(order)
  end

  def validate(order)
    raise "Empty" if order.empty?
  end
end

def standalone_helper
  puts "hello"
end
"#;
        let tree = parse_ruby(source);
        let result = extract(&tree, "lib/order_processor.rb", source);

        // Check module
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "lib/order_processor.rb::Validators" && n.kind == NodeKind::Module
            )
        );

        // Check class
        assert!(result.nodes.iter().any(
            |n| n.fqn == "lib/order_processor.rb::OrderProcessor" && n.kind == NodeKind::Class
        ));

        // Check methods inside class
        assert!(result.nodes.iter().any(|n| n.fqn
            == "lib/order_processor.rb::OrderProcessor::initialize"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "lib/order_processor.rb::OrderProcessor::process"
            && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "lib/order_processor.rb::OrderProcessor::validate"
            && n.kind == NodeKind::Function));

        // Check standalone method
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/order_processor.rb::standalone_helper"
                    && n.kind == NodeKind::Function)
        );

        // Check require edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "json"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "helpers/utils"));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
module MyModule
  class MyClass
    def my_method
    end
  end
end

def standalone
end
"#;
        let tree = parse_ruby(source);
        let result = extract(&tree, "app/models/thing.rb", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"app/models/thing.rb::MyModule"));
        assert!(fqns.contains(&"app/models/thing.rb::MyModule::MyClass"));
        assert!(fqns.contains(&"app/models/thing.rb::MyClass::my_method"));
        assert!(fqns.contains(&"app/models/thing.rb::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
def valid_method
  puts "hello"
end

class Broken {{{ invalid !!!

def another_valid
  puts "world"
end
"#;
        let tree = parse_ruby(source);
        let result = extract(&tree, "broken.rb", source);

        assert!(!result.nodes.is_empty());
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "broken.rb::valid_method")
        );
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_ruby(source);
        let result = extract(&tree, "empty.rb", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
