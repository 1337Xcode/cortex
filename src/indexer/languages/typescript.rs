//! TypeScript/TSX AST extractor.
//!
//! Extracts structural nodes (functions, arrow functions, methods, classes, interfaces)
//! and edges (calls, imports) from a tree-sitter TypeScript parse tree.
//! Handles both .ts and .tsx files with the same extractor.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed TypeScript/TSX file.
///
/// Handles:
/// - Function declarations (`function foo() {}`)
/// - Arrow functions assigned to const/let (`const foo = () => {}`)
/// - Class declarations
/// - Interface declarations
/// - Methods inside classes
/// - Import statements
/// - Intra-file call expressions resolved to definitions in the same file
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
        if node.kind == NodeKind::Function || node.kind == NodeKind::Method {
            // TypeScript functions can be function_declaration or method_definition
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_declaration")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "method_definition"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "lexical_declaration"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "typescript");
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

/// Recursively collect function, class, interface, and method definitions.
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
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    let fqn = match class_name {
                        Some(cls) => format!("{file}::{cls}::{name}"),
                        None => format!("{file}::{name}"),
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

                    defined_fqns.push((name.to_string(), fqn));
                }
            }
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

                    // Recurse into class body to find methods
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
                }
            }
            // Arrow functions assigned to const/let at top level
            "lexical_declaration" | "variable_declaration" if class_name.is_none() => {
                extract_arrow_functions_from_declaration(child, file, source, nodes, defined_fqns);
            }
            // Export statements may contain declarations
            "export_statement" => {
                collect_definitions(child, file, source, class_name, nodes, defined_fqns);
            }
            // Methods inside class body
            "method_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    if let Some(cls) = class_name {
                        let fqn = format!("{file}::{cls}::{method_name}");
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
            }
            _ => {}
        }
    }
}

/// Extract arrow functions from variable declarations like `const foo = () => {}`
fn extract_arrow_functions_from_declaration(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name_node = child.child_by_field_name("name");
            let value_node = child.child_by_field_name("value");

            if let (Some(name), Some(value)) = (name_node, value_node) {
                // Check if the value is an arrow function (possibly with type annotation)
                let is_arrow = value.kind() == "arrow_function"
                    || (value.kind() == "as_expression"
                        && has_child_of_kind(value, "arrow_function"));

                if is_arrow {
                    let func_name = name.utf8_text(source).unwrap_or("");
                    let fqn = format!("{file}::{func_name}");
                    // Use the whole declaration's span
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

                    defined_fqns.push((func_name.to_string(), fqn));
                }
            }
        }
    }
}

/// Check if a node has a child of a specific kind (non-recursive).
fn has_child_of_kind(node: tree_sitter::Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

/// Collect import statements and create Imports edges.
/// Also detects re-export statements and marks them with `reexport: true`.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_statement" {
            if let Some(source_node) = child.child_by_field_name("source") {
                let raw = source_node.utf8_text(source).unwrap_or("");
                let module_path = raw.trim_matches(|c| c == '\'' || c == '"');
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
        } else if child.kind() == "export_statement" {
            // Detect re-exports: `export { foo } from './bar'` or `export * from './bar'`
            if let Some(source_node) = child.child_by_field_name("source") {
                let raw = source_node.utf8_text(source).unwrap_or("");
                let module_path = raw.trim_matches(|c| c == '\'' || c == '"');
                if !module_path.is_empty() {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: module_path.to_string(),
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({"reexport": true}),
                    });
                }
            }
        }
    }
}

/// Collect intra-file call expressions and create Calls edges.
///
/// Handles both simple calls (`foo()`) and member expression calls (`obj.method()`).
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
                            attributes: json!({}),
                        });
                    }
                }
                "member_expression" => {
                    // obj.method() - member_expression has object and property fields
                    let full_text = func_node.utf8_text(source).unwrap_or("");
                    if let Some(dot_pos) = full_text.rfind('.') {
                        let receiver = &full_text[..dot_pos];
                        let method_name = &full_text[dot_pos + 1..];
                        if !method_name.is_empty() {
                            let chain_position = count_chain_depth_ts(func_node);
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
                    let class_name = find_enclosing_class(parent, source);
                    return match class_name {
                        Some(cls) => Some(format!("{file}::{cls}::{func_name}")),
                        None => Some(format!("{file}::{func_name}")),
                    };
                }
            }
            "method_definition" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let method_name = name_node.utf8_text(source).unwrap_or("");
                    let class_name = find_enclosing_class(parent, source);
                    return match class_name {
                        Some(cls) => Some(format!("{file}::{cls}::{method_name}")),
                        None => Some(format!("{file}::{method_name}")),
                    };
                }
            }
            "arrow_function" => {
                // Check if this arrow function is assigned to a variable
                if let Some(declarator) = parent.parent()
                    && declarator.kind() == "variable_declarator"
                    && let Some(name_node) = declarator.child_by_field_name("name")
                {
                    let func_name = name_node.utf8_text(source).unwrap_or("");
                    return Some(format!("{file}::{func_name}"));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    // Call at module level
    Some(file.to_string())
}

/// Find the enclosing class for a method node.
fn find_enclosing_class(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
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

/// Count chain depth for TypeScript member expressions.
fn count_chain_depth_ts(func_node: tree_sitter::Node) -> u32 {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            return 1 + count_chain_depth_in_call_ts(child);
        }
    }
    0
}

