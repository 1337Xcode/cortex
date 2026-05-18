//! Objective-C AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (classes, protocols, categories, methods)
//! and edges (imports, intra-file calls, message sends) from a tree-sitter
//! Objective-C parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Objective-C file.
///
/// Handles:
/// - Classes (`@interface ClassName : SuperClass ... @end`)
/// - Class implementations (`@implementation ClassName ... @end`)
/// - Protocols (`@protocol ProtocolName ... @end`)
/// - Categories (`@interface ClassName (CategoryName) ... @end`)
/// - Instance methods (`- (ReturnType)methodName:...`)
/// - Class methods (`+ (ReturnType)methodName:...`)
/// - Imports (`#import <Framework/Header.h>`, `#import "header.h"`, `@import Module;`)
/// - Message sends (`[object message:arg]`)
/// - C-style function calls (`function_name(args)`)
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)

    collect_definitions(root, file, source_bytes, &mut nodes, &mut defined_fqns);

    // Compute cyclomatic complexity for Function nodes
    compute_node_complexities(&mut nodes, root, source_bytes);

    // Second pass: collect imports and calls
    collect_imports(root, file, source_bytes, &mut edges);
    collect_calls(root, file, source_bytes, &defined_fqns, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Deprecated wrapper for backward compatibility with the regex-based pipeline.
/// New code should use `extract()` with a pre-parsed tree.
#[deprecated(note = "Use extract() with a tree-sitter Tree instead")]
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_objc::LANGUAGE.into())
        .expect("Objective-C grammar should load");
    match parser.parse(source, None) {
        Some(tree) => extract(&tree, file, source),
        None => ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        },
    }
}

// ─── Complexity ─────────────────────────────────────────────────────────────

/// Compute cyclomatic complexity for all Function nodes.
fn compute_node_complexities(nodes: &mut [Node], root: tree_sitter::Node, source: &[u8]) {
    for node in nodes.iter_mut() {
        if node.kind == NodeKind::Function {
            if let Some(ast_node) = find_ast_node_at_line(root, node.start_line, "method_definition")
                .or_else(|| find_ast_node_at_line(root, node.start_line, "instance_method_declaration"))
                .or_else(|| find_ast_node_at_line(root, node.start_line, "class_method_declaration"))
                .or_else(|| find_ast_node_at_line(root, node.start_line, "function_definition"))
                .or_else(|| find_ast_node_at_line_fuzzy(root, node.start_line))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "objc");
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

/// Fuzzy fallback: find any node at the target line that looks like a method/function.
fn find_ast_node_at_line_fuzzy<'a>(
    node: tree_sitter::Node<'a>,
    target_line: u32,
) -> Option<tree_sitter::Node<'a>> {
    let node_start_line = node.start_position().row as u32 + 1;
    let kind = node.kind();
    if node_start_line == target_line
        && (kind.contains("method") || kind.contains("function") || kind.contains("Method"))
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_ast_node_at_line_fuzzy(child, target_line) {
            return Some(found);
        }
    }
    None
}

// ─── Definition Collection ──────────────────────────────────────────────────

/// Recursively collect definitions from the AST.
///
/// In tree-sitter-objc, top-level constructs include:
/// - `@interface ClassName : SuperClass ... @end` (class_interface)
/// - `@implementation ClassName ... @end` (class_implementation)
/// - `@protocol ProtocolName ... @end` (protocol_declaration)
/// - `@interface ClassName (CategoryName) ... @end` (category_interface)
/// - Method declarations/definitions within classes
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        match kind {
            // Class interface: @interface ClassName : SuperClass <Protocols> ... @end
            "class_interface" | "class_declaration" => {
                extract_class_interface(child, file, source, nodes, defined_fqns);
            }
            // Class implementation: @implementation ClassName ... @end
            "class_implementation" | "implementation_definition" => {
                extract_class_implementation(child, file, source, nodes, defined_fqns);
            }
            // Protocol declaration: @protocol ProtocolName ... @end
            "protocol_declaration" | "protocol_definition" => {
                extract_protocol(child, file, source, nodes, defined_fqns);
            }
            // Category interface: @interface ClassName (CategoryName) ... @end
            "category_interface" | "category_declaration" => {
                extract_category(child, file, source, nodes, defined_fqns);
            }
            // Category implementation
            "category_implementation" => {
                extract_category_impl(child, file, source, nodes, defined_fqns);
            }
            // Standalone function definitions (C-style)
            "function_definition" | "declaration" => {
                try_extract_c_function(child, file, source, nodes, defined_fqns);
            }
            _ => {
                // Recurse into other container nodes
                if child.child_count() > 0 {
                    collect_definitions(child, file, source, nodes, defined_fqns);
                }
            }
        }
    }
}

