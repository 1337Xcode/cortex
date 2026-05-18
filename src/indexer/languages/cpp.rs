//! C++ AST extractor.
//!
//! Extracts structural nodes (functions, methods, classes, structs) and edges
//! (#include statements) from a tree-sitter C++ parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed C++ file.
///
/// Handles:
/// - Function definitions (top-level and member functions)
/// - Class specifiers
/// - Struct specifiers
/// - #include preprocessor directives
/// - Member function calls (obj.method()) and scope resolution calls (Cls::method())
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut defined_fqns: Vec<(String, String)> = Vec::new();
    collect_definitions_with_fqns(root, file, source_bytes, None, &mut nodes, &mut defined_fqns);
    collect_includes(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Recursively collect function, class, and struct definitions (with FQN tracking).
fn collect_definitions_with_fqns(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: Option<&str>,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = extract_function_name(child, source) {
                    let fqn = match class_name {
                        Some(cls) => format!("{file}::{cls}::{name}"),
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
                    defined_fqns.push((name, fqn));
                }
            }
            "class_specifier" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = name_node.utf8_text(source).unwrap_or("");
                    if !cls_name.is_empty() {
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
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_definitions_with_fqns(body, file, source, Some(cls_name), nodes, defined_fqns);
                        }
                    }
                }
            }
            "struct_specifier" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let struct_name = name_node.utf8_text(source).unwrap_or("");
                    if !struct_name.is_empty() {
                        let fqn = format!("{file}::{struct_name}");
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
                        defined_fqns.push((struct_name.to_string(), fqn));
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_definitions_with_fqns(body, file, source, Some(struct_name), nodes, defined_fqns);
                        }
                    }
                }
            }
            _ => {
                collect_definitions_with_fqns(child, file, source, class_name, nodes, defined_fqns);
            }
        }
    }
}

/// Collect call expressions: member calls (obj.method()) and scope resolution (Cls::method()).
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
                let caller_fqn = find_enclosing_function_cpp(child, file, source)
                    .unwrap_or_else(|| file.to_string());
                let full_text = func_node.utf8_text(source).unwrap_or("");

                // Member call: obj.method or obj->method
                if full_text.contains('.') || full_text.contains("->") {
                    let sep = if full_text.contains("->") { "->" } else { "." };
                    if let Some(dot_pos) = full_text.rfind(sep) {
                        let receiver = &full_text[..dot_pos];
                        let method_name = &full_text[dot_pos + sep.len()..];
                        if !method_name.is_empty() {
                            let (target_fqn, confidence) =
                                if let Some((_, fqn)) = defined_fqns.iter().find(|(n, _)| n == method_name) {
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
                        collect_calls(child, file, source, defined_fqns, edges);
                        continue;
                    }
                }

                // Scope resolution: Cls::method
                if full_text.contains("::") {
                    if let Some(sep_pos) = full_text.rfind("::") {
                        let qualifier = &full_text[..sep_pos];
                        let method_name = &full_text[sep_pos + 2..];
                        if !method_name.is_empty() {
                            let (target_fqn, confidence) =
                                if let Some((_, fqn)) = defined_fqns.iter().find(|(n, _)| n == method_name) {
                                    (fqn.clone(), 1.0_f64)
                                } else {
                                    (full_text.to_string(), 0.0_f64)
                                };
                            edges.push(Edge {
                                id: None,
                                source_fqn: caller_fqn,
                                target_fqn,
                                kind: EdgeKind::Calls,
                                confidence,
                                attributes: json!({"receiver": qualifier, "call_type": "qualified"}),
                            });
                        }
                        collect_calls(child, file, source, defined_fqns, edges);
                        continue;
                    }
                }

                // Simple identifier call
                if func_node.kind() == "identifier" {
                    let call_name = full_text;
                    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(n, _)| n == call_name) {
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
            }
        }
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

fn find_enclosing_function_cpp(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            if let Some(name) = extract_function_name(parent, source) {
                return Some(format!("{file}::{name}"));
            }
        }
        current = parent.parent();
    }
    Some(file.to_string())
}




/// Extract the function name from a function_definition node.
/// The name is typically inside a "declarator" field which may be a
/// "function_declarator" containing an "identifier" or "field_identifier",
/// or a qualified_identifier for class::method definitions.
fn extract_function_name(func_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let declarator = func_node.child_by_field_name("declarator")?;
    extract_name_from_declarator(declarator, source)
}

/// Recursively extract the function name from a declarator node.
fn extract_name_from_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "function_declarator" => {
            // The declarator field of a function_declarator contains the name
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
        "identifier" | "field_identifier" => {
            let name = node.utf8_text(source).unwrap_or("");
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
        "qualified_identifier" => {
            // For Class::method style, extract just the method name (rightmost identifier)
            // We look for the "name" field which is the rightmost part
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node.utf8_text(source).unwrap_or("");
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
            // Fallback: get the full text
            let text = node.utf8_text(source).unwrap_or("");
            if text.contains("::") {
                text.rsplit("::").next().map(|s| s.to_string())
            } else if !text.is_empty() {
                Some(text.to_string())
            } else {
                None
            }
        }
        "pointer_declarator" | "reference_declarator" => {
            // *func or &func - look inside for the actual declarator
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
        _ => {
            // Try child_by_field_name("declarator") as a fallback
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
    }
}

/// Collect #include directives and create Imports edges.
fn collect_includes(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include" {
            if let Some(path_node) = child.child_by_field_name("path") {
                let raw = path_node.utf8_text(source).unwrap_or("");
                // Strip quotes or angle brackets
                let include_path = raw
                    .trim_matches('"')
                    .trim_start_matches('<')
                    .trim_end_matches('>');
                if !include_path.is_empty() {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: include_path.to_string(),
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cpp(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_classes_structs_functions_includes() {
        let source = r#"#include <iostream>
#include "server.h"

class Server {
public:
    void start() {
        std::cout << "starting" << std::endl;
    }

    void stop() {
        std::cout << "stopping" << std::endl;
    }
};

struct Config {
    int port;
    const char* host;
};

int main() {
    Server srv;
    srv.start();
    return 0;
}
"#;
        let tree = parse_cpp(source);
        let result = extract(&tree, "src/main.cpp", source);

        // Check class
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.cpp::Server" && n.kind == NodeKind::Class));

        // Check struct
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.cpp::Config" && n.kind == NodeKind::Class));

        // Check methods inside class
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.cpp::Server::start" && n.kind == NodeKind::Function));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.cpp::Server::stop" && n.kind == NodeKind::Function));

        // Check top-level function
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "src/main.cpp::main" && n.kind == NodeKind::Function));

        // Check include edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "iostream"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "server.h"));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
class MyClass {
public:
    void myMethod() {}
};

void standalone() {}
"#;
        let tree = parse_cpp(source);
        let result = extract(&tree, "src/app.cpp", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/app.cpp::MyClass"));
        assert!(fqns.contains(&"src/app.cpp::MyClass::myMethod"));
        assert!(fqns.contains(&"src/app.cpp::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
void validFunc() {}

class Broken {{{ invalid !!!

void anotherValid() {}
"#;
        let tree = parse_cpp(source);
        let result = extract(&tree, "broken.cpp", source);

        assert!(!result.nodes.is_empty());
        assert!(result
            .nodes
            .iter()
            .any(|n| n.fqn == "broken.cpp::validFunc"));
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_cpp(source);
        let result = extract(&tree, "empty.cpp", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
