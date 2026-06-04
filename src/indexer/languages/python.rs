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
fn compute_node_complexities(nodes: &mut [Node], root: tree_sitter::Node, source: &[u8]) {
    for node in nodes.iter_mut() {
        if (node.kind == NodeKind::Function || node.kind == NodeKind::Method)
            && let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_definition")
        {
            let c = complexity::compute_full_complexity(ast_node, source, "python");
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
                        Some(_) => NodeKind::Method,
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
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
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
                                edge_source: crate::store::confidence::EdgeSource::AstDirect,
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
                            edge_source: crate::store::confidence::EdgeSource::AstDirect,
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
        if child.kind() == "call"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let caller_fqn =
                find_enclosing_function(child, file, source).unwrap_or_else(|| file.to_string());

            match func_node.kind() {
                "identifier" => {
                    // Simple call: foo()
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
    let mut class_name: Option<String> = None;

    while let Some(parent) = current {
        if parent.kind() == "function_definition"
            && let Some(name_node) = parent.child_by_field_name("name")
        {
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
    if let Some(func) = call_node.child_by_field_name("function")
        && func.kind() == "attribute"
    {
        return count_chain_depth(func);
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
        assert_eq!(init_node.kind, NodeKind::Method);

        let process_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/orders/processor.py::OrderProcessor::process")
            .unwrap();
        assert_eq!(process_node.kind, NodeKind::Method);

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
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "broken.py::valid_function")
        );
    }

    #[test]
    fn test_node_line_numbers() {
        let source = "def foo():\n    pass\n\ndef bar():\n    pass\n";
        let tree = parse_python(source);
        let result = extract(&tree, "test.py", source);

        let foo = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.py::foo")
            .unwrap();
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 2);

        let bar = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.py::bar")
            .unwrap();
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

    /// Validates: Requirements 9.4
    /// Verify that standalone functions get NodeKind::Function and methods get NodeKind::Method.
    #[test]
    fn test_nodekind_method_vs_function() {
        let source = r#"
class Calculator:
    def add(self, a, b):
        return a + b

    def subtract(self, a, b):
        return a - b

def standalone_helper():
    return 42
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "test_nodekind.py", source);

        // Standalone function should be NodeKind::Function
        let helper = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::standalone_helper"))
            .expect("standalone_helper should be extracted");
        assert_eq!(
            helper.kind,
            NodeKind::Function,
            "Standalone function should have NodeKind::Function"
        );

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
    }

    /// Validates: Requirements 9.4
    /// Verify that method FQNs include the parent type name in the format file::ClassName::method_name.
    #[test]
    fn test_method_fqn_includes_parent_type() {
        let source = r#"
class MyService:
    def process(self):
        pass

    def validate(self):
        pass

def free_function():
    pass
"#;
        let tree = parse_python(source);
        let result = extract(&tree, "src/service.py", source);

        // Method FQNs should include parent type
        let process = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("process"))
            .expect("process method should be extracted");
        assert_eq!(
            process.fqn, "src/service.py::MyService::process",
            "Method FQN should be file::ClassName::method_name"
        );

        let validate = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("validate"))
            .expect("validate method should be extracted");
        assert_eq!(
            validate.fqn, "src/service.py::MyService::validate",
            "Method FQN should be file::ClassName::method_name"
        );

        // Standalone function FQN should NOT include a class name
        let free_fn = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.fqn.contains("free_function"))
            .expect("free_function should be extracted");
        assert_eq!(
            free_fn.fqn, "src/service.py::free_function",
            "Function FQN should be file::function_name without class"
        );
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy to generate a valid Python identifier (lowercase letters + underscores).
    fn arb_python_ident() -> impl Strategy<Value = String> {
        "[a-z][a-z_]{0,9}".prop_filter("not a keyword", |s| {
            !matches!(
                s.as_str(),
                "if" | "else"
                    | "for"
                    | "while"
                    | "def"
                    | "class"
                    | "return"
                    | "import"
                    | "from"
                    | "pass"
                    | "try"
                    | "except"
                    | "with"
                    | "as"
                    | "in"
                    | "is"
                    | "not"
                    | "and"
                    | "or"
                    | "None"
                    | "True"
                    | "False"
                    | "lambda"
                    | "yield"
                    | "raise"
                    | "del"
                    | "global"
                    | "nonlocal"
                    | "assert"
                    | "break"
                    | "continue"
                    | "elif"
                    | "finally"
            )
        })
    }

    /// Strategy to generate a class with methods.
    fn arb_class() -> impl Strategy<Value = (String, Vec<String>)> {
        (
            arb_python_ident(),
            proptest::collection::vec(arb_python_ident(), 1..5),
        )
            .prop_map(|(class_name, methods)| {
                // Deduplicate method names
                let mut unique_methods: Vec<String> = Vec::new();
                for m in methods {
                    if !unique_methods.contains(&m) {
                        unique_methods.push(m);
                    }
                }
                (class_name, unique_methods)
            })
    }

    /// Strategy to generate a Python source file with classes and standalone functions.
    /// Returns (source_code, expected_methods, expected_functions) where:
    /// - expected_methods: Vec of (class_name, method_name) pairs
    /// - expected_functions: Vec of standalone function names
    fn arb_python_source() -> impl Strategy<Value = (String, Vec<(String, String)>, Vec<String>)> {
        (
            proptest::collection::vec(arb_class(), 0..4),
            proptest::collection::vec(arb_python_ident(), 0..5),
        )
            .prop_map(|(classes, standalone_fns)| {
                let mut source = String::new();
                let mut expected_methods: Vec<(String, String)> = Vec::new();
                let mut expected_functions: Vec<String> = Vec::new();

                // Track all used names to avoid collisions
                let mut used_names: Vec<String> = Vec::new();

                // Generate class definitions
                for (class_name, methods) in &classes {
                    if used_names.contains(class_name) {
                        continue;
                    }
                    used_names.push(class_name.clone());

                    source.push_str(&format!("class {}:\n", class_name));
                    let mut class_method_names: Vec<String> = Vec::new();
                    for method_name in methods {
                        if class_method_names.contains(method_name) {
                            continue;
                        }
                        class_method_names.push(method_name.clone());
                        source.push_str(&format!("    def {}(self):\n", method_name));
                        source.push_str("        pass\n\n");
                        expected_methods.push((class_name.clone(), method_name.clone()));
                    }
                    source.push('\n');
                }

                // Generate standalone functions
                for func_name in &standalone_fns {
                    if used_names.contains(func_name) {
                        continue;
                    }
                    used_names.push(func_name.clone());
                    source.push_str(&format!("def {}():\n", func_name));
                    source.push_str("    pass\n\n");
                    expected_functions.push(func_name.clone());
                }

                (source, expected_methods, expected_functions)
            })
    }

    // **Validates: Requirements 9.1, 9.2**

    proptest! {
        /// **Property 8: NodeKind classification correctness**
        ///
        /// For any extracted function, NodeKind is `Method` iff defined inside
        /// class/struct/impl/trait; otherwise `Function`.
        #[test]
        fn prop_nodekind_classification_correctness(
            (source, expected_methods, expected_functions) in arb_python_source()
        ) {
            // Skip empty sources (no definitions to verify)
            if expected_methods.is_empty() && expected_functions.is_empty() {
                return Ok(());
            }

            let tree = parse_python(&source);
            let result = extract(&tree, "test_prop.py", &source);

            // Verify: every function inside a class gets NodeKind::Method
            for (class_name, method_name) in &expected_methods {
                let expected_fqn = format!("test_prop.py::{}::{}", class_name, method_name);
                let node = result.nodes.iter().find(|n| n.fqn == expected_fqn);
                prop_assert!(
                    node.is_some(),
                    "Expected method node with FQN '{}' not found. Source:\n{}",
                    expected_fqn,
                    source
                );
                let node = node.unwrap();
                prop_assert!(
                    node.kind == NodeKind::Method,
                    "Function '{}' inside class '{}' should be NodeKind::Method, got {:?}. Source:\n{}",
                    method_name,
                    class_name,
                    &node.kind,
                    source
                );
            }

            // Verify: every standalone function gets NodeKind::Function
            for func_name in &expected_functions {
                let expected_fqn = format!("test_prop.py::{}", func_name);
                let node = result.nodes.iter().find(|n| n.fqn == expected_fqn);
                prop_assert!(
                    node.is_some(),
                    "Expected function node with FQN '{}' not found. Source:\n{}",
                    expected_fqn,
                    source
                );
                let node = node.unwrap();
                prop_assert!(
                    node.kind == NodeKind::Function,
                    "Standalone function '{}' should be NodeKind::Function, got {:?}. Source:\n{}",
                    func_name,
                    &node.kind,
                    source
                );
            }

            // Verify: no node with NodeKind::Method exists that isn't in our expected list
            for node in &result.nodes {
                if node.kind == NodeKind::Method {
                    // Must be in expected_methods
                    let is_expected = expected_methods.iter().any(|(cls, meth)| {
                        node.fqn == format!("test_prop.py::{}::{}", cls, meth)
                    });
                    prop_assert!(
                        is_expected,
                        "Unexpected Method node '{}' found. Source:\n{}",
                        node.fqn,
                        source
                    );
                }
                if node.kind == NodeKind::Function {
                    // Must be in expected_functions
                    let is_expected = expected_functions.iter().any(|f| {
                        node.fqn == format!("test_prop.py::{}", f)
                    });
                    prop_assert!(
                        is_expected,
                        "Unexpected Function node '{}' found. Source:\n{}",
                        node.fqn,
                        source
                    );
                }
            }
        }
    }

    // **Validates: Requirements 9.3**

    proptest! {
        /// **Property 9: Method FQN contains parent type**
        ///
        /// For any node with NodeKind `Method`, the FQN matches the pattern
        /// `file::ClassName::method_name`, containing at least two `::` separators,
        /// and the class name segment matches the actual class name from the source.
        ///
        /// **Validates: Requirements 9.3**
        #[test]
        fn prop_method_fqn_contains_parent_type(
            (source, expected_methods, _expected_functions) in arb_python_source()
        ) {
            // Skip sources with no methods
            if expected_methods.is_empty() {
                return Ok(());
            }

            let tree = parse_python(&source);
            let file = "test_fqn.py";
            let result = extract(&tree, file, &source);

            // Collect all class names from the generated source
            let class_names: Vec<&str> = expected_methods
                .iter()
                .map(|(cls, _)| cls.as_str())
                .collect();

            // For every Method node, verify FQN format
            let method_nodes: Vec<&Node> = result
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Method)
                .collect();

            prop_assert!(
                !method_nodes.is_empty(),
                "Expected at least one Method node from source with {} expected methods:\n{}",
                expected_methods.len(),
                source
            );

            for node in &method_nodes {
                // FQN must have at least two `::` separators (file::Class::method)
                let segments: Vec<&str> = node.fqn.split("::").collect();
                prop_assert!(
                    segments.len() >= 3,
                    "Method FQN '{}' should have at least 3 segments (file::Class::method), got {}",
                    node.fqn,
                    segments.len()
                );

                // The first segment should be the file path
                prop_assert_eq!(
                    segments[0], file,
                    "First FQN segment '{}' should be the file path '{}'",
                    segments[0], file
                );

                // The second segment (class name) must be one of the generated class names
                let class_segment = segments[1];
                prop_assert!(
                    class_names.contains(&class_segment),
                    "Class segment '{}' in FQN '{}' should be one of the generated class names: {:?}",
                    class_segment,
                    node.fqn,
                    class_names
                );

                // The third segment (method name) should be non-empty
                let method_segment = segments[2];
                prop_assert!(
                    !method_segment.is_empty(),
                    "Method name segment in FQN '{}' should not be empty",
                    node.fqn
                );

                // Verify the (class, method) pair exists in expected_methods
                let pair_exists = expected_methods
                    .iter()
                    .any(|(cls, meth)| cls == class_segment && meth == method_segment);
                prop_assert!(
                    pair_exists,
                    "FQN '{}' has class='{}' method='{}' but this pair is not in expected_methods: {:?}",
                    node.fqn,
                    class_segment,
                    method_segment,
                    expected_methods
                );
            }
        }
    }
}
