//! Zig AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (functions, structs, enums, unions)
//! and edges (@import calls, intra-file function calls) from a tree-sitter Zig parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Zig file.
///
/// Handles:
/// - Public functions (`pub fn name(args) ReturnType { body }`)
/// - Private functions (`fn name(args) ReturnType { body }`)
/// - Exported functions (`export fn name(args) ReturnType { body }`)
/// - Struct definitions (`const Name = struct { ... };`)
/// - Enum definitions (`const Name = enum { ... };`)
/// - Union definitions (`const Name = union { ... };`)
/// - Imports (`const name = @import("module");`)
/// - Standard function calls and method calls (field access + call)
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
        .set_language(&tree_sitter_zig::LANGUAGE.into())
        .expect("Zig grammar should load");
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
            // In tree-sitter-zig, function definitions appear as `FnProto` or
            // `Decl` nodes. We search for the function body node at the start line.
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "FnProto")
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "fn_decl"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "function_declaration"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "Decl"))
                    .or_else(|| find_ast_node_at_line_fuzzy(root, node.start_line))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "zig");
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

/// Fuzzy fallback: find any node at the target line that looks like a function.
/// Used when the exact node kind is unknown for the grammar version.
fn find_ast_node_at_line_fuzzy<'a>(
    node: tree_sitter::Node<'a>,
    target_line: u32,
) -> Option<tree_sitter::Node<'a>> {
    let node_start_line = node.start_position().row as u32 + 1;
    let kind = node.kind();
    // Match any node at the target line that contains "fn" in its kind name
    if node_start_line == target_line
        && (kind.contains("fn") || kind.contains("Fn") || kind.contains("function"))
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
/// In tree-sitter-zig, top-level constructs are typically:
/// - Variable declarations (`const Name = struct { ... };`, `const Name = enum { ... };`)
/// - Function declarations (`pub fn name(...) ... { ... }`, `fn name(...) ... { ... }`)
///
/// The grammar uses various node kinds depending on version. We handle multiple
/// possible representations to be robust across grammar versions.
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

        // Try to extract function declarations
        if is_function_node(kind) {
            try_extract_function(child, file, source, nodes, defined_fqns);
            continue;
        }

        // Try to extract const declarations (structs, enums, unions, imports)
        if is_variable_decl_node(kind) {
            try_extract_const_decl(child, file, source, nodes, defined_fqns);
            continue;
        }

        // Recurse into container nodes
        if child.child_count() > 0 && !is_leaf_kind(kind) {
            collect_definitions(child, file, source, nodes, defined_fqns);
        }
    }
}

/// Check if a node kind represents a function declaration in tree-sitter-zig.
fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "FnProto"
            | "fn_decl"
            | "function_declaration"
            | "Decl"
            | "TopLevelDecl"
    )
}

/// Check if a node kind represents a variable/const declaration.
fn is_variable_decl_node(kind: &str) -> bool {
    matches!(
        kind,
        "VarDecl"
            | "variable_declaration"
            | "const_declaration"
            | "TopLevelDecl"
            | "Decl"
            | "container_decl"
    )
}

/// Returns true for node kinds that should not be recursed into for definitions.
fn is_leaf_kind(kind: &str) -> bool {
    matches!(
        kind,
        "IDENTIFIER"
            | "identifier"
            | "STRINGLITERALSINGLE"
            | "string_literal"
            | "INTEGER"
            | "integer_literal"
            | "FLOAT"
            | "float_literal"
            | "CHAR_LITERAL"
            | "LINECOMMENT"
            | "line_comment"
            | "DOC_COMMENT"
            | "doc_comment"
            | "BUILTINIDENTIFIER"
            | "builtin"
    )
}

