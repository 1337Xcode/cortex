//! Go AST extractor.
//!
//! Extracts structural nodes (functions, methods, structs, interfaces) and edges
//! (calls, imports) from a tree-sitter Go parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Go file.
///
/// Handles:
/// - Function declarations (`func foo() {}`)
/// - Method declarations (`func (r *Receiver) Method() {}`)
/// - Struct type declarations
/// - Interface type declarations
/// - Import declarations
/// - Intra-file call expressions resolved to definitions in the same file
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(root, file, source_bytes, &mut nodes, &mut defined_fqns);

    // Second pass: collect imports and calls
    collect_imports(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Collect function, method, struct, and interface definitions.
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
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{name}");
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
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    // Extract receiver type name
                    let receiver_type = extract_receiver_type(child, source);
                    let fqn = match receiver_type {
                        Some(ref recv) => format!("{file}::{recv}::{method_name}"),
                        None => format!("{file}::{method_name}"),
                    };
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;

                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Method,
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
            "type_declaration" => {
                // type_declaration contains type_spec children
                let mut type_cursor = child.walk();
                for type_child in child.children(&mut type_cursor) {
                    if type_child.kind() == "type_spec"
                        && let Some(name_node) = type_child.child_by_field_name("name")
                    {
                        let type_name = name_node.utf8_text(source).unwrap_or("");
                        // Determine if it's a struct or interface
                        let type_node = type_child.child_by_field_name("type");
                        let kind = match type_node.map(|n| n.kind()) {
                            Some("struct_type") => NodeKind::Class,
                            Some("interface_type") => NodeKind::Interface,
                            _ => NodeKind::Type,
                        };
                        let fqn = format!("{file}::{type_name}");
                        let start_line = type_child.start_position().row as u32 + 1;
                        let end_line = type_child.end_position().row as u32 + 1;

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

                        defined_fqns.push((type_name.to_string(), fqn));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract the receiver type name from a method declaration.
/// Handles both pointer receivers `(r *Type)` and value receivers `(r Type)`.
fn extract_receiver_type(method_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let receiver = method_node.child_by_field_name("receiver")?;
    // The receiver is a parameter_list containing a parameter_declaration
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            // The type field contains the receiver type (possibly a pointer_type)
            if let Some(type_node) = child.child_by_field_name("type") {
                return extract_type_name(type_node, source);
            }
        }
    }
    None
}

/// Extract the base type name from a type node, stripping pointer indirection.
fn extract_type_name(type_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match type_node.kind() {
        "pointer_type" => {
            // *Type -> get the inner type
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return Some(child.utf8_text(source).unwrap_or("").to_string());
                }
            }
            None
        }
        "type_identifier" => Some(type_node.utf8_text(source).unwrap_or("").to_string()),
        _ => Some(type_node.utf8_text(source).unwrap_or("").to_string()),
    }
}

/// Collect import declarations and create Imports edges.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            // Can be a single import_spec or an import_spec_list
            collect_import_specs(child, file, source, edges);
        }
    }
}

/// Recursively collect import specs from an import declaration.
fn collect_import_specs(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let raw = path_node.utf8_text(source).unwrap_or("");
                    let import_path = raw.trim_matches('"');
                    if !import_path.is_empty() {
                        edges.push(Edge {
                            id: None,
                            source_fqn: file.to_string(),
                            target_fqn: import_path.to_string(),
                            kind: EdgeKind::Imports,
                            confidence: 1.0,
                            edge_source: crate::store::confidence::EdgeSource::AstDirect,
                            attributes: json!({}),
                        });
                    }
                }
            }
            "import_spec_list" => {
                collect_import_specs(child, file, source, edges);
            }
            _ => {}
        }
    }
}

/// Collect intra-file call expressions and create Calls edges.
///
/// Handles both simple calls (`foo()`) and selector expression calls (`obj.Method()`).
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let caller_fqn =
                find_enclosing_function(child, file, source).unwrap_or_else(|| file.to_string());

            match func_node.kind() {
                "identifier" => {
                    let call_name = func_node.utf8_text(source).unwrap_or("");
                    if let Some((_, target_fqn)) =
                        defined_fqns.iter().find(|(name, _)| name == call_name)
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
                "selector_expression" => {
                    // obj.Method() - selector_expression has operand and field fields
                    let full_text = func_node.utf8_text(source).unwrap_or("");
                    if let Some(dot_pos) = full_text.rfind('.') {
                        let receiver = &full_text[..dot_pos];
                        let method_name = &full_text[dot_pos + 1..];
                        if !method_name.is_empty() {
                            let chain_position = count_chain_depth_go(func_node);
                            let (target_fqn, confidence) = if let Some((_, fqn)) =
                                defined_fqns.iter().find(|(name, _)| name == method_name)
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

        // Recurse into children
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

/// Find the enclosing function/method for a given node to determine the caller FQN.
fn find_enclosing_function(node: tree_sitter::Node, file: &str, source: &[u8]) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let func_name = name_node.utf8_text(source).unwrap_or("");
                    return Some(format!("{file}::{func_name}"));
                }
            }
            "method_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    let receiver_type = extract_receiver_type(parent, source);
                    return match receiver_type {
                        Some(recv) => Some(format!("{file}::{recv}::{method_name}")),
                        None => Some(format!("{file}::{method_name}")),
                    };
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    // Call at package level
    Some(file.to_string())
}

fn count_chain_depth_go(func_node: tree_sitter::Node) -> u32 {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            return 1 + count_chain_depth_in_call_go(child);
        }
    }
    0
}

