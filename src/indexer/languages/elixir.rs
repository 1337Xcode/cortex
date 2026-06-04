//! Elixir AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (modules, functions, macros, protocols)
//! and edges (imports, intra-file calls) from a tree-sitter Elixir parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Elixir file.
///
/// Handles:
/// - Modules (`defmodule Name do ... end`)
/// - Public functions (`def name(...) do ... end`)
/// - Private functions (`defp name(...) do ... end`)
/// - Public macros (`defmacro name(...) do ... end`)
/// - Private macros (`defmacrop name(...) do ... end`)
/// - Protocols (`defprotocol Name do ... end`)
/// - Protocol implementations (`defimpl Protocol, for: Type do ... end`)
/// - Imports (`import Module`, `alias Module`, `use Module`, `require Module`)
/// - Intra-file function calls
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // First pass: collect all definitions (for intra-file call resolution)
    let mut defined_fqns: Vec<(String, String)> = Vec::new(); // (simple_name, fqn)
    let mut module_stack: Vec<String> = Vec::new();

    collect_definitions(
        root,
        file,
        source_bytes,
        &mut nodes,
        &mut defined_fqns,
        &mut module_stack,
    );

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
        .set_language(&tree_sitter_elixir::LANGUAGE.into())
        .expect("Elixir grammar should load");
    match parser.parse(source, None) {
        Some(tree) => extract(&tree, file, source),
        None => ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        },
    }
}

/// Compute cyclomatic complexity for all Function nodes.
fn compute_node_complexities(nodes: &mut [Node], root: tree_sitter::Node, source: &[u8]) {
    for node in nodes.iter_mut() {
        if node.kind == NodeKind::Function {
            // In tree-sitter-elixir, function definitions are `call` nodes
            // with the function name being `def`, `defp`, `defmacro`, `defmacrop`
            if let Some(ast_node) = find_ast_node_at_line(root, node.start_line, "call") {
                let c = complexity::compute_full_complexity(ast_node, source, "elixir");
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

/// Recursively collect definitions from the AST.
///
/// In tree-sitter-elixir, most Elixir constructs (defmodule, def, defp, defmacro,
/// defprotocol, defimpl, import, alias, use, require) are represented as `call` nodes
/// where the first child is an `identifier` with the keyword name.
fn collect_definitions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                // In tree-sitter-elixir, `call` nodes represent function calls
                // including special forms like defmodule, def, defp, etc.
                let call_name = get_call_identifier(child, source);
                match call_name.as_str() {
                    "defmodule" => {
                        extract_module(child, file, source, nodes, defined_fqns, module_stack);
                    }
                    "def" => {
                        extract_function_def(
                            child,
                            file,
                            source,
                            nodes,
                            defined_fqns,
                            module_stack,
                            "public",
                        );
                    }
                    "defp" => {
                        extract_function_def(
                            child,
                            file,
                            source,
                            nodes,
                            defined_fqns,
                            module_stack,
                            "private",
                        );
                    }
                    "defmacro" => {
                        extract_macro_def(
                            child,
                            file,
                            source,
                            nodes,
                            defined_fqns,
                            module_stack,
                            "public",
                        );
                    }
                    "defmacrop" => {
                        extract_macro_def(
                            child,
                            file,
                            source,
                            nodes,
                            defined_fqns,
                            module_stack,
                            "private",
                        );
                    }
                    "defprotocol" => {
                        extract_protocol(child, file, source, nodes, defined_fqns, module_stack);
                    }
                    "defimpl" => {
                        extract_impl(child, file, source, nodes, defined_fqns, module_stack);
                    }
                    _ => {
                        // Recurse into other call nodes (they may contain nested definitions)
                        collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
                    }
                }
            }
            _ => {
                // Recurse into other node types
                if child.child_count() > 0 {
                    collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
                }
            }
        }
    }
}