/// Extract a class from @interface declaration.
fn extract_class_interface(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("");
    let name = extract_objc_name_after_keyword(text, "@interface");
    if name.is_empty() {
        return;
    }

    // Check if this is a category (has parentheses after class name)
    if is_category_text(text, &name) {
        extract_category_from_text(node, file, source, &name, text, nodes, defined_fqns);
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn && n.kind == NodeKind::Class) {
        let mut attrs = serde_json::Map::new();
        attrs.insert("declaration_type".to_string(), json!("interface"));

        // Extract superclass if present
        if let Some(super_name) = extract_superclass(text) {
            attrs.insert("superclass".to_string(), json!(super_name));
        }

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!(attrs),
        });
        defined_fqns.push((name.clone(), fqn.clone()));
    }

    // Extract methods declared in the interface
    extract_methods_from_container(node, file, source, &name, nodes, defined_fqns);
}

/// Extract a class from @implementation declaration.
fn extract_class_implementation(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("");
    let name = extract_objc_name_after_keyword(text, "@implementation");
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn && n.kind == NodeKind::Class) {
        let mut attrs = serde_json::Map::new();
        attrs.insert("declaration_type".to_string(), json!("implementation"));

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!(attrs),
        });
        defined_fqns.push((name.clone(), fqn.clone()));
    }

    // Extract methods defined in the implementation
    extract_methods_from_container(node, file, source, &name, nodes, defined_fqns);
}

/// Extract a protocol declaration.
fn extract_protocol(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("");
    let name = extract_objc_name_after_keyword(text, "@protocol");
    if name.is_empty() {
        return;
    }

    // Skip forward declarations like `@protocol Foo;`
    let trimmed = text.trim();
    if trimmed.ends_with(';') && !trimmed.contains('\n') {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        let mut attrs = serde_json::Map::new();
        attrs.insert("protocol".to_string(), json!(true));

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Interface,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!(attrs),
        });
        defined_fqns.push((name.clone(), fqn.clone()));
    }

    // Extract methods declared in the protocol
    extract_methods_from_container(node, file, source, &name, nodes, defined_fqns);
}

/// Extract a category declaration.
fn extract_category(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("");
    let name = extract_objc_name_after_keyword(text, "@interface");
    if name.is_empty() {
        return;
    }
    extract_category_from_text(node, file, source, &name, text, nodes, defined_fqns);
}

/// Extract a category implementation.
fn extract_category_impl(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("");
    let name = extract_objc_name_after_keyword(text, "@implementation");
    if name.is_empty() {
        return;
    }

    // Extract category name from parentheses
    let category_name = extract_category_name(text, &name).unwrap_or_default();
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let display_name = if category_name.is_empty() {
        name.clone()
    } else {
        format!("{}+{}", name, category_name)
    };
    let fqn = format!("{file}::{display_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        let mut attrs = serde_json::Map::new();
        attrs.insert("category".to_string(), json!(category_name));
        attrs.insert("class_name".to_string(), json!(name));
        attrs.insert("declaration_type".to_string(), json!("category_implementation"));

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!(attrs),
        });
        defined_fqns.push((display_name.clone(), fqn.clone()));
    }

    // Extract methods in the category implementation
    extract_methods_from_container(node, file, source, &display_name, nodes, defined_fqns);
}

/// Helper to extract a category from text when we know it's a category.
fn extract_category_from_text(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    class_name: &str,
    text: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let category_name = extract_category_name(text, class_name).unwrap_or_default();
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let display_name = if category_name.is_empty() {
        class_name.to_string()
    } else {
        format!("{}+{}", class_name, category_name)
    };
    let fqn = format!("{file}::{display_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        let mut attrs = serde_json::Map::new();
        attrs.insert("category".to_string(), json!(category_name));
        attrs.insert("class_name".to_string(), json!(class_name));

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!(attrs),
        });
        defined_fqns.push((display_name.clone(), fqn.clone()));
    }

    // Extract methods in the category
    extract_methods_from_container(node, file, source, &display_name, nodes, defined_fqns);
}

