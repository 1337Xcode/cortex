//! C AST extractor.
//!
//! Extracts structural nodes (functions, structs) and edges
//! (#include statements) from a tree-sitter C parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed C file.
///
/// Handles:
/// - Function definitions
/// - Struct specifiers
/// - #include preprocessor directives
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    collect_definitions(root, file, source_bytes, &mut nodes);
    collect_includes(root, file, source_bytes, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Collect function and struct definitions at the top level.
fn collect_definitions(node: tree_sitter::Node, file: &str, source: &[u8], nodes: &mut Vec<Node>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = extract_function_name(child, source) {
                    let fqn = format!("{file}::{name}");
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

                    nodes.push(Node {
                        fqn,
                        kind: NodeKind::Function,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({}),
                    });
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
                            fqn,
                            kind: NodeKind::Class,
                            file: file.to_string(),
                            start_line,
                            end_line,
                            file_hash: String::new(),
                            indexed_at: 0,
                            attributes: json!({}),
                        });
                    }
                }
            }
            "declaration" => {
                // Structs can appear inside declarations (e.g., `struct Foo { ... };`)
                collect_structs_in_declaration(child, file, source, nodes);
            }
            _ => {}
        }
    }
}

/// Look for struct specifiers inside a declaration node.
fn collect_structs_in_declaration(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "struct_specifier"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let struct_name = name_node.utf8_text(source).unwrap_or("");
            if !struct_name.is_empty() {
                // Check it has a body (not just a forward declaration reference)
                if child.child_by_field_name("body").is_some() {
                    let fqn = format!("{file}::{struct_name}");
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

                    nodes.push(Node {
                        fqn,
                        kind: NodeKind::Class,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({}),
                    });
                }
            }
        }
    }
}

/// Extract the function name from a function_definition node.
/// The name is inside a "declarator" field which may be a "function_declarator"
/// containing an "identifier".
fn extract_function_name(func_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let declarator = func_node.child_by_field_name("declarator")?;
    extract_name_from_declarator(declarator, source)
}

/// Recursively extract the function name from a declarator node.
fn extract_name_from_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "function_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
        "identifier" => {
            let name = node.utf8_text(source).unwrap_or("");
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
        "pointer_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
        "parenthesized_declarator" => {
            // e.g., (*func_ptr)(args)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "pointer_declarator" || child.kind() == "identifier" {
                    return extract_name_from_declarator(child, source);
                }
            }
            None
        }
        _ => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_name_from_declarator(inner, source);
            }
            None
        }
    }
}

/// Collect #include directives and create Imports edges.
fn collect_includes(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include"
            && let Some(path_node) = child.child_by_field_name("path")
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_c(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_functions_structs_includes() {
        let source = r#"#include <stdio.h>
#include "utils.h"

struct Point {
    int x;
    int y;
};

struct Config {
    int port;
    const char* host;
};

int add(int a, int b) {
    return a + b;
}

void print_point(struct Point* p) {
    printf("(%d, %d)\n", p->x, p->y);
}

int main() {
    struct Point p = {1, 2};
    print_point(&p);
    printf("%d\n", add(1, 2));
    return 0;
}
"#;
        let tree = parse_c(source);
        let result = extract(&tree, "src/main.c", source);

        // Check functions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/main.c::add" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/main.c::print_point" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/main.c::main" && n.kind == NodeKind::Function)
        );

        // Check structs
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/main.c::Point" && n.kind == NodeKind::Class)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/main.c::Config" && n.kind == NodeKind::Class)
        );

        // Check include edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "stdio.h"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "utils.h"));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
struct MyStruct {
    int value;
};

void my_function() {}
"#;
        let tree = parse_c(source);
        let result = extract(&tree, "src/lib.c", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/lib.c::MyStruct"));
        assert!(fqns.contains(&"src/lib.c::my_function"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
void valid_func() {}

struct Broken {{{ invalid !!!

void another_valid() {}
"#;
        let tree = parse_c(source);
        let result = extract(&tree, "broken.c", source);

        assert!(!result.nodes.is_empty());
        assert!(result.nodes.iter().any(|n| n.fqn == "broken.c::valid_func"));
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_c(source);
        let result = extract(&tree, "empty.c", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }
}