/// Extract a module definition (`defmodule Name do ... end`).
fn extract_module(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    let module_name = get_defmodule_name(node, source);
    if module_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{module_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
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
        defined_fqns.push((module_name.clone(), fqn));
    }

    // Push module onto stack and recurse into the body
    module_stack.push(module_name);
    collect_definitions_in_body(node, file, source, nodes, defined_fqns, module_stack);
    module_stack.pop();
}

/// Extract a public or private function definition.
fn extract_function_def(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
    visibility: &str,
) {
    let func_name = get_def_name(node, source);
    if func_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &func_name);

    // Elixir allows multiple function clauses with the same name;
    // only create the node once, but extend end_line if needed
    if let Some(existing) = nodes.iter_mut().find(|n| n.fqn == fqn) {
        if end_line > existing.end_line {
            existing.end_line = end_line;
        }
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Function,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"visibility": visibility}),
    });

    defined_fqns.push((func_name, fqn));
}

/// Extract a macro definition (defmacro/defmacrop).
fn extract_macro_def(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
    visibility: &str,
) {
    let macro_name = get_def_name(node, source);
    if macro_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &macro_name);

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Function,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"visibility": visibility, "macro": true}),
    });

    defined_fqns.push((macro_name, fqn));
}

/// Extract a protocol definition (`defprotocol Name do ... end`).
fn extract_protocol(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    let protocol_name = get_defmodule_name(node, source);
    if protocol_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{protocol_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Interface,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"protocol": true}),
        });
        defined_fqns.push((protocol_name.clone(), fqn));
    }

    // Recurse into protocol body for function specs
    module_stack.push(protocol_name);
    collect_definitions_in_body(node, file, source, nodes, defined_fqns, module_stack);
    module_stack.pop();
}

/// Extract a protocol implementation (`defimpl Protocol, for: Type do ... end`).
fn extract_impl(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    let (protocol_name, for_type) = get_defimpl_names(node, source);
    if protocol_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let impl_name = if for_type.is_empty() {
        protocol_name.clone()
    } else {
        format!("{protocol_name}.{for_type}")
    };

    let fqn = format!("{file}::{impl_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"impl": true, "protocol": protocol_name, "for_type": for_type}),
        });
        defined_fqns.push((impl_name.clone(), fqn));
    }

    // Recurse into impl body for function definitions
    module_stack.push(impl_name);
    collect_definitions_in_body(node, file, source, nodes, defined_fqns, module_stack);
    module_stack.pop();
}