/// Try to extract a C-style function definition.
fn try_extract_c_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let kind = node.kind();
    if kind != "function_definition" {
        return;
    }

    let text = node.utf8_text(source).unwrap_or("");
    // Extract function name: look for identifier before '('
    let name = extract_c_function_name(text);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"function_type": "c_function"}),
        });
        defined_fqns.push((name, fqn));
    }
}

/// Extract methods (instance and class) from a container node (class, protocol, category).
fn extract_methods_from_container(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    container_name: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "method_definition" | "instance_method_declaration"
            | "class_method_declaration" | "method_declaration"
            | "instance_method_definition" | "class_method_definition" => {
                extract_method(child, file, source, container_name, nodes, defined_fqns);
            }
            _ => {
                // Recurse into child containers (e.g., method lists)
                if child.child_count() > 0 && !is_leaf_kind(kind) {
                    extract_methods_from_container(
                        child, file, source, container_name, nodes, defined_fqns,
                    );
                }
            }
        }
    }
}

/// Extract a single method (instance or class).
fn extract_method(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    container_name: &str,
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim();
    if text.is_empty() {
        return;
    }

    // Determine if instance (-) or class (+) method
    let is_class_method = text.starts_with('+');
    let method_name = extract_method_selector(text);
    if method_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{container_name}::{method_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    if is_class_method {
        attrs.insert("method_type".to_string(), json!("class"));
    } else {
        attrs.insert("method_type".to_string(), json!("instance"));
    }
    attrs.insert("container".to_string(), json!(container_name));

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Function,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
    defined_fqns.push((method_name, fqn));
}

// ─── Import Collection ──────────────────────────────────────────────────────

/// Collect import edges from the AST.
///
/// Handles:
/// - `#import <Framework/Header.h>`
/// - `#import "header.h"`
/// - `#include <header.h>`
/// - `#include "header.h"`
/// - `@import Module;`
fn collect_imports(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    collect_imports_recursive(node, file, source, edges);
}

fn collect_imports_recursive(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    edges: &mut Vec<Edge>,
) {
    let kind = node.kind();

    // Handle preprocessor import/include directives
    if kind == "preproc_import" || kind == "preproc_include"
        || kind == "#import" || kind == "import_declaration"
        || kind == "preproc_def" || kind == "preprocessing_directive"
    {
        let text = node.utf8_text(source).unwrap_or("").trim();
        extract_import_from_text(text, file, edges);
    }

    // Handle @import Module; (module import)
    if kind == "module_import" || kind == "import_declaration" {
        let text = node.utf8_text(source).unwrap_or("").trim();
        if text.starts_with("@import") {
            extract_module_import(text, file, edges);
        }
    }

    // Also check raw text for lines that look like imports (fallback)
    if node.child_count() == 0 {
        let text = node.utf8_text(source).unwrap_or("").trim();
        if (text.starts_with("#import") || text.starts_with("#include") || text.starts_with("@import"))
            && !text.is_empty()
        {
            extract_import_from_text(text, file, edges);
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports_recursive(child, file, source, edges);
    }
}

/// Extract import target from a preprocessor directive text.
fn extract_import_from_text(text: &str, file: &str, edges: &mut Vec<Edge>) {
    // Handle #import <Framework/Header.h> or #import "header.h"
    // Handle #include <header.h> or #include "header.h"
    if text.starts_with("#import") || text.starts_with("#include") {
        if let Some(target) = extract_import_path(text) {
            add_import_edge(file, &target, edges);
        }
    } else if text.starts_with("@import") {
        extract_module_import(text, file, edges);
    }
}

/// Extract the path from #import or #include directive.
fn extract_import_path(text: &str) -> Option<String> {
    // Find the first < or " after #import/#include
    let after_keyword = if text.starts_with("#import") {
        &text[7..]
    } else if text.starts_with("#include") {
        &text[8..]
    } else {
        return None;
    };

    let trimmed = after_keyword.trim_start();

    if trimmed.starts_with('<') {
        // System import: <Framework/Header.h>
        let end = trimmed.find('>')?;
        let path = &trimmed[1..end];
        if !path.is_empty() {
            return Some(path.to_string());
        }
    } else if trimmed.starts_with('"') {
        // Local import: "header.h"
        let rest = &trimmed[1..];
        let end = rest.find('"')?;
        let path = &rest[..end];
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

/// Extract module name from @import directive.
fn extract_module_import(text: &str, file: &str, edges: &mut Vec<Edge>) {
    // @import Module; or @import Module.SubModule;
    let after_import = match text.strip_prefix("@import") {
        Some(s) => s.trim_start(),
        None => return,
    };
    let module_name = after_import.trim_end_matches(';').trim();
    if !module_name.is_empty() {
        add_import_edge(file, module_name, edges);
    }
}

/// Add an import edge, avoiding duplicates.
fn add_import_edge(file: &str, target: &str, edges: &mut Vec<Edge>) {
    if !edges.iter().any(|e| {
        e.kind == EdgeKind::Imports && e.source_fqn == file && e.target_fqn == target
    }) {
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({}),
        });
    }
}

// ─── Call Collection ────────────────────────────────────────────────────────

/// Collect intra-file function/method calls.
///
/// In Objective-C, calls come in two forms:
/// - Message sends: `[object message:arg1 param:arg2]`
/// - C-style function calls: `function_name(args)`
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    collect_calls_recursive(node, file, source, defined_fqns, edges);
}

fn collect_calls_recursive(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let kind = node.kind();

    match kind {
        // Objective-C message send: [receiver message]
        "message_expression" | "message_send" | "objc_message_expression" => {
            extract_message_send(node, file, source, defined_fqns, edges);
        }
        // C-style function call
        "call_expression" | "function_call" => {
            extract_c_call(node, file, source, defined_fqns, edges);
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

/// Extract a call edge from an Objective-C message send expression.
///
/// Message sends look like: `[receiver selector:arg1 param2:arg2]`
/// The selector is the method name (e.g., "selector:param2:" or just "selector").
fn extract_message_send(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim();
    if text.is_empty() || !text.starts_with('[') {
        // Fallback: try to parse from text anyway
        let text_full = node.utf8_text(source).unwrap_or("");
        if text_full.is_empty() {
            return;
        }
    }

    // Extract receiver and selector from the message expression
    let (receiver, selector) = extract_message_parts(text);
    if selector.is_empty() {
        return;
    }

    // Simplify selector for matching (use first keyword before colon)
    let simple_selector = selector.split(':').next().unwrap_or(&selector).to_string();
    if simple_selector.is_empty() || is_objc_keyword(&simple_selector) {
        return;
    }

    // Determine the source FQN (enclosing method or file-level)
    let source_fqn =
        find_enclosing_method_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Try to resolve to a defined method
    if let Some((_, target_fqn)) = defined_fqns
        .iter()
        .find(|(simple, _)| simple == &simple_selector || simple == &selector)
    {
        if source_fqn != *target_fqn {
            edges.push(Edge {
                id: None,
                source_fqn: source_fqn.clone(),
                target_fqn: target_fqn.clone(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: json!({"receiver": receiver, "call_type": "message_send", "selector": selector}),
            });
        }
    } else {
        // Emit unresolved call for cross-file resolution
        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: simple_selector,
            kind: EdgeKind::Calls,
            confidence: 0.0,
            attributes: json!({"receiver": receiver, "call_type": "message_send", "selector": selector}),
        });
    }
}

/// Extract a call edge from a C-style function call.
fn extract_c_call(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim();
    if text.is_empty() {
        return;
    }

    // Extract function name (identifier before parenthesis)
    let call_name = extract_c_call_name(text);
    if call_name.is_empty() || is_objc_keyword(&call_name) || is_common_macro(&call_name) {
        return;
    }

    let source_fqn =
        find_enclosing_method_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == &call_name) {
        if source_fqn != *target_fqn {
            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn: target_fqn.clone(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: json!({"call_type": "function"}),
            });
        }
    } else {
        // Emit unresolved call
        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: call_name,
            kind: EdgeKind::Calls,
            confidence: 0.0,
            attributes: json!({"call_type": "function"}),
        });
    }
}