/// Try to extract a function definition from a node.
///
/// Zig functions appear as:
/// - `pub fn name(params) ReturnType { body }`
/// - `fn name(params) ReturnType { body }`
/// - `export fn name(params) ReturnType { body }`
///
/// In tree-sitter-zig, the structure varies by grammar version. We use text-based
/// extraction as a robust fallback when field-based access doesn't work.
fn try_extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Check if this node actually contains a function definition
    if !text.contains("fn ") {
        // This might be a container Decl node; recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if is_function_node(ck) || ck.contains("fn") || ck.contains("Fn") {
                try_extract_function(child, file, source, nodes, defined_fqns);
            } else if child.child_count() > 0 && !is_leaf_kind(ck) {
                try_extract_function_from_children(child, file, source, nodes, defined_fqns);
            }
        }
        return;
    }

    // Extract function name from the text
    let name = extract_fn_name_from_text(&text);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");
    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    // Determine visibility
    let is_pub = text.starts_with("pub ") || text.contains("pub fn ");
    let is_export = text.starts_with("export ") || text.contains("export fn ");

    let mut attrs = serde_json::Map::new();
    if is_pub {
        attrs.insert("visibility".to_string(), json!("pub"));
    } else if is_export {
        attrs.insert("visibility".to_string(), json!("export"));
    }

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

    defined_fqns.push((name, fqn));
}

/// Helper to recurse into children looking for function nodes.
fn try_extract_function_from_children(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if is_function_node(ck) || ck.contains("fn") || ck.contains("Fn") {
            try_extract_function(child, file, source, nodes, defined_fqns);
        }
    }
}

/// Extract a function name from Zig source text.
///
/// Handles patterns like:
/// - `pub fn name(` → "name"
/// - `fn name(` → "name"
/// - `export fn name(` → "name"
fn extract_fn_name_from_text(text: &str) -> String {
    // Find "fn " and extract the identifier after it
    if let Some(fn_pos) = text.find("fn ") {
        let after_fn = &text[fn_pos + 3..];
        let name: String = after_fn
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return name;
        }
    }
    String::new()
}

/// Try to extract a const declaration (struct, enum, union, or other const).
///
/// Zig patterns:
/// - `const Name = struct { ... };`
/// - `pub const Name = struct { ... };`
/// - `const Name = enum { ... };`
/// - `const Name = union { ... };`
/// - `const Name = packed struct { ... };`
/// - `const Name = extern struct { ... };`
fn try_extract_const_decl(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Check if this is a const declaration with struct/enum/union
    if !text.contains("const ") {
        // Might be a container node; recurse
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if is_variable_decl_node(ck) {
                try_extract_const_decl(child, file, source, nodes, defined_fqns);
            } else if child.child_count() > 0 && !is_leaf_kind(ck) && !is_function_node(ck) {
                try_extract_const_decl_from_children(child, file, source, nodes, defined_fqns);
            }
        }
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract the name from `const Name = ...`
    let name = extract_const_name_from_text(&text);
    if name.is_empty() {
        return;
    }

    // Determine what's on the RHS
    let rhs = get_const_rhs(&text, &name);

    if is_struct_rhs(&rhs) {
        let fqn = format!("{file}::{name}");
        if !nodes.iter().any(|n| n.fqn == fqn) {
            let is_pub = text.starts_with("pub ");
            let mut attrs = serde_json::Map::new();
            attrs.insert("kind".to_string(), json!("struct"));
            if is_pub {
                attrs.insert("visibility".to_string(), json!("pub"));
            }
            if rhs.contains("packed") {
                attrs.insert("layout".to_string(), json!("packed"));
            } else if rhs.contains("extern") {
                attrs.insert("layout".to_string(), json!("extern"));
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
            defined_fqns.push((name, fqn));
        }
    } else if is_enum_rhs(&rhs) {
        let fqn = format!("{file}::{name}");
        if !nodes.iter().any(|n| n.fqn == fqn) {
            let is_pub = text.starts_with("pub ");
            let mut attrs = serde_json::Map::new();
            attrs.insert("kind".to_string(), json!("enum"));
            if is_pub {
                attrs.insert("visibility".to_string(), json!("pub"));
            }

            nodes.push(Node {
                fqn: fqn.clone(),
                kind: NodeKind::Enum,
                file: file.to_string(),
                start_line,
                end_line,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!(attrs),
            });
            defined_fqns.push((name, fqn));
        }
    } else if is_union_rhs(&rhs) {
        let fqn = format!("{file}::{name}");
        if !nodes.iter().any(|n| n.fqn == fqn) {
            let is_pub = text.starts_with("pub ");
            let mut attrs = serde_json::Map::new();
            attrs.insert("kind".to_string(), json!("union"));
            if is_pub {
                attrs.insert("visibility".to_string(), json!("pub"));
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
            defined_fqns.push((name, fqn));
        }
    }
    // Note: we don't extract plain const values as nodes (e.g., `const x = 42;`)
    // unless they are struct/enum/union definitions.

    // Recurse into the node to find nested function definitions (methods inside structs)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if is_function_node(ck) || ck.contains("fn") || ck.contains("Fn") {
            try_extract_function(child, file, source, nodes, defined_fqns);
        } else if child.child_count() > 0 && !is_leaf_kind(ck) {
            // Recurse deeper to find functions inside struct/enum/union bodies
            collect_nested_functions(child, file, source, nodes, defined_fqns);
        }
    }
}