/// Collect definitions from the body (do-block) of a module/protocol/impl.
fn collect_definitions_in_body(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    // In tree-sitter-elixir, the do-block is typically the last argument
    // or a `do_block` child of the call node.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "do_block" | "do" => {
                collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
            }
            "arguments" => {
                // The do block might be nested inside arguments
                let mut arg_cursor = child.walk();
                for arg_child in child.children(&mut arg_cursor) {
                    if arg_child.kind() == "do_block" || arg_child.kind() == "do" {
                        collect_definitions(
                            arg_child,
                            file,
                            source,
                            nodes,
                            defined_fqns,
                            module_stack,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Collect import/alias/use/require edges from the AST.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            let call_name = get_call_identifier(child, source);
            match call_name.as_str() {
                "import" | "alias" | "use" | "require" => {
                    if let Some(target) = get_import_target(child, source) {
                        // Avoid duplicate imports
                        if !edges.iter().any(|e| {
                            e.kind == EdgeKind::Imports
                                && e.source_fqn == file
                                && e.target_fqn == target
                        }) {
                            edges.push(Edge {
                                id: None,
                                source_fqn: file.to_string(),
                                target_fqn: target,
                                kind: EdgeKind::Imports,
                                confidence: 1.0,
                                edge_source: crate::store::confidence::EdgeSource::AstDirect,
                                attributes: json!({"import_type": call_name}),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Recurse into children
        if child.child_count() > 0 {
            collect_imports(child, file, source, edges);
        }
    }
}

/// Collect intra-file function calls.
/// A call is a `call` node whose target matches a defined function.
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
            let call_name = get_call_identifier(child, source);

            // Skip special forms and import-like calls
            if !is_special_form(&call_name) && !call_name.is_empty() {
                extract_call_edge(child, file, source, &call_name, defined_fqns, edges);
            }
        }

        // Recurse into children
        if child.child_count() > 0 {
            collect_calls(child, file, source, defined_fqns, edges);
        }
    }
}

/// Extract a call edge from a call node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    call_name: &str,
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    // Determine the source FQN (enclosing function or file-level)
    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Check for dot-call (Module.function or variable.method)
    if call_name.contains('.') {
        let parts: Vec<&str> = call_name.rsplitn(2, '.').collect();
        if parts.len() == 2 {
            let method = parts[0];
            let receiver = parts[1];

            // Try to resolve to a defined function
            if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == method)
            {
                if source_fqn != *target_fqn {
                    edges.push(Edge {
                        id: None,
                        source_fqn: source_fqn.clone(),
                        target_fqn: target_fqn.clone(),
                        kind: EdgeKind::Calls,
                        confidence: 1.0,
                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                        attributes: json!({"receiver": receiver, "call_type": "method"}),
                    });
                }
            } else {
                // Emit unresolved call edge for cross-file resolution
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.clone(),
                    target_fqn: call_name.to_string(),
                    kind: EdgeKind::Calls,
                    confidence: 0.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: json!({"receiver": receiver, "call_type": "qualified"}),
                });
            }
            return;
        }
    }

    // Simple function call - try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == call_name)
        && source_fqn != *target_fqn
    {
        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: target_fqn.clone(),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            edge_source: crate::store::confidence::EdgeSource::AstDirect,
            attributes: json!({}),
        });
    }
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Get the identifier (function name) from a `call` node.
///
/// In tree-sitter-elixir, a `call` node typically has structure:
/// - (call target: (identifier) arguments: (arguments ...))
/// - (call target: (dot left: ... right: (identifier)) arguments: ...)
fn get_call_identifier(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the "target" field first (tree-sitter-elixir uses this)
    if let Some(target) = node.child_by_field_name("target") {
        match target.kind() {
            "identifier" | "atom" => {
                return target.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "dot" => {
                // Dot access: Module.function
                return target.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                return target.utf8_text(source).unwrap_or("").trim().to_string();
            }
        }
    }

    // Fallback: look for the first identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "dot" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {}
        }
    }

    String::new()
}

/// Get the module name from a `defmodule` or `defprotocol` call node.
///
/// The module name is the first argument, which can be:
/// - A simple alias: `MyModule`
/// - A dotted alias: `MyApp.UserController`
fn get_defmodule_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Look for the arguments node, then find the alias/atom within
    if let Some(args) = node.child_by_field_name("arguments") {
        return get_first_alias_from_args(args, source);
    }

    // Fallback: look through children for arguments
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            return get_first_alias_from_args(child, source);
        }
    }

    // Last resort: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    extract_name_from_def_text(&text, "defmodule")
        .or_else(|| extract_name_from_def_text(&text, "defprotocol"))
        .unwrap_or_default()
}

/// Get the function/macro name from a `def`/`defp`/`defmacro`/`defmacrop` call node.
///
/// The function name is extracted from the first argument which is typically
/// a call node (the function head) or an identifier.
fn get_def_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Look for the arguments node
    if let Some(args) = node.child_by_field_name("arguments") {
        return get_function_name_from_args(args, source);
    }

    // Fallback: look through children for arguments
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            return get_function_name_from_args(child, source);
        }
    }

    // Last resort: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    for prefix in &["def ", "defp ", "defmacro ", "defmacrop "] {
        if let Some(name) = extract_name_from_def_text(&text, prefix.trim()) {
            return name;
        }
    }

    String::new()
}