/// Find the enclosing method/function for a given node and return its FQN.
fn find_enclosing_method_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        let kind = parent.kind();
        match kind {
            "method_definition" | "instance_method_definition"
            | "class_method_definition" | "instance_method_declaration"
            | "class_method_declaration" | "method_declaration" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let method_name = extract_method_selector(text);
                if !method_name.is_empty() {
                    // Try to find the container name
                    if let Some(container) = find_container_name(parent, source) {
                        return Some(format!("{file}::{container}::{method_name}"));
                    }
                    return Some(format!("{file}::{method_name}"));
                }
            }
            "function_definition" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let fn_name = extract_c_function_name(text);
                if !fn_name.is_empty() {
                    return Some(format!("{file}::{fn_name}"));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

/// Find the container (class/protocol/category) name for a method node.
fn find_container_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        let kind = parent.kind();
        match kind {
            "class_implementation" | "implementation_definition" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let name = extract_objc_name_after_keyword(text, "@implementation");
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "class_interface" | "class_declaration" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let name = extract_objc_name_after_keyword(text, "@interface");
                if !name.is_empty() {
                    if is_category_text(text, &name) {
                        let cat = extract_category_name(text, &name).unwrap_or_default();
                        if cat.is_empty() {
                            return Some(name);
                        }
                        return Some(format!("{}+{}", name, cat));
                    }
                    return Some(name);
                }
            }
            "protocol_declaration" | "protocol_definition" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let name = extract_objc_name_after_keyword(text, "@protocol");
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "category_interface" | "category_declaration" | "category_implementation" => {
                let text = parent.utf8_text(source).unwrap_or("");
                let keyword = if text.contains("@implementation") {
                    "@implementation"
                } else {
                    "@interface"
                };
                let name = extract_objc_name_after_keyword(text, keyword);
                if !name.is_empty() {
                    let cat = extract_category_name(text, &name).unwrap_or_default();
                    if cat.is_empty() {
                        return Some(name);
                    }
                    return Some(format!("{}+{}", name, cat));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Extract the class/protocol name after a keyword like @interface, @implementation, @protocol.
fn extract_objc_name_after_keyword(text: &str, keyword: &str) -> String {
    if let Some(pos) = text.find(keyword) {
        let after = &text[pos + keyword.len()..];
        let trimmed = after.trim_start();
        let name: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return name;
    }
    String::new()
}

/// Check if text represents a category (has parentheses after class name).
fn is_category_text(text: &str, class_name: &str) -> bool {
    if let Some(pos) = text.find(class_name) {
        let after_name = &text[pos + class_name.len()..];
        let trimmed = after_name.trim_start();
        return trimmed.starts_with('(');
    }
    false
}

/// Extract category name from parentheses after class name.
fn extract_category_name(text: &str, class_name: &str) -> Option<String> {
    let pos = text.find(class_name)?;
    let after_name = &text[pos + class_name.len()..];
    let trimmed = after_name.trim_start();
    if !trimmed.starts_with('(') {
        return None;
    }
    let inner = &trimmed[1..];
    let end = inner.find(')')?;
    let cat_name = inner[..end].trim().to_string();
    Some(cat_name)
}

/// Extract superclass name from @interface declaration.
/// Pattern: `@interface ClassName : SuperClass`
fn extract_superclass(text: &str) -> Option<String> {
    // Find the colon that separates class name from superclass
    // But skip colons inside protocol lists <...>
    let first_line = text.lines().next().unwrap_or(text);
    // Remove anything in angle brackets
    let without_protocols = if let Some(angle_start) = first_line.find('<') {
        &first_line[..angle_start]
    } else {
        first_line
    };

    if let Some(colon_pos) = without_protocols.find(':') {
        let after_colon = &without_protocols[colon_pos + 1..];
        let super_name: String = after_colon
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !super_name.is_empty() {
            return Some(super_name);
        }
    }
    None
}

/// Extract the method selector from method text.
///
/// Handles:
/// - `- (void)simpleMethod` -> "simpleMethod"
/// - `+ (id)classMethod` -> "classMethod"
/// - `- (void)method:(Type)arg param:(Type)arg2` -> "method:param:"
/// - `- (void)method:(Type)arg` -> "method"
fn extract_method_selector(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Skip the +/- prefix
    let after_prefix = if trimmed.starts_with('-') || trimmed.starts_with('+') {
        &trimmed[1..]
    } else {
        return String::new();
    };

    // Skip the return type in parentheses
    let after_type = skip_parenthesized(after_prefix.trim_start());

    // Now extract the selector parts
    let selector_text = after_type.trim_start();

    // Get the first identifier (method name or first keyword)
    let first_part: String = selector_text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if first_part.is_empty() {
        return String::new();
    }

    // Check if there's a colon after the first part (keyword selector)
    let rest = &selector_text[first_part.len()..];
    if !rest.trim_start().starts_with(':') {
        // Simple selector (no parameters)
        return first_part;
    }

    // For simplicity, just return the first keyword (before the colon)
    // This matches how methods are typically referenced
    first_part
}

/// Skip a parenthesized expression and return the remaining text.
fn skip_parenthesized(text: &str) -> &str {
    if !text.starts_with('(') {
        return text;
    }
    let mut depth = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[i + 1..];
                }
            }
            _ => {}
        }
    }
    text
}

