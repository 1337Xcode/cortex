//! Java AST extractor.
//!
//! Extracts structural nodes (classes, interfaces, methods) and edges
//! (calls, imports) from a tree-sitter Java parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Java file.
///
/// Handles:
/// - Class declarations
/// - Interface declarations
/// - Method declarations (inside classes/interfaces)
/// - Import declarations
/// - Intra-file method invocations resolved to definitions in the same file
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(
        root,
        file,
        source_bytes,
        None,
        &mut nodes,
        &mut defined_fqns,
    );

    // Second pass: collect imports and calls
    collect_imports(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Recursively collect class, interface, and method definitions.
fn collect_definitions(
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
            "class_declaration" => {
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

                    // Recurse into class body to find methods and inner classes
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions(
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
                    let fqn = format!("{file}::{iface_name}");
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

                    // Recurse into interface body to find method signatures
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_definitions(
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
                    let fqn = match class_name {
                        Some(cls) => format!("{file}::{cls}::{method_name}"),
                        None => format!("{file}::{method_name}"),
                    };
                    let kind = if class_name.is_some() {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
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

                    defined_fqns.push((method_name.to_string(), fqn));
                }
            }
            "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let ctor_name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match class_name {
                        Some(cls) => format!("{file}::{cls}::{ctor_name}"),
                        None => format!("{file}::{ctor_name}"),
                    };
                    let kind = if class_name.is_some() {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
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

                    defined_fqns.push((ctor_name.to_string(), fqn));
                }
            }
            _ => {}
        }
    }
}

/// Collect import declarations and create Imports edges.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            // The import path is in a scoped_identifier or identifier child
            let import_text = child.utf8_text(source).unwrap_or("");
            // Strip "import " prefix and trailing ";"
            let path = import_text
                .trim_start_matches("import ")
                .trim_start_matches("static ")
                .trim_end_matches(';')
                .trim();
            if !path.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: file.to_string(),
                    target_fqn: path.to_string(),
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({}),
                });
            }
        }
    }
}

/// Collect intra-file method invocations and create Calls edges.
///
/// Handles both simple calls (no object) and method calls with receiver (object field).
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "method_invocation"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let call_name = name_node.utf8_text(source).unwrap_or("");
            let caller_fqn =
                find_enclosing_method(child, file, source).unwrap_or_else(|| file.to_string());

            let object_node = child.child_by_field_name("object");

            if let Some(obj_node) = object_node {
                // Method call with receiver: obj.method()
                let receiver = obj_node.utf8_text(source).unwrap_or("");
                let (target_fqn, confidence) = if let Some((_, fqn)) =
                    defined_fqns.iter().find(|(name, _)| name == call_name)
                {
                    (fqn.clone(), 1.0_f64)
                } else {
                    (call_name.to_string(), 0.0_f64)
                };
                if caller_fqn != target_fqn {
                    edges.push(Edge {
                        id: None,
                        source_fqn: caller_fqn,
                        target_fqn,
                        kind: EdgeKind::Calls,
                        confidence,
                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                        attributes: json!({
                            "receiver": receiver,
                            "call_type": "method"
                        }),
                    });
                }
            } else {
                // Simple call within same class
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
        }

        // Recurse into children
        collect_calls(child, file, source, defined_fqns, edges);
    }
}

/// Find the enclosing method for a given node to determine the caller FQN.
fn find_enclosing_method(node: tree_sitter::Node, file: &str, source: &[u8]) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    let class_name = find_enclosing_class(parent, source);
                    return match class_name {
                        Some(cls) => Some(format!("{file}::{cls}::{method_name}")),
                        None => Some(format!("{file}::{method_name}")),
                    };
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    // Call at file level (unlikely in Java but handle gracefully)
    Some(file.to_string())
}