fn count_chain_depth_in_call_ts(call_node: tree_sitter::Node) -> u32 {
    if let Some(func) = call_node.child_by_field_name("function")
        && func.kind() == "member_expression"
    {
        return count_chain_depth_ts(func);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_typescript(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_class_interface_arrow_functions_imports() {
        let source = r#"
import { Request, Response } from 'express';
import { validate } from './utils';

interface IOrderService {
    process(order: Order): Promise<void>;
}

class OrderController {
    constructor(private service: IOrderService) {}

    async handleOrder(req: Request, res: Response): Promise<void> {
        const order = req.body;
        validate(order);
        await this.service.process(order);
        res.json({ ok: true });
    }
}

const createApp = () => {
    return new OrderController(null);
};

function bootstrap(): void {
    const app = createApp();
}
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "src/controllers/order.ts", source);

        // Check nodes: interface + class + constructor + handleOrder + createApp + bootstrap = 6
        assert_eq!(result.nodes.len(), 6);

        // Check interface
        let iface = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Interface)
            .unwrap();
        assert_eq!(iface.fqn, "src/controllers/order.ts::IOrderService");

        // Check class
        let class = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class)
            .unwrap();
        assert_eq!(class.fqn, "src/controllers/order.ts::OrderController");

        // Check methods
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/controllers/order.ts::OrderController::constructor")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/controllers/order.ts::OrderController::handleOrder")
        );

        // Check arrow function
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/controllers/order.ts::createApp")
        );

        // Check function declaration
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/controllers/order.ts::bootstrap")
        );

        // Check import edges
        let import_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 2);
        assert!(import_edges.iter().any(|e| e.target_fqn == "express"));
        assert!(import_edges.iter().any(|e| e.target_fqn == "./utils"));

        // Check call edges: createApp called from bootstrap
        let call_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(call_edges.iter().any(|e| {
            e.source_fqn == "src/controllers/order.ts::bootstrap"
                && e.target_fqn == "src/controllers/order.ts::createApp"
        }));
    }

    #[test]
    fn test_fqn_format() {
        let source = r#"
class MyService {
    doWork(): void {}
}

interface IRepo {
    find(): void;
}

const helper = () => {};

function standalone(): void {}
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "src/service.ts", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"src/service.ts::MyService"));
        assert!(fqns.contains(&"src/service.ts::MyService::doWork"));
        assert!(fqns.contains(&"src/service.ts::IRepo"));
        assert!(fqns.contains(&"src/service.ts::helper"));
        assert!(fqns.contains(&"src/service.ts::standalone"));
    }

    #[test]
    fn test_invalid_syntax_returns_partial_results() {
        let source = r#"
function validFunc(): void {
    console.log("hello");
}

class Broken {{{ invalid syntax !!!

function anotherValid(): void {}
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "broken.ts", source);

        // Should not panic and should extract at least the valid definitions
        assert!(!result.nodes.is_empty());
        assert!(result.nodes.iter().any(|n| n.fqn == "broken.ts::validFunc"));
    }

    #[test]
    fn test_node_line_numbers() {
        let source = "function foo(): void {\n}\n\nfunction bar(): void {\n}\n";
        let tree = parse_typescript(source);
        let result = extract(&tree, "test.ts", source);

        let foo = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.ts::foo")
            .unwrap();
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 2);

        let bar = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.ts::bar")
            .unwrap();
        assert_eq!(bar.start_line, 4);
        assert_eq!(bar.end_line, 5);
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_typescript(source);
        let result = extract(&tree, "empty.ts", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_export_statements() {
        let source = r#"
export function exported(): void {}
export class ExportedClass {
    method(): void {}
}
export interface ExportedInterface {}
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "lib.ts", source);

        let fqns: Vec<&str> = result.nodes.iter().map(|n| n.fqn.as_str()).collect();
        assert!(fqns.contains(&"lib.ts::exported"));
        assert!(fqns.contains(&"lib.ts::ExportedClass"));
        assert!(fqns.contains(&"lib.ts::ExportedClass::method"));
        assert!(fqns.contains(&"lib.ts::ExportedInterface"));
    }

    /// Validates: Requirements 9.4
    /// Verify that standalone functions get NodeKind::Function and methods get NodeKind::Method.
    #[test]
    fn test_nodekind_method_vs_function() {
        let source = r#"
class Calculator {
    add(a: number, b: number): number {
        return a + b;
    }

    subtract(a: number, b: number): number {
        return a - b;
    }
}

function standaloneHelper(): number {
    return 42;
}

const arrowHelper = () => {
    return 99;
};
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "test_nodekind.ts", source);

        // Standalone function declaration should be NodeKind::Function
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

        // Arrow function should be NodeKind::Function
        let arrow = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::arrowHelper"))
            .expect("arrowHelper should be extracted");
        assert_eq!(
            arrow.kind,
            NodeKind::Function,
            "Arrow function should have NodeKind::Function"
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
class UserService {
    getUser(id: string): void {}
    deleteUser(id: string): void {}
}

function freeFunction(): void {}
"#;
        let tree = parse_typescript(source);
        let result = extract(&tree, "src/user.ts", source);

        // Method FQNs should include parent type
        let get_user = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("getUser"))
            .expect("getUser method should be extracted");
        assert_eq!(
            get_user.fqn, "src/user.ts::UserService::getUser",
            "Method FQN should be file::ClassName::method_name"
        );

        let delete_user = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.fqn.contains("deleteUser"))
            .expect("deleteUser method should be extracted");
        assert_eq!(
            delete_user.fqn, "src/user.ts::UserService::deleteUser",
            "Method FQN should be file::ClassName::method_name"
        );

        // Standalone function FQN should NOT include a class name
        let free_fn = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.fqn.contains("freeFunction"))
            .expect("freeFunction should be extracted");
        assert_eq!(
            free_fn.fqn, "src/user.ts::freeFunction",
            "Function FQN should be file::function_name without class"
        );
    }
}