/// Extract a C function name from function definition text.
fn extract_c_function_name(text: &str) -> String {
    // Look for pattern: type identifier(
    // Find the opening paren
    if let Some(paren_pos) = text.find('(') {
        let before_paren = text[..paren_pos].trim();
        // The function name is the last identifier before the paren
        let name: String = before_paren
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
        {
            return name;
        }
    }
    String::new()
}

/// Extract the function name from a C-style call expression.
fn extract_c_call_name(text: &str) -> String {
    if let Some(paren_pos) = text.find('(') {
        let before_paren = text[..paren_pos].trim();
        let name: String = before_paren
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return name;
    }
    String::new()
}

/// Extract receiver and selector from a message expression text.
/// Input: `[receiver selector:arg1 param:arg2]` or `[receiver selector]`
/// Returns: (receiver, selector_first_keyword)
fn extract_message_parts(text: &str) -> (String, String) {
    let trimmed = text.trim();
    // Remove outer brackets
    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    let inner = inner.trim();
    if inner.is_empty() {
        return (String::new(), String::new());
    }

    // The receiver is the first token (could be an identifier, nested message, etc.)
    // For simplicity, take the first word or nested expression
    let (receiver, rest) = split_receiver_and_message(inner);

    // The selector is the first identifier in the rest
    let selector: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    (receiver, selector)
}