/// Find the enclosing class or interface for a method node.
fn find_enclosing_class(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if (parent.kind() == "class_declaration" || parent.kind() == "interface_declaration")
            && let Some(name_node) = parent.child_by_field_name("name")
        {
            return Some(name_node.utf8_text(source).unwrap_or("").to_string());
        }
        current = parent.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_java(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_classes_interfaces_methods_imports() {
        let source = r#"package com.example.service;

import java.util.List;
import java.util.Optional;

public interface OrderRepository {
    Optional<Order> findById(String id);
    List<Order> findAll();
}

public class OrderService {
    private final OrderRepository repository;

    public OrderService(OrderRepository repository) {
        this.repository = repository;
    }

    public Order getOrder(String id) {
        return findById(id).orElseThrow();
    }

    public List<Order> listOrders() {
        return findAll();
    }

    private Optional<Order> findById(String id) {
        return repository.findById(id);
    }

    private List<Order> findAll() {
        return repository.findAll();
    }
}
"#;
        let tree = parse_java(source);
        let result = extract(
            &tree,
            "src/main/java/com/example/service/OrderService.java",
            source,
        );

        // Nodes: OrderRepository interface + findById + findAll (interface methods)
        //        + OrderService class + OrderService constructor + getOrder + listOrders + findById + findAll
        // = 2 (interface + class) + 2 (interface methods) + 5 (class methods) = 9
        assert!(result.nodes.len() >= 7); // At minimum: interface + class + constructor + 4 methods

        // Check interface
        let iface = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Interface)
            .unwrap();
        assert_eq!(
            iface.fqn,
            "src/main/java/com/example/service/OrderService.java::OrderRepository"
        );

        // Check class
        let class = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class)
            .unwrap();
        assert_eq!(
            class.fqn,
            "src/main/java/com/example/service/OrderService.java::OrderService"
        );

        // Check methods
        assert!(result.nodes.iter().any(|n| n.fqn
            == "src/main/java/com/example/service/OrderService.java::OrderService::getOrder"));
        assert!(result.nodes.iter().any(|n| n.fqn
            == "src/main/java/com/example/service/OrderService.java::OrderService::listOrders"));

        // Check import edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "java.util.List")
        );
        assert!(
            import_edges
                .iter()
                .any(|e| e.target_fqn == "java.util.Optional")
        );

        // Check call edges: getOrder calls findById
        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(call_edges.iter().any(|e| {
            e.source_fqn
                == "src/main/java/com/example/service/OrderService.java::OrderService::getOrder"
                && e.target_fqn.contains("findById")
        }));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
public class MyService {
    public void doWork() {}
    public String getName() { return ""; }
}

public interface IRepo {
    void find();
}
"#;
        let tree = parse_java(source);
        let result = extract(&tree, "src/Service.java", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/Service.java::MyService"));
        assert!(fqns.contains(&"src/Service.java::MyService::doWork"));
        assert!(fqns.contains(&"src/Service.java::MyService::getName"));
        assert!(fqns.contains(&"src/Service.java::IRepo"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
public class ValidClass {
    public void validMethod() {}
}

public class Broken {{{ invalid syntax !!!

public class AnotherValid {
    public void anotherMethod() {}
}
"#;
        let tree = parse_java(source);
        let result = extract(&tree, "Broken.java", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Broken.java::ValidClass")
        );
    }

    #[test]
    fn test_constructor_extraction() {
        let source = r#"
public class Person {
    private String name;

    public Person(String name) {
        this.name = name;
    }

    public String getName() {
        return this.name;
    }
}
"#;
        let tree = parse_java(source);
        let result = extract(&tree, "Person.java", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Person.java::Person::Person")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "Person.java::Person::getName")
        );
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_java(source);
        let result = extract(&tree, "Empty.java", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    /// Validates: Requirements 9.4
    /// Verify that methods inside classes get NodeKind::Method.
    /// Note: Java does not have standalone functions; all methods are inside classes.
    #[test]
    fn test_nodekind_method_vs_function() {
        let source = r#"
public class Calculator {
    public int add(int a, int b) {
        return a + b;
    }

    public int subtract(int a, int b) {
        return a - b;
    }
}
"#;
        let tree = parse_java(source);
        let result = extract(&tree, "test_nodekind/Calculator.java", source);

        // Methods inside class should be NodeKind::Method
        let add_method = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::Calculator::add"))
            .expect("Calculator.add should be extracted");
        assert_eq!(
            add_method.kind,
            NodeKind::Method,
            "Method inside class should have NodeKind::Method"
        );

        let subtract_method = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::Calculator::subtract"))
            .expect("Calculator.subtract should be extracted");
        assert_eq!(
            subtract_method.kind,
            NodeKind::Method,
            "Method inside class should have NodeKind::Method"
        );

        // Class itself should be NodeKind::Class
        let class_node = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::Calculator") && n.kind == NodeKind::Class)
            .expect("Calculator class should be extracted");
        assert_eq!(class_node.kind, NodeKind::Class);
    }

    /// Validates: Requirements 9.4
    /// Verify that method FQNs include the parent type name in the format file::ClassName::method_name.
    #[test]
    fn test_method_fqn_includes_parent_type() {
        let source = r#"
public class UserService {
    public void getUser(String id) {}
    public void deleteUser(String id) {}
}

public interface UserRepository {
    void findById(String id);
}
"#;
        let tree = parse_java(source);
        let result = extract(&tree, "src/UserService.java", source);

        // Method FQNs should include parent class
        let get_user = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("getUser"))
            .expect("getUser method should be extracted");
        assert_eq!(
            get_user.fqn, "src/UserService.java::UserService::getUser",
            "Method FQN should be file::ClassName::method_name"
        );

        let delete_user = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("deleteUser"))
            .expect("deleteUser method should be extracted");
        assert_eq!(
            delete_user.fqn, "src/UserService.java::UserService::deleteUser",
            "Method FQN should be file::ClassName::method_name"
        );

        // Interface method FQNs should include parent interface
        let find_by_id = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("findById"))
            .expect("findById method should be extracted");
        assert_eq!(
            find_by_id.fqn, "src/UserService.java::UserRepository::findById",
            "Interface method FQN should be file::InterfaceName::method_name"
        );
    }
}