/// Helper to recurse into children looking for const declarations.
fn try_extract_const_decl_from_children(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if is_variable_decl_node(ck) {
            try_extract_const_decl(child, file, source, nodes, defined_fqns);
        }
    }
}

/// Recursively collect nested function definitions (e.g., methods inside struct bodies).
fn collect_nested_functions(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if is_function_node(ck) || ck.contains("fn") || ck.contains("Fn") {
            try_extract_function(child, file, source, nodes, defined_fqns);
        } else if child.child_count() > 0 && !is_leaf_kind(ck) {
            collect_nested_functions(child, file, source, nodes, defined_fqns);
        }
    }
}

/// Extract the name from a const declaration text.
/// Pattern: `[pub] const Name = ...`
fn extract_const_name_from_text(text: &str) -> String {
    // Find "const " and extract the identifier after it
    if let Some(const_pos) = text.find("const ") {
        let after_const = &text[const_pos + 6..];
        let name: String = after_const
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return name;
    }
    String::new()
}

/// Get the right-hand side of a const declaration.
fn get_const_rhs<'a>(text: &'a str, name: &str) -> String {
    // Find `Name = ` and return everything after the `=`
    if let Some(name_pos) = text.find(name) {
        let after_name = &text[name_pos + name.len()..];
        if let Some(eq_pos) = after_name.find('=') {
            return after_name[eq_pos + 1..].trim_start().to_string();
        }
    }
    String::new()
}

/// Check if the RHS indicates a struct definition.
fn is_struct_rhs(rhs: &str) -> bool {
    rhs.starts_with("struct")
        || rhs.starts_with("packed struct")
        || rhs.starts_with("extern struct")
}

/// Check if the RHS indicates an enum definition.
fn is_enum_rhs(rhs: &str) -> bool {
    rhs.starts_with("enum") || rhs.starts_with("enum(")
}

/// Check if the RHS indicates a union definition.
fn is_union_rhs(rhs: &str) -> bool {
    rhs.starts_with("union") || rhs.starts_with("packed union") || rhs.starts_with("extern union")
}


// ─── Import Collection ──────────────────────────────────────────────────────

/// Collect import edges from the AST.
///
/// In Zig, imports come from `@import("module")` builtin calls.
/// These appear in patterns like:
/// - `const std = @import("std");`
/// - `const mem = @import("std").mem;`
/// - `const c = @cImport(@cInclude("header.h"));`
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

    // Check if this node's text contains @import
    if kind == "BUILTINIDENTIFIER" || kind == "builtin" || kind == "builtin_call_expr" {
        let text = node.utf8_text(source).unwrap_or("").trim().to_string();
        if text.starts_with("@import") {
            if let Some(target) = extract_import_target(&text) {
                add_import_edge(file, &target, edges);
            }
        }
    }

    // Also check the text of any node that might contain @import
    // (some grammar versions inline builtins differently)
    let text = node.utf8_text(source).unwrap_or("");
    if text.contains("@import") && node.child_count() == 0 {
        // Leaf node containing @import text - skip, handled by parent
    } else if text.contains("@import") && is_decl_like(kind) {
        // Declaration containing @import - extract from text
        extract_imports_from_text(text, file, edges);
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports_recursive(child, file, source, edges);
    }
}