/// Split a message expression into receiver and message parts.
fn split_receiver_and_message(text: &str) -> (String, String) {
    let text = text.trim();

    // If receiver is a nested message expression [...]
    if text.starts_with('[') {
        let mut depth = 0;
        for (i, ch) in text.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        let receiver = text[..=i].to_string();
                        let rest = text[i + 1..].to_string();
                        return (receiver, rest);
                    }
                }
                _ => {}
            }
        }
        return (text.to_string(), String::new());
    }

    // Otherwise, receiver is the first whitespace-delimited token
    if let Some(space_pos) = text.find(|c: char| c.is_whitespace()) {
        let receiver = text[..space_pos].to_string();
        let rest = text[space_pos..].to_string();
        (receiver, rest)
    } else {
        (text.to_string(), String::new())
    }
}

/// Returns true for node kinds that should not be recursed into for definitions.
fn is_leaf_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "string_literal"
            | "number_literal"
            | "char_literal"
            | "comment"
            | "line_comment"
            | "block_comment"
            | "nil"
            | "true"
            | "false"
            | "self"
            | "super"
            | "type_identifier"
    )
}

/// Check if a name is an Objective-C keyword that should not be treated as a call.
fn is_objc_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else" | "for" | "while" | "do" | "switch" | "case"
            | "break" | "continue" | "return" | "goto" | "sizeof"
            | "typedef" | "struct" | "union" | "enum" | "static"
            | "extern" | "const" | "volatile" | "register" | "auto"
            | "void" | "int" | "char" | "float" | "double" | "long"
            | "short" | "unsigned" | "signed" | "self" | "super"
            | "nil" | "NULL" | "YES" | "NO" | "true" | "false"
            | "id" | "Class" | "SEL" | "IMP" | "BOOL"
    )
}