fn count_chain_depth_in_call_go(call_node: tree_sitter::Node) -> u32 {
    if let Some(func) = call_node.child_by_field_name("function")
        && func.kind() == "selector_expression"
    {
        return count_chain_depth_go(func);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_go(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_functions_structs_interfaces_imports() {
        let source = r#"package main

import (
    "fmt"
    "net/http"
)

type Server struct {
    port int
    host string
}

type Handler interface {
    ServeHTTP(w http.ResponseWriter, r *http.Request)
}

func NewServer(port int) *Server {
    return &Server{port: port, host: "localhost"}
}

func (s *Server) Start() error {
    addr := fmt.Sprintf("%s:%d", s.host, s.port)
    return http.ListenAndServe(addr, nil)
}

func (s *Server) Stop() {
    fmt.Println("stopping")
}

func main() {
    srv := NewServer(8080)
    srv.Start()
}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "cmd/server/main.go", source);

        // Nodes: Server struct + Handler interface + NewServer + Start + Stop + main = 6
        assert_eq!(result.nodes.len(), 6);

        // Check struct
        let server_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "cmd/server/main.go::Server")
            .unwrap();
        assert_eq!(server_node.kind, NodeKind::Class);

        // Check interface
        let handler_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "cmd/server/main.go::Handler")
            .unwrap();
        assert_eq!(handler_node.kind, NodeKind::Interface);

        // Check function
        let new_server = result
            .nodes
            .iter()
            .find(|n| n.fqn == "cmd/server/main.go::NewServer")
            .unwrap();
        assert_eq!(new_server.kind, NodeKind::Function);

        // Check methods with receiver type in FQN
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "cmd/server/main.go::Server::Start")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "cmd/server/main.go::Server::Stop")
        );

        // Check main function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "cmd/server/main.go::main")
        );

        // Check import edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "fmt"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "net/http"));

        // Check call edges: NewServer called from main
        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(call_edges.iter().any(|e| {
            e.source_fqn == "cmd/server/main.go::main"
                && e.target_fqn == "cmd/server/main.go::NewServer"
        }));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"package main

type MyStruct struct {}

func (m *MyStruct) DoWork() {}

func standalone() {}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "pkg/worker.go", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"pkg/worker.go::MyStruct"));
        assert!(fqns.contains(&"pkg/worker.go::MyStruct::DoWork"));
        assert!(fqns.contains(&"pkg/worker.go::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"package main

func validFunc() {
    fmt.Println("hello")
}

func broken( {{{ invalid !!!

func anotherValid() {}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "broken.go", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        assert!(result.nodes.iter().any(|n| n.fqn == "broken.go::validFunc"));
    }

    #[test]
    fn test_single_import() {
        let source = r#"package main

import "fmt"

func hello() {
    fmt.Println("hi")
}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "hello.go", source);

        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 1);
        assert_eq!(import_edges[0].target_fqn, "fmt");
    }

    #[test]
    fn test_empty_file() {
        let source = "package main\n";
        let tree = parse_go(source);
        let result = extract(&tree, "empty.go", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    /// Validates: Requirements 9.4
    /// Verify that standalone functions get NodeKind::Function and methods get NodeKind::Method.
    #[test]
    fn test_nodekind_method_vs_function() {
        let source = r#"package main

type Calculator struct {
    value int
}

func (c *Calculator) Add(a, b int) int {
    return a + b
}

func (c *Calculator) Subtract(a, b int) int {
    return a - b
}

func standaloneHelper() int {
    return 42
}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "test_nodekind.go", source);

        // Standalone function should be NodeKind::Function
        let helper = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::standaloneHelper"))
            .expect("standaloneHelper should be extracted");
        assert_eq!(
            helper.kind,
            NodeKind::Function,
            "Standalone function should have NodeKind::Function"
        );

        // Methods with receiver should be NodeKind::Method
        let add_method = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::Calculator::Add"))
            .expect("Calculator.Add should be extracted");
        assert_eq!(
            add_method.kind,
            NodeKind::Method,
            "Method with receiver should have NodeKind::Method"
        );

        let subtract_method = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::Calculator::Subtract"))
            .expect("Calculator.Subtract should be extracted");
        assert_eq!(
            subtract_method.kind,
            NodeKind::Method,
            "Method with receiver should have NodeKind::Method"
        );
    }

    /// Validates: Requirements 9.4
    /// Verify that method FQNs include the parent type name in the format file::TypeName::method_name.
    #[test]
    fn test_method_fqn_includes_parent_type() {
        let source = r#"package main

type MyService struct {
    data []string
}

func (s *MyService) Process() {}

func (s MyService) Validate() bool {
    return true
}

func FreeFunction() {}
"#;
        let tree = parse_go(source);
        let result = extract(&tree, "pkg/service.go", source);

        // Method FQNs should include parent type (receiver type)
        let process = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("Process"))
            .expect("Process method should be extracted");
        assert_eq!(
            process.fqn, "pkg/service.go::MyService::Process",
            "Method FQN should be file::ReceiverType::method_name"
        );

        let validate = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("Validate"))
            .expect("Validate method should be extracted");
        assert_eq!(
            validate.fqn, "pkg/service.go::MyService::Validate",
            "Method FQN should be file::ReceiverType::method_name (value receiver)"
        );

        // Standalone function FQN should NOT include a type name
        let free_fn = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.fqn.contains("FreeFunction"))
            .expect("FreeFunction should be extracted");
        assert_eq!(
            free_fn.fqn, "pkg/service.go::FreeFunction",
            "Function FQN should be file::function_name without type"
        );
    }
}