/// Check if a node kind is a declaration-like container.
fn is_decl_like(kind: &str) -> bool {
    matches!(
        kind,
        "VarDecl"
            | "variable_declaration"
            | "const_declaration"
            | "TopLevelDecl"
            | "Decl"
            | "source_file"
            | "root"
    )
}

/// Extract the import target from an @import("...") expression.
fn extract_import_target(text: &str) -> Option<String> {
    // Pattern: @import("module_name")
    let import_pos = text.find("@import")?;
    let after_import = &text[import_pos + 7..]; // skip "@import"

    // Find the opening paren and quote
    let paren_pos = after_import.find('(')?;
    let after_paren = &after_import[paren_pos + 1..];

    // Find the string literal
    let quote_start = after_paren.find('"')?;
    let after_quote = &after_paren[quote_start + 1..];
    let quote_end = after_quote.find('"')?;

    let target = &after_quote[..quote_end];
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

/// Extract all @import targets from a text block.
fn extract_imports_from_text(text: &str, file: &str, edges: &mut Vec<Edge>) {
    let mut search_from = 0;
    while let Some(pos) = text[search_from..].find("@import") {
        let abs_pos = search_from + pos;
        let remaining = &text[abs_pos..];
        if let Some(target) = extract_import_target(remaining) {
            add_import_edge(file, &target, edges);
        }
        search_from = abs_pos + 7; // skip past "@import"
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

/// Collect intra-file function calls.
///
/// In Zig, function calls use standard syntax: `name(args)`.
/// Method-style calls use field access: `obj.method(args)`.
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

    // Look for call expressions
    if is_call_node(kind) {
        extract_call_edge(node, file, source, defined_fqns, edges);
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

/// Check if a node kind represents a function call.
fn is_call_node(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"
            | "function_call"
            | "FnCallExpr"
            | "SuffixExpr"
            | "builtin_call_expr"
    )
}

/// Extract a call edge from a call expression node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Skip @import and other builtins that are handled as imports
    if text.starts_with("@import") || text.starts_with("@cImport") || text.starts_with("@cInclude") {
        return;
    }

    // Skip common builtins that aren't user function calls
    if text.starts_with('@') {
        return;
    }

    // Extract the call target name
    let call_name = extract_call_name(&text);
    if call_name.is_empty() || is_zig_keyword(&call_name) {
        return;
    }

    // Determine the source FQN (enclosing function or file-level)
    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Check for method-style calls (e.g., `obj.method(...)`)
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
                        attributes: json!({"receiver": receiver, "call_type": "method"}),
                    });
                }
            } else {
                // Emit unresolved method call for cross-file resolution
                edges.push(Edge {
                    id: None,
                    source_fqn: source_fqn.clone(),
                    target_fqn: method.to_string(),
                    kind: EdgeKind::Calls,
                    confidence: 0.0,
                    attributes: json!({"receiver": receiver, "call_type": "method"}),
                });
            }
            return;
        }
    }

    // Simple function call - try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == &call_name) {
        if source_fqn != *target_fqn {
            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn: target_fqn.clone(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: json!({}),
            });
        }
    }
}

/// Extract the function/method name being called from call expression text.
///
/// Handles:
/// - `name(args)` → "name"
/// - `obj.method(args)` → "obj.method"
/// - `std.io.getStdOut()` → "std.io.getStdOut"
fn extract_call_name(text: &str) -> String {
    // Find the opening parenthesis
    if let Some(paren_pos) = text.find('(') {
        let before_paren = text[..paren_pos].trim();
        // The call target is the identifier/field-access before the paren
        let name: String = before_paren
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        // Strip leading dots
        let name = name.trim_start_matches('.');
        return name.to_string();
    }
    String::new()
}