/// Check if a name is a common Objective-C/C macro that should not be treated as a user call.
fn is_common_macro(name: &str) -> bool {
    matches!(
        name,
        "NSLog" | "NSAssert" | "NSCAssert" | "dispatch_async"
            | "dispatch_sync" | "dispatch_once" | "sizeof"
            | "offsetof" | "va_start" | "va_end" | "va_arg"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_objc(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_objc::LANGUAGE.into())
            .expect("Objective-C grammar should load");
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_class_interface_and_implementation() {
        let source = r#"
@interface UserController : NSObject
- (void)loadData;
@end

@implementation UserController
- (void)loadData {
    // implementation
}
@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "Classes/UserController.m", source);

        // Should have at least one Class node for UserController
        let class_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class && n.fqn.contains("UserController"))
            .collect();
        assert!(!class_nodes.is_empty(), "Should extract UserController class");

        // Should have method nodes
        let method_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function && n.fqn.contains("loadData"))
            .collect();
        assert!(!method_nodes.is_empty(), "Should extract loadData method");
    }

    #[test]
    fn test_extract_protocol() {
        let source = r#"
@protocol DataSource
- (NSInteger)numberOfItems;
- (id)itemAtIndex:(NSInteger)index;
@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "Protocols/DataSource.h", source);

        // Should have a protocol node
        let protocol_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Interface && n.fqn.contains("DataSource"))
            .collect();
        assert!(
            !protocol_nodes.is_empty(),
            "Should extract DataSource protocol"
        );

        // Should have method declarations
        let method_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(
            !method_nodes.is_empty(),
            "Should extract protocol method declarations"
        );
    }

    #[test]
    fn test_extract_category() {
        let source = r#"
@interface NSString (Utilities)
- (BOOL)isValidEmail;
@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "Categories/NSString+Utilities.h", source);

        // Should have a category node
        let cat_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class && n.fqn.contains("Utilities"))
            .collect();
        assert!(!cat_nodes.is_empty(), "Should extract category node");

        // Check category attribute
        if let Some(cat_node) = cat_nodes.first() {
            let attrs = cat_node.attributes.as_object().unwrap();
            assert!(
                attrs.contains_key("category"),
                "Category node should have category attribute"
            );
        }
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
#import <Foundation/Foundation.h>
#import "AppDelegate.h"
@import UIKit;
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "App/Main.m", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        // Should have at least the framework and local imports
        assert!(
            imports.iter().any(|e| e.target_fqn == "Foundation/Foundation.h"),
            "Should extract Foundation import. Got: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "AppDelegate.h"),
            "Should extract local import. Got: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_message_sends() {
        let source = r#"
@implementation AppController
- (void)doWork {
    [self loadData];
    [self.delegate didFinish];
}
- (void)loadData {
}
@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "App/AppController.m", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // Should have call edges for message sends
        assert!(!calls.is_empty(), "Should extract message send calls");
    }

    #[test]
    fn test_extract_class_and_instance_methods() {
        let source = r#"
@implementation Singleton
+ (instancetype)sharedInstance {
    static Singleton *instance = nil;
    return instance;
}
- (void)doSomething {
}
@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "App/Singleton.m", source);

        let methods: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();

        // Should have both class and instance methods
        let has_class_method = methods.iter().any(|n| {
            n.attributes
                .get("method_type")
                .and_then(|v| v.as_str())
                == Some("class")
        });
        let has_instance_method = methods.iter().any(|n| {
            n.attributes
                .get("method_type")
                .and_then(|v| v.as_str())
                == Some("instance")
        });

        assert!(has_class_method, "Should extract class method (+)");
        assert!(has_instance_method, "Should extract instance method (-)");
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let tree = parse_objc(source);
        let result = extract(&tree, "empty.m", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_extract_regex_backward_compat() {
        let source = r#"
@implementation Simple
- (void)hello {
}
@end
"#;
        #[allow(deprecated)]
        let result = extract_regex("test.m", source);
        // Should produce some nodes (backward compatibility)
        assert!(!result.nodes.is_empty(), "extract_regex should still work");
    }

    #[test]
    fn test_full_objc_file() {
        let source = r#"
#import <Foundation/Foundation.h>
#import "AppDelegate.h"

@protocol DataSource
- (NSInteger)numberOfItems;
@end

@interface UserController : NSObject <DataSource>
@property (nonatomic, strong) NSString *name;
- (void)loadData;
+ (instancetype)sharedInstance;
@end

@implementation UserController

- (void)loadData {
    NSLog(@"Loading data...");
    [self.delegate numberOfItems];
}

+ (instancetype)sharedInstance {
    static UserController *instance = nil;
    return instance;
}

- (NSInteger)numberOfItems {
    return 42;
}

@end
"#;
        let tree = parse_objc(source);
        let result = extract(&tree, "Classes/UserController.m", source);

        // Verify we get a mix of nodes and edges
        assert!(!result.nodes.is_empty(), "Should extract nodes");
        assert!(!result.edges.is_empty(), "Should extract edges (imports and/or calls)");

        // Check that we have imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(!imports.is_empty(), "Should have import edges");

        // Check that we have class nodes
        let classes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert!(!classes.is_empty(), "Should have class nodes");

        // Check that we have function nodes
        let functions: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(!functions.is_empty(), "Should have function nodes");
    }
}