/// Get the protocol and for-type names from a `defimpl` call node.
fn get_defimpl_names(node: tree_sitter::Node, source: &[u8]) -> (String, String) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Pattern: defimpl ProtocolName, for: TypeName do ... end
    // Or: defimpl ProtocolName do ... end (inside the type module)
    let after_defimpl = text.strip_prefix("defimpl").unwrap_or("").trim();

    // Extract protocol name (first identifier/alias before comma or 'do')
    let protocol_name: String = after_defimpl
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();

    // Look for "for:" keyword
    let for_type = if let Some(for_pos) = text.find("for:") {
        let after_for = text[for_pos + 4..].trim();
        let type_name: String = after_for
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        type_name
    } else {
        String::new()
    };

    (protocol_name, for_type)
}

/// Get the first alias (module name) from an arguments node.
fn get_first_alias_from_args(args_node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        match child.kind() {
            "alias" | "atom" | "identifier" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            // Handle dotted aliases like MyApp.UserController
            "dot" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                // Check if the child text looks like a module name (starts with uppercase)
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && text.chars().next().is_some_and(|c| c.is_uppercase())
                    && text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                {
                    return text;
                }
            }
        }
    }
    String::new()
}

/// Get the function name from the arguments of a def/defp call.
///
/// The first argument to `def` is typically the function head:
/// - `def name(args)` -> name is a call node
/// - `def name` -> name is an identifier
/// - `def name(args) when guard` -> name is inside a binary_operator
fn get_function_name_from_args(args_node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                // The function head is itself a call: `name(args)`
                // Get the target/identifier of this call
                return get_call_identifier(child, source);
            }
            "identifier" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "binary_operator" => {
                // Guard clause: `name(args) when condition`
                // The left side contains the function head
                if let Some(left) = child.child_by_field_name("left") {
                    match left.kind() {
                        "call" => return get_call_identifier(left, source),
                        "identifier" => {
                            return left.utf8_text(source).unwrap_or("").trim().to_string();
                        }
                        _ => {}
                    }
                }
                // Fallback: first child
                let mut bc = child.walk();
                for bchild in child.children(&mut bc) {
                    if bchild.kind() == "call" {
                        return get_call_identifier(bchild, source);
                    }
                    if bchild.kind() == "identifier" {
                        return bchild.utf8_text(source).unwrap_or("").trim().to_string();
                    }
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// Get the import target module name from an import/alias/use/require call.
fn get_import_target(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for arguments
    if let Some(args) = node.child_by_field_name("arguments") {
        let target = get_first_alias_from_args(args, source);
        if !target.is_empty() {
            return Some(target);
        }
    }

    // Fallback: look through children for arguments
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let target = get_first_alias_from_args(child, source);
            if !target.is_empty() {
                return Some(target);
            }
        }
    }

    // Last resort: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    for prefix in &["import ", "alias ", "use ", "require "] {
        if let Some(after) = text.strip_prefix(prefix) {
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }

    None
}