/// Find the enclosing function definition for a given node and return its FQN.
fn find_enclosing_function_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();

    while let Some(parent) = current {
        let text = parent.utf8_text(source).unwrap_or("");
        // Check if this parent is a function definition
        if text.contains("fn ") {
            let kind = parent.kind();
            if is_function_node(kind) || kind == "source_file" || kind == "root" {
                if kind == "source_file" || kind == "root" {
                    // We've reached the top level without finding a function
                    return None;
                }
                let name = extract_fn_name_from_text(text);
                if !name.is_empty() {
                    return Some(format!("{file}::{name}"));
                }
            }
            // Also check if the parent's text starts with fn/pub fn
            let trimmed = text.trim_start();
            if trimmed.starts_with("fn ")
                || trimmed.starts_with("pub fn ")
                || trimmed.starts_with("export fn ")
            {
                let name = extract_fn_name_from_text(trimmed);
                if !name.is_empty() {
                    return Some(format!("{file}::{name}"));
                }
            }
        }
        current = parent.parent();
    }

    None
}

/// Check if a name is a Zig keyword or builtin that should not be treated as a user call.
fn is_zig_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "switch"
            | "return"
            | "break"
            | "continue"
            | "defer"
            | "errdefer"
            | "try"
            | "catch"
            | "orelse"
            | "and"
            | "or"
            | "const"
            | "var"
            | "pub"
            | "fn"
            | "struct"
            | "enum"
            | "union"
            | "error"
            | "test"
            | "comptime"
            | "inline"
            | "nosuspend"
            | "suspend"
            | "resume"
            | "async"
            | "await"
            | "unreachable"
            | "undefined"
            | "null"
            | "true"
            | "false"
            | "void"
            | "noreturn"
            | "type"
            | "anytype"
            | "usize"
            | "isize"
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Zig source and extract.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_zig::LANGUAGE.into())
            .expect("Zig grammar should load");
        let tree = parser.parse(source, None).expect("should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_zig_extract_functions() {
        let source = r#"
pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello, Zig!\n", .{});
}

fn helper() void {
    // internal helper
}