/// Extract a name from definition text (fallback when AST traversal fails).
fn extract_name_from_def_text(text: &str, keyword: &str) -> Option<String> {
    let after = text.strip_prefix(keyword)?.trim_start();
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Build a fully qualified name from file, module stack, and symbol name.
fn build_fqn(file: &str, module_stack: &[String], name: &str) -> String {
    if module_stack.is_empty() {
        format!("{file}::{name}")
    } else {
        let module_path = module_stack.join("::");
        format!("{file}::{module_path}::{name}")
    }
}

/// Check if a call name is a special form that should not be treated as a regular call.
fn is_special_form(name: &str) -> bool {
    matches!(
        name,
        "defmodule"
            | "def"
            | "defp"
            | "defmacro"
            | "defmacrop"
            | "defprotocol"
            | "defimpl"
            | "defstruct"
            | "defexception"
            | "defguard"
            | "defguardp"
            | "defdelegate"
            | "defoverridable"
            | "import"
            | "alias"
            | "use"
            | "require"
            | "if"
            | "unless"
            | "case"
            | "cond"
            | "with"
            | "for"
            | "receive"
            | "try"
            | "raise"
            | "throw"
            | "quote"
            | "unquote"
            | "super"
    )
}

/// Find the enclosing function definition for a given node and return its FQN.
fn find_enclosing_function_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    let mut module_parts: Vec<String> = Vec::new();

    while let Some(parent) = current {
        if parent.kind() == "call" {
            let call_name = get_call_identifier(parent, source);
            match call_name.as_str() {
                "def" | "defp" | "defmacro" | "defmacrop" => {
                    let func_name = get_def_name(parent, source);
                    if !func_name.is_empty() {
                        if module_parts.is_empty() {
                            return Some(format!("{file}::{func_name}"));
                        } else {
                            module_parts.reverse();
                            let module_path = module_parts.join("::");
                            return Some(format!("{file}::{module_path}::{func_name}"));
                        }
                    }
                }
                "defmodule" | "defprotocol" | "defimpl" => {
                    let mod_name = get_defmodule_name(parent, source);
                    if !mod_name.is_empty() {
                        module_parts.push(mod_name);
                    }
                }
                _ => {}
            }
        }
        current = parent.parent();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Elixir source and run the extractor.
    fn parse_elixir(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .expect("Elixir grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, "lib/my_app.ex", source)
    }

    #[test]
    fn test_module_extraction() {
        let source = r#"
defmodule MyApp.UserController do
  def index(conn, _params) do
    :ok
  end
end
"#;
        let result = parse_elixir(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::MyApp.UserController"
                    && n.kind == NodeKind::Module),
            "Should find MyApp.UserController module"
        );
    }

    #[test]
    fn test_public_and_private_functions() {
        let source = r#"
defmodule MyApp.Users do
  def get_user(id) do
    Repo.get(User, id)
  end

  def list_users do
    Repo.all(User)
  end

  defp validate(user) do
    user
  end
end
"#;
        let result = parse_elixir(source);

        // Check public functions
        let public_fns: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| {
                n.kind == NodeKind::Function
                    && n.attributes.get("visibility").and_then(|v| v.as_str()) == Some("public")
            })
            .collect();

        assert!(
            public_fns.iter().any(|n| n.fqn.ends_with("::get_user")),
            "Should find get_user as public function"
        );
        assert!(
            public_fns.iter().any(|n| n.fqn.ends_with("::list_users")),
            "Should find list_users as public function"
        );

        // Check private function
        let private_fns: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| {
                n.kind == NodeKind::Function
                    && n.attributes.get("visibility").and_then(|v| v.as_str()) == Some("private")
            })
            .collect();

        assert!(
            private_fns.iter().any(|n| n.fqn.ends_with("::validate")),
            "Should find validate as private function"
        );
    }

    #[test]
    fn test_macro_extraction() {
        let source = r#"
defmodule MyApp.Macros do
  defmacro my_macro(expr) do
    quote do
      unquote(expr)
    end
  end

  defmacrop private_macro(x) do
    quote do: unquote(x)
  end
end
"#;
        let result = parse_elixir(source);

        let macros: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| {
                n.kind == NodeKind::Function
                    && n.attributes.get("macro").and_then(|v| v.as_bool()) == Some(true)
            })
            .collect();

        assert!(
            macros.iter().any(|n| n.fqn.ends_with("::my_macro")),
            "Should find my_macro"
        );
        assert!(
            macros.iter().any(|n| n.fqn.ends_with("::private_macro")),
            "Should find private_macro"
        );
    }

    #[test]
    fn test_protocol_extraction() {
        let source = r#"
defprotocol Printable do
  def print(data)
end
"#;
        let result = parse_elixir(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::Printable" && n.kind == NodeKind::Interface),
            "Should find Printable protocol as Interface"
        );
    }

    #[test]
    fn test_defimpl_extraction() {
        let source = r#"
defimpl Printable, for: Integer do
  def print(data) do
    IO.puts(data)
  end
end
"#;
        let result = parse_elixir(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::Printable.Integer" && n.kind == NodeKind::Class),
            "Should find Printable.Integer impl as Class"
        );
    }

    #[test]
    fn test_imports() {
        let source = r#"
defmodule MyApp.UserController do
  use MyApp.Web, :controller
  alias MyApp.Repo
  import Ecto.Query
  require Logger

  def index(conn, _params) do
    :ok
  end
end
"#;
        let result = parse_elixir(source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "MyApp.Web"),
            "Should find use MyApp.Web import"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "MyApp.Repo"),
            "Should find alias MyApp.Repo import"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Ecto.Query"),
            "Should find import Ecto.Query"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Logger"),
            "Should find require Logger"
        );
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"
defmodule MyApp.Users do
  def get_user(id) do
    validate(id)
  end

  defp validate(id) do
    id
  end
end
"#;
        let result = parse_elixir(source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // get_user should call validate
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn.ends_with("::get_user")
                    && e.target_fqn.ends_with("::validate")),
            "get_user should call validate. Calls found: {:?}",
            calls
                .iter()
                .map(|e| format!("{} -> {}", e.source_fqn, e.target_fqn))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_elixir("");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_combined_extraction() {
        let source = r#"
defmodule MyApp.UserController do
  use MyApp.Web, :controller
  alias MyApp.Repo
  import Ecto.Query

  def index(conn, _params) do
    users = list_users()
    render(conn, "index.html", users: users)
  end

  def show(conn, %{"id" => id}) do
    user = Repo.get!(User, id)
    render(conn, "show.html", user: user)
  end

  defp list_users do
    Repo.all(User)
  end
end
"#;
        let result = parse_elixir(source);

        // Check module
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::MyApp.UserController"
                    && n.kind == NodeKind::Module),
            "Should find module"
        );

        // Check functions exist
        let functions: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(
            functions.iter().any(|n| n.fqn.ends_with("::index")),
            "Should find index function"
        );
        assert!(
            functions.iter().any(|n| n.fqn.ends_with("::show")),
            "Should find show function"
        );
        assert!(
            functions.iter().any(|n| n.fqn.ends_with("::list_users")),
            "Should find list_users function"
        );

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.len() >= 3, "Should have at least 3 imports");
    }

    #[test]
    fn test_multiple_function_clauses() {
        let source = r#"
defmodule MyApp.Math do
  def factorial(0), do: 1
  def factorial(n) when n > 0 do
    n * factorial(n - 1)
  end
end
"#;
        let result = parse_elixir(source);

        // Should only create one node for factorial (multiple clauses)
        let factorial_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.fqn.ends_with("::factorial"))
            .collect();

        assert_eq!(
            factorial_nodes.len(),
            1,
            "Should have exactly one factorial node (multiple clauses merged)"
        );
    }

    #[test]
    fn test_nested_modules() {
        let source = r#"
defmodule MyApp do
  defmodule Inner do
    def hello do
      :world
    end
  end
end
"#;
        let result = parse_elixir(source);

        // Should find both modules
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::MyApp" && n.kind == NodeKind::Module),
            "Should find outer module MyApp"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "lib/my_app.ex::Inner" && n.kind == NodeKind::Module),
            "Should find inner module Inner"
        );
    }

    #[test]
    fn test_complexity_computed() {
        let source = r#"
defmodule MyApp.Complex do
  def complex_func(x) do
    if x > 0 do
      case x do
        1 -> :one
        2 -> :two
        _ -> :other
      end
    else
      :negative
    end
  end
end
"#;
        let result = parse_elixir(source);

        let func = result
            .nodes
            .iter()
            .find(|n| n.fqn.ends_with("::complex_func"));

        assert!(func.is_some(), "Should find complex_func");
        if let Some(f) = func {
            let complexity = f.attributes.get("complexity");
            // Complexity should be computed (at least 1 for base)
            assert!(
                complexity.is_some(),
                "Should have complexity attribute computed"
            );
            if let Some(c) = complexity {
                assert!(
                    c.as_u64().unwrap_or(0) >= 1,
                    "Complexity should be at least 1"
                );
            }
        }
    }
}