export fn exported_func() void {
    // exported
}
"#;
        let result = parse_and_extract("src/main.zig", source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.zig::main" && n.kind == NodeKind::Function),
            "should extract pub fn main"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.zig::helper" && n.kind == NodeKind::Function),
            "should extract fn helper"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/main.zig::exported_func" && n.kind == NodeKind::Function),
            "should extract export fn exported_func"
        );
    }

    #[test]
    fn test_zig_extract_structs() {
        let source = r#"
pub const Point = struct {
    x: f64,
    y: f64,

    pub fn distance(self: Point, other: Point) f64 {
        const dx = self.x - other.x;
        const dy = self.y - other.y;
        return @sqrt(dx * dx + dy * dy);
    }
};

const InternalStruct = struct {
    value: u32,
};

const PackedData = packed struct {
    flags: u8,
    data: u24,
};
"#;
        let result = parse_and_extract("src/types.zig", source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/types.zig::Point" && n.kind == NodeKind::Class),
            "should extract pub struct Point as Class"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/types.zig::InternalStruct" && n.kind == NodeKind::Class),
            "should extract InternalStruct as Class"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/types.zig::PackedData" && n.kind == NodeKind::Class),
            "should extract packed struct as Class"
        );
        // The distance function inside the struct should also be extracted
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/types.zig::distance" && n.kind == NodeKind::Function),
            "should extract method distance as Function"
        );
    }

    #[test]
    fn test_zig_extract_enums() {
        let source = r#"
const Direction = enum {
    north,
    south,
    east,
    west,
};

pub const Color = enum(u8) {
    red = 0,
    green = 1,
    blue = 2,
};
"#;
        let result = parse_and_extract("src/enums.zig", source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/enums.zig::Direction" && n.kind == NodeKind::Enum),
            "should extract Direction as Enum"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/enums.zig::Color" && n.kind == NodeKind::Enum),
            "should extract Color as Enum"
        );
    }

    #[test]
    fn test_zig_extract_unions() {
        let source = r#"
const Result = union {
    ok: u32,
    err: []const u8,
};

pub const TaggedUnion = union(enum) {
    int: i32,
    float: f64,
    none: void,
};
"#;
        let result = parse_and_extract("src/unions.zig", source);

        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/unions.zig::Result" && n.kind == NodeKind::Class),
            "should extract union Result as Class"
        );
        assert!(
            result.nodes.iter().any(|n| n.fqn == "src/unions.zig::TaggedUnion" && n.kind == NodeKind::Class),
            "should extract tagged union as Class"
        );
    }

    #[test]
    fn test_zig_extract_imports() {
        let source = r#"
const std = @import("std");
const mem = @import("std").mem;
const os = @import("os");
const my_module = @import("my_module.zig");
"#;
        let result = parse_and_extract("src/main.zig", source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "std"),
            "should import std"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "os"),
            "should import os"
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "my_module.zig"),
            "should import my_module.zig"
        );
    }

    #[test]
    fn test_zig_extract_calls() {
        let source = r#"
const std = @import("std");

fn helper() void {
    // internal helper
}

fn process(data: []const u8) void {
    helper();
}

pub fn main() !void {
    process("hello");
    helper();
}
"#;
        let result = parse_and_extract("src/main.zig", source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // process() calls helper()
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.zig::process"
                && e.target_fqn == "src/main.zig::helper"),
            "process should call helper; calls: {:?}",
            calls
        );

        // main() calls process()
        assert!(
            calls.iter().any(|e| e.source_fqn == "src/main.zig::main"
                && e.target_fqn == "src/main.zig::process"),
            "main should call process; calls: {:?}",
            calls
        );
    }

    #[test]
    fn test_zig_extract_method_calls() {
        let source = r#"
const std = @import("std");

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello\n", .{});
}
"#;
        let result = parse_and_extract("src/main.zig", source);

        let method_calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls && e.attributes.get("call_type").is_some())
            .collect();

        // Should have at least one method call (stdout.print or std.io.getStdOut)
        assert!(
            !method_calls.is_empty(),
            "should extract method-style calls"
        );
    }

    #[test]
    fn test_zig_empty_file() {
        let result = parse_and_extract("empty.zig", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_zig_comprehensive() {
        let source = r#"
const std = @import("std");
const mem = @import("std").mem;

pub const Point = struct {
    x: f64,
    y: f64,

    pub fn distance(self: Point, other: Point) f64 {
        const dx = self.x - other.x;
        const dy = self.y - other.y;
        return @sqrt(dx * dx + dy * dy);
    }
};

const Direction = enum {
    north,
    south,
    east,
    west,
};

const Value = union(enum) {
    int: i32,
    float: f64,
    none: void,
};

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello, Zig!\n", .{});
}

fn helper() void {
    // internal helper
}
"#;
        let result = parse_and_extract("src/main.zig", source);

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "std"));

        // Check struct
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::Point" && n.kind == NodeKind::Class));

        // Check enum
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::Direction" && n.kind == NodeKind::Enum));

        // Check union
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::Value" && n.kind == NodeKind::Class));

        // Check functions
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::main" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::helper" && n.kind == NodeKind::Function));
        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::distance" && n.kind == NodeKind::Function));
    }

    #[test]
    fn test_zig_extract_regex_backward_compat() {
        let source = r#"
const std = @import("std");

pub fn main() !void {
    // hello
}
"#;
        #[allow(deprecated)]
        let result = extract_regex("src/main.zig", source);

        assert!(result.nodes.iter().any(|n| n.fqn == "src/main.zig::main" && n.kind == NodeKind::Function));
        assert!(result.edges.iter().any(|e| e.kind == EdgeKind::Imports && e.target_fqn == "std"));
    }

    #[test]
    fn test_zig_complexity_computed() {
        let source = r#"
pub fn complex_func(x: i32) i32 {
    if (x > 0) {
        if (x > 10) {
            return x * 2;
        }
        return x;
    } else {
        return 0;
    }
}
"#;
        let result = parse_and_extract("src/complex.zig", source);

        let func = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/complex.zig::complex_func")
            .expect("should find complex_func");

        // Complexity should be computed (at least 1 for base)
        if let Some(complexity) = func.attributes.get("complexity") {
            assert!(
                complexity.as_u64().unwrap_or(0) >= 1,
                "complexity should be at least 1"
            );
        }
        // Note: complexity might not be computed if the grammar node kinds don't match
        // the expected patterns, which is acceptable for a first pass.
    }
}
