//! Julia AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (modules, functions, structs, macros, abstract types)
//! and edges (imports, intra-file calls) from a tree-sitter Julia parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Julia file.
///
/// Handles:
/// - Module declarations (`module Name ... end`)
/// - Functions (`function name(args) ... end`)
/// - Short-form functions (`name(args) = expr`)
/// - Structs (`struct Name ... end`, `mutable struct Name ... end`)
/// - Abstract types (`abstract type Name end`)
/// - Macros (`macro name(args) ... end`)
/// - Imports (`using Module`, `import Module`)
/// - Exports (`export symbol1, symbol2`)
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
        .set_language(&tree_sitter_julia::LANGUAGE.into())
        .expect("Julia grammar should load");
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
            // In tree-sitter-julia, function definitions are `function_definition`
            // or `assignment` (short-form). Try both node kinds.
            if let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_definition")
                    .or_else(|| {
                        find_ast_node_at_line(root, node.start_line, "short_function_definition")
                    })
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "assignment"))
                    .or_else(|| find_ast_node_at_line(root, node.start_line, "macro_definition"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "julia");
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
/// In tree-sitter-julia, top-level constructs include:
/// - `module_definition` (module Name ... end)
/// - `function_definition` (function name(args) ... end)
/// - `short_function_definition` (name(args) = expr)
/// - `assignment` (can also be short-form function: f(x) = ...)
/// - `struct_definition` (struct Name ... end)
/// - `abstract_definition` (abstract type Name end)
/// - `macro_definition` (macro name(args) ... end)
/// - `import_statement` / `using_statement` (handled in collect_imports)
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
            "module_definition" => {
                extract_module(child, file, source, nodes, defined_fqns, module_stack);
            }
            "function_definition" => {
                extract_function(child, file, source, nodes, defined_fqns, module_stack);
            }
            "short_function_definition" => {
                extract_short_function(child, file, source, nodes, defined_fqns, module_stack);
            }
            "assignment" => {
                // Check if this is a short-form function definition: f(x) = expr
                maybe_extract_assignment_function(
                    child,
                    file,
                    source,
                    nodes,
                    defined_fqns,
                    module_stack,
                );
            }
            "struct_definition" => {
                extract_struct(child, file, source, nodes, defined_fqns, module_stack);
            }
            "abstract_definition" => {
                extract_abstract_type(child, file, source, nodes, defined_fqns, module_stack);
            }
            "macro_definition" => {
                extract_macro(child, file, source, nodes, defined_fqns, module_stack);
            }
            // Recurse into source_file, module bodies, etc.
            "source_file" | "block" | "let_statement" | "if_statement" | "begin_statement" => {
                collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
            }
            _ => {
                // Recurse into other compound nodes that might contain definitions
                if child.child_count() > 0 && !is_leaf_kind(child.kind()) {
                    collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
                }
            }
        }
    }
}

/// Returns true for node kinds that should not be recursed into for definitions.
fn is_leaf_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "string_literal"
            | "integer_literal"
            | "float_literal"
            | "character_literal"
            | "line_comment"
            | "block_comment"
            | "operator"
            | "number"
            | "boolean_literal"
    )
}

/// Extract a module definition (`module Name ... end`).
fn extract_module(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    let module_name = get_module_name(node, source);
    if module_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &module_name);

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

/// Extract a function definition (`function name(args) ... end`).
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    let func_name = get_function_name(node, source);
    if func_name.is_empty() || is_julia_keyword(&func_name) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &func_name);

    // Julia allows multiple method definitions for the same function name;
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
        attributes: json!({}),
    });

    defined_fqns.push((func_name, fqn));
}

/// Extract a short-form function definition (`name(args) = expr`).
fn extract_short_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    let func_name = get_short_function_name(node, source);
    if func_name.is_empty() || is_julia_keyword(&func_name) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &func_name);

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
        attributes: json!({"short_form": true}),
    });

    defined_fqns.push((func_name, fqn));
}

/// Check if an assignment node is a short-form function definition.
/// Pattern: `name(args) = expr` appears as an assignment in some grammar versions.
fn maybe_extract_assignment_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    // In tree-sitter-julia, a short-form function `f(x) = x + 1` may be parsed as
    // an assignment where the LHS is a call_expression.
    let text = node.utf8_text(source).unwrap_or("").trim();

    // Check if LHS looks like a function call (has parentheses)
    // The first child should be a call_expression or typed_expression with call
    if let Some(lhs) = node.child(0) {
        let lhs_kind = lhs.kind();
        if lhs_kind == "call_expression" || lhs_kind == "typed_expression" {
            let func_name = get_call_expression_name(lhs, source);
            if !func_name.is_empty() && !is_julia_keyword(&func_name) {
                let start_line = node.start_position().row as u32 + 1;
                let end_line = node.end_position().row as u32 + 1;

                let fqn = build_fqn(file, module_stack, &func_name);

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
                    attributes: json!({"short_form": true}),
                });

                defined_fqns.push((func_name, fqn));
            }
        }
    } else {
        // Fallback: check text pattern
        if let Some(paren_pos) = text.find('(')
            && let Some(eq_pos) = text.find('=')
            && paren_pos < eq_pos
        {
            let name: String = text[..paren_pos]
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !is_julia_keyword(&name) {
                let start_line = node.start_position().row as u32 + 1;
                let end_line = node.end_position().row as u32 + 1;
                let fqn = build_fqn(file, module_stack, &name);

                if !nodes.iter().any(|n| n.fqn == fqn) {
                    nodes.push(Node {
                        fqn: fqn.clone(),
                        kind: NodeKind::Function,
                        file: file.to_string(),
                        start_line,
                        end_line,
                        file_hash: String::new(),
                        indexed_at: 0,
                        attributes: json!({"short_form": true}),
                    });
                    defined_fqns.push((name, fqn));
                }
            }
        }
    }
}

/// Extract a struct definition (`struct Name ... end` or `mutable struct Name ... end`).
fn extract_struct(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    let struct_name = get_struct_name(node, source);
    if struct_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &struct_name);

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    // Determine if mutable
    let text = node.utf8_text(source).unwrap_or("");
    let is_mutable = text.trim_start().starts_with("mutable");

    let mut attributes = json!({});
    if is_mutable && let Some(attrs) = attributes.as_object_mut() {
        attrs.insert("mutable".to_string(), json!(true));
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Class,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes,
    });

    defined_fqns.push((struct_name, fqn));
}

/// Extract an abstract type definition (`abstract type Name end`).
fn extract_abstract_type(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    let type_name = get_abstract_type_name(node, source);
    if type_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = build_fqn(file, module_stack, &type_name);

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Type,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"abstract": true}),
    });

    defined_fqns.push((type_name, fqn));
}

/// Extract a macro definition (`macro name(args) ... end`).
fn extract_macro(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &[String],
) {
    let macro_name = get_macro_name(node, source);
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
        attributes: json!({"macro": true}),
    });

    defined_fqns.push((macro_name, fqn));
}

/// Collect definitions from the body of a module.
fn collect_definitions_in_body(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
    module_stack: &mut Vec<String>,
) {
    // In tree-sitter-julia, module body items are direct children of module_definition.
    // We need to handle each child that could be a definition.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "module_definition" => {
                extract_module(child, file, source, nodes, defined_fqns, module_stack);
            }
            "function_definition" => {
                extract_function(child, file, source, nodes, defined_fqns, module_stack);
            }
            "short_function_definition" => {
                extract_short_function(child, file, source, nodes, defined_fqns, module_stack);
            }
            "assignment" => {
                maybe_extract_assignment_function(
                    child,
                    file,
                    source,
                    nodes,
                    defined_fqns,
                    module_stack,
                );
            }
            "struct_definition" => {
                extract_struct(child, file, source, nodes, defined_fqns, module_stack);
            }
            "abstract_definition" => {
                extract_abstract_type(child, file, source, nodes, defined_fqns, module_stack);
            }
            "macro_definition" => {
                extract_macro(child, file, source, nodes, defined_fqns, module_stack);
            }
            _ => {
                // Recurse into compound nodes that might contain definitions
                if child.child_count() > 0 && !is_leaf_kind(child.kind()) {
                    collect_definitions(child, file, source, nodes, defined_fqns, module_stack);
                }
            }
        }
    }
}

/// Collect import/using edges from the AST.
///
/// In Julia, imports are:
/// - `using Module` (brings exported names into scope)
/// - `using Module: name1, name2` (selective import)
/// - `import Module` (brings module into scope)
/// - `import Module: name1, name2` (selective import)
/// - `export name1, name2` (marks symbols for export)
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" | "using_statement" => {
                extract_import_edges(child, file, source, edges);
            }
            "export_statement" => {
                // Export statements don't create edges but we note them
                // They could be used for re-export resolution later
            }
            _ => {
                // Recurse into children to find nested imports
                if child.child_count() > 0 {
                    collect_imports(child, file, source, edges);
                }
            }
        }
    }
}

/// Extract import edges from a using/import statement node.
fn extract_import_edges(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let text = node.utf8_text(source).unwrap_or("").trim();

    // Determine import type from the keyword
    let import_type = if text.starts_with("using") {
        "using"
    } else {
        "import"
    };

    // Try AST-based extraction first
    let mut found_targets = false;
    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        match child.kind() {
            // Simple import: `using LinearAlgebra` or `import Statistics`
            "identifier" => {
                let target = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !target.is_empty() && target != "using" && target != "import" {
                    add_import_edge(file, &target, import_type, edges);
                    found_targets = true;
                }
            }
            // Selective import: `using Base: @show, println`
            // The selected_import node contains the module identifier as first child
            "selected_import" => {
                // First identifier child is the module name
                let mut sel_cursor = child.walk();
                for sel_child in child.children(&mut sel_cursor) {
                    if sel_child.kind() == "identifier" {
                        let target = sel_child.utf8_text(source).unwrap_or("").trim().to_string();
                        if !target.is_empty() {
                            add_import_edge(file, &target, import_type, edges);
                            found_targets = true;
                        }
                        break; // Only the first identifier is the module name
                    }
                }
            }
            // Scoped/qualified import: `import Module.SubModule`
            "scoped_identifier" | "field_expression" => {
                let target = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !target.is_empty() {
                    add_import_edge(file, &target, import_type, edges);
                    found_targets = true;
                }
            }
            // Handle comma-separated imports
            "import_list" | "argument_list" => {
                let mut list_cursor = child.walk();
                for item in child.children(&mut list_cursor) {
                    if matches!(
                        item.kind(),
                        "identifier" | "scoped_identifier" | "field_expression" | "selected_import"
                    ) {
                        if item.kind() == "selected_import" {
                            // Extract module name from selected_import
                            let mut si_cursor = item.walk();
                            for si_child in item.children(&mut si_cursor) {
                                if si_child.kind() == "identifier" {
                                    let target =
                                        si_child.utf8_text(source).unwrap_or("").trim().to_string();
                                    if !target.is_empty() {
                                        add_import_edge(file, &target, import_type, edges);
                                        found_targets = true;
                                    }
                                    break;
                                }
                            }
                        } else {
                            let target = item.utf8_text(source).unwrap_or("").trim().to_string();
                            if !target.is_empty() {
                                add_import_edge(file, &target, import_type, edges);
                                found_targets = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback: parse from text if AST extraction didn't find targets
    if !found_targets {
        let after_keyword = if let Some(rest) = text.strip_prefix("using") {
            rest.trim()
        } else if let Some(rest) = text.strip_prefix("import") {
            rest.trim()
        } else {
            ""
        };

        if !after_keyword.is_empty() {
            // Handle comma-separated modules: `using Mod1, Mod2`
            // Handle colon syntax: `using Mod: name1, name2` -> import Mod
            for part in after_keyword.split(',') {
                let part = part.trim();
                // Take the module part (before any colon for selective imports)
                let module_part = if let Some(colon_pos) = part.find(':') {
                    part[..colon_pos].trim()
                } else {
                    part
                };
                // Extract the module name (alphanumeric, _, .)
                let target: String = module_part
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                    .collect();
                if !target.is_empty() {
                    add_import_edge(file, &target, import_type, edges);
                }
            }
        }
    }
}

/// Add an import edge, avoiding duplicates.
fn add_import_edge(file: &str, target: &str, import_type: &str, edges: &mut Vec<Edge>) {
    if !edges
        .iter()
        .any(|e| e.kind == EdgeKind::Imports && e.source_fqn == file && e.target_fqn == target)
    {
        edges.push(Edge {
            id: None,
            source_fqn: file.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({"import_type": import_type}),
        });
    }
}

/// Collect intra-file function calls.
///
/// In Julia, function calls use standard syntax: `name(args)`.
/// In tree-sitter-julia, these are `call_expression` nodes.
/// Also handles dot-call syntax: `Module.function(args)`.
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
    match node.kind() {
        "call_expression" => {
            extract_call_edge(node, file, source, defined_fqns, edges);
        }
        // Also handle macro calls: @macro_name(args)
        "macro_expression" | "macrocall_expression" => {
            extract_macro_call_edge(node, file, source, defined_fqns, edges);
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

/// Extract a call edge from a call_expression node.
fn extract_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let call_name = get_call_expression_name(node, source);
    if call_name.is_empty() || is_julia_keyword(&call_name) {
        return;
    }

    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Check for qualified call (Module.function)
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
                        attributes: json!({"receiver": receiver, "call_type": "qualified"}),
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
                    attributes: json!({"receiver": receiver, "call_type": "qualified"}),
                });
            }
            return;
        }
    }

    // Simple function call - try to resolve to a defined function
    if let Some((_, target_fqn)) = defined_fqns.iter().find(|(simple, _)| simple == &call_name)
        && source_fqn != *target_fqn
    {
        // Avoid duplicate edges
        if !edges.iter().any(|e| {
            e.kind == EdgeKind::Calls && e.source_fqn == source_fqn && e.target_fqn == *target_fqn
        }) {
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

/// Extract a macro call edge from a macro_expression node.
fn extract_macro_call_edge(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim();

    // Macro calls start with @: @macro_name(args) or @macro_name args
    let macro_name = if let Some(after_at) = text.strip_prefix('@') {
        let name: String = after_at
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        name
    } else {
        // Try AST-based extraction
        let mut cursor = node.walk();
        let mut name = String::new();
        for child in node.children(&mut cursor) {
            if child.kind() == "macro_identifier" || child.kind() == "identifier" {
                let t = child.utf8_text(source).unwrap_or("").trim();
                let t = t.strip_prefix('@').unwrap_or(t);
                name = t.to_string();
                break;
            }
        }
        name
    };

    if macro_name.is_empty() || is_julia_keyword(&macro_name) {
        return;
    }

    let source_fqn =
        find_enclosing_function_fqn(node, file, source).unwrap_or_else(|| file.to_string());

    // Try to resolve to a defined macro
    if let Some((_, target_fqn)) = defined_fqns
        .iter()
        .find(|(simple, _)| simple == &macro_name)
        && source_fqn != *target_fqn
    {
        edges.push(Edge {
            id: None,
            source_fqn,
            target_fqn: target_fqn.clone(),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            attributes: json!({"call_type": "macro"}),
        });
    }
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Get the module name from a module_definition node.
///
/// In tree-sitter-julia, a module_definition typically has structure:
/// (module_definition "module" name: (identifier) body ... "end")
fn get_module_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }

    // Fallback: look for the first identifier child after "module" keyword
    let mut cursor = node.walk();
    let mut found_module_keyword = false;
    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if text == "module" || text == "baremodule" {
            found_module_keyword = true;
            continue;
        }

        if found_module_keyword && child.kind() == "identifier" {
            return text;
        }

        // Also accept if the child is an identifier and comes after position 0
        if found_module_keyword
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return text;
        }
    }

    // Last resort: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after) = text
        .strip_prefix("module")
        .or_else(|| text.strip_prefix("baremodule"))
    {
        let name: String = after
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return name;
        }
    }

    String::new()
}

/// Get the function name from a function_definition node.
///
/// In tree-sitter-julia, a function_definition has structure:
/// (function_definition "function" (signature (call_expression (identifier) (argument_list))) ... "end")
fn get_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(name_node) = node.child_by_field_name("name") {
        match name_node.kind() {
            "identifier" => {
                return name_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "call_expression" | "typed_expression" => {
                return get_call_expression_name(name_node, source);
            }
            "field_expression" => {
                let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
                return text.rsplit('.').next().unwrap_or("").to_string();
            }
            _ => {
                let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return name;
                }
            }
        }
    }

    // Look for signature node (tree-sitter-julia uses this)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "signature" {
            // signature contains a call_expression with the function name
            let mut sig_cursor = child.walk();
            for sig_child in child.children(&mut sig_cursor) {
                if sig_child.kind() == "call_expression" {
                    return get_call_expression_name(sig_child, source);
                }
                if sig_child.kind() == "identifier" {
                    return sig_child.utf8_text(source).unwrap_or("").trim().to_string();
                }
                if sig_child.kind() == "typed_expression" {
                    // function name(args)::ReturnType
                    let mut te_cursor = sig_child.walk();
                    for te_child in sig_child.children(&mut te_cursor) {
                        if te_child.kind() == "call_expression" {
                            return get_call_expression_name(te_child, source);
                        }
                    }
                }
            }
            // Fallback: extract name from signature text
            let sig_text = child.utf8_text(source).unwrap_or("").trim();
            let name: String = sig_text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !is_julia_keyword(&name) {
                return name;
            }
        }
    }

    // Fallback: look through children for identifier after "function" keyword
    let mut cursor2 = node.walk();
    let mut found_function_keyword = false;
    for child in node.children(&mut cursor2) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if text == "function" {
            found_function_keyword = true;
            continue;
        }

        if found_function_keyword {
            match child.kind() {
                "identifier" => return text,
                "call_expression" => return get_call_expression_name(child, source),
                "field_expression" => {
                    return text.rsplit('.').next().unwrap_or("").to_string();
                }
                _ => {
                    let name: String = text
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() && !is_julia_keyword(&name) {
                        return name;
                    }
                }
            }
        }
    }

    // Last resort: parse from text
    let full_text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after) = full_text.strip_prefix("function") {
        let after = after.trim();
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !is_julia_keyword(&name) {
            return name;
        }
    }

    String::new()
}

/// Get the function name from a short_function_definition node.
///
/// Pattern: `name(args) = expr` or `name(args)::Type = expr`
fn get_short_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(name_node) = node.child_by_field_name("name") {
        match name_node.kind() {
            "identifier" => {
                return name_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "call_expression" => {
                return get_call_expression_name(name_node, source);
            }
            _ => {
                let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return name;
                }
            }
        }
    }

    // Fallback: first child is typically the LHS (call expression)
    if let Some(first_child) = node.child(0) {
        match first_child.kind() {
            "call_expression" => return get_call_expression_name(first_child, source),
            "typed_expression" => {
                // name(args)::Type = expr
                if let Some(inner) = first_child.child(0)
                    && inner.kind() == "call_expression"
                {
                    return get_call_expression_name(inner, source);
                }
            }
            "identifier" => {
                return first_child
                    .utf8_text(source)
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            _ => {}
        }
    }

    // Last resort: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let name: String = text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if !name.is_empty() && !is_julia_keyword(&name) {
        return name;
    }

    String::new()
}

/// Get the function name from a call_expression node (the callee).
///
/// A call_expression in tree-sitter-julia has structure:
/// (call_expression function: (identifier) arguments: (argument_list ...))
fn get_call_expression_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(func_node) = node.child_by_field_name("function") {
        match func_node.kind() {
            "identifier" => {
                return func_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "field_expression" => {
                // Module.function(args) - return the full qualified name
                return func_node.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {
                let text = func_node.utf8_text(source).unwrap_or("").trim().to_string();
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                    .collect();
                if !name.is_empty() {
                    return name;
                }
            }
        }
    }

    // Fallback: first child is typically the function being called
    if let Some(first_child) = node.child(0) {
        match first_child.kind() {
            "identifier" => {
                return first_child
                    .utf8_text(source)
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            "field_expression" => {
                return first_child
                    .utf8_text(source)
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            _ => {
                let text = first_child
                    .utf8_text(source)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let name: String = text
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                    .collect();
                if !name.is_empty() {
                    return name;
                }
            }
        }
    }

    // Last resort: parse from full text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let name: String = text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    name
}

/// Get the struct name from a struct_definition node.
///
/// Patterns:
/// - `struct Name ... end`
/// - `mutable struct Name ... end`
/// - `struct Name <: SuperType ... end`
fn get_struct_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            // Handle parametric types: Name{T}
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    // Fallback: look for identifier after struct keyword
    let mut cursor = node.walk();
    let mut found_struct_keyword = false;
    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if text == "struct" || text == "mutable" {
            if text == "struct" {
                found_struct_keyword = true;
            }
            continue;
        }

        if found_struct_keyword && child.kind() == "identifier" {
            return text;
        }

        if found_struct_keyword
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
        {
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    // Last resort: parse from text
    let full_text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let after_struct = if let Some(rest) = full_text.strip_prefix("mutable") {
        rest.trim().strip_prefix("struct").unwrap_or("").trim()
    } else if let Some(rest) = full_text.strip_prefix("struct") {
        rest.trim()
    } else {
        ""
    };

    let name: String = after_struct
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    name
}

/// Get the abstract type name from an abstract_definition node.
///
/// Pattern: `abstract type Name end` or `abstract type Name <: SuperType end`
/// AST: (abstract_definition "abstract" "type" (type_head (identifier)) "end")
fn get_abstract_type_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    // Look for type_head node which contains the identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_head" {
            // type_head contains the identifier (and possibly type parameters)
            let mut th_cursor = child.walk();
            for th_child in child.children(&mut th_cursor) {
                if th_child.kind() == "identifier" {
                    return th_child.utf8_text(source).unwrap_or("").trim().to_string();
                }
            }
            // Fallback: use the type_head text directly
            let text = child.utf8_text(source).unwrap_or("").trim().to_string();
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return name;
            }
        }
    }

    // Fallback: look for identifier after "type" keyword
    let mut found_type_keyword = false;
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if text == "abstract" {
            continue;
        }
        if text == "type" {
            found_type_keyword = true;
            continue;
        }

        if found_type_keyword && child.kind() == "identifier" {
            return text;
        }

        if found_type_keyword
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
        {
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    // Last resort: parse from text
    let full_text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after) = full_text.strip_prefix("abstract") {
        let after = after.trim();
        if let Some(after_type) = after.strip_prefix("type") {
            let name: String = after_type
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    String::new()
}

/// Get the macro name from a macro_definition node.
///
/// Pattern: `macro name(args) ... end`
fn get_macro_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try field-based access
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() {
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }
    }

    // Fallback: look for identifier after "macro" keyword
    let mut cursor = node.walk();
    let mut found_macro_keyword = false;
    for child in node.children(&mut cursor) {
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        if text == "macro" {
            found_macro_keyword = true;
            continue;
        }

        if found_macro_keyword {
            match child.kind() {
                "identifier" => return text,
                "call_expression" => return get_call_expression_name(child, source),
                _ => {
                    let name: String = text
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() && !is_julia_keyword(&name) {
                        return name;
                    }
                }
            }
        }
    }

    // Last resort: parse from text
    let full_text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after) = full_text.strip_prefix("macro") {
        let name: String = after
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return name;
        }
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
        match parent.kind() {
            "function_definition" => {
                let name = get_function_name(parent, source);
                if !name.is_empty() && !is_julia_keyword(&name) {
                    return Some(format!("{file}::{name}"));
                }
            }
            "short_function_definition" => {
                let name = get_short_function_name(parent, source);
                if !name.is_empty() && !is_julia_keyword(&name) {
                    return Some(format!("{file}::{name}"));
                }
            }
            "macro_definition" => {
                let name = get_macro_name(parent, source);
                if !name.is_empty() {
                    return Some(format!("{file}::{name}"));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    None
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

/// Check if a name is a Julia keyword that should not be treated as a function name.
fn is_julia_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "elseif"
            | "for"
            | "while"
            | "begin"
            | "end"
            | "function"
            | "macro"
            | "module"
            | "baremodule"
            | "struct"
            | "mutable"
            | "abstract"
            | "type"
            | "let"
            | "const"
            | "global"
            | "local"
            | "return"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "using"
            | "import"
            | "export"
            | "do"
            | "in"
            | "isa"
            | "where"
            | "true"
            | "false"
            | "nothing"
            | "missing"
            | "break"
            | "continue"
            | "quote"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Julia source and extract.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_julia::LANGUAGE.into())
            .expect("Julia grammar should load");
        let tree = parser.parse(source, None).expect("should parse");
        extract(&tree, file, source)
    }

    #[test]
    fn test_julia_extract_module() {
        let source = r#"
module MyPackage

end
"#;
        let result = parse_and_extract("src/MyPackage.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/MyPackage.jl::MyPackage" && n.kind == NodeKind::Module)
        );
    }

    #[test]
    fn test_julia_extract_functions() {
        let source = r#"
function area(c::Circle)
    return π * c.radius^2
end

function main()
    c = Circle(5.0)
    println("Area: $(area(c))")
end
"#;
        let result = parse_and_extract("src/shapes.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/shapes.jl::area" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/shapes.jl::main" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_julia_extract_short_function() {
        let source = r#"
distance(p1::Point, p2::Point) = sqrt((p1.x - p2.x)^2 + (p1.y - p2.y)^2)
"#;
        let result = parse_and_extract("src/utils.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/utils.jl::distance" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_julia_extract_structs() {
        let source = r#"
struct Circle
    radius::Float64
end

mutable struct Point
    x::Float64
    y::Float64
end
"#;
        let result = parse_and_extract("src/types.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/types.jl::Circle" && n.kind == NodeKind::Class)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/types.jl::Point" && n.kind == NodeKind::Class)
        );

        // Check mutable attribute
        let point = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/types.jl::Point")
            .unwrap();
        assert_eq!(point.attributes["mutable"], true);
    }

    #[test]
    fn test_julia_extract_abstract_type() {
        let source = r#"
abstract type Shape end
abstract type Animal <: LivingThing end
"#;
        let result = parse_and_extract("src/types.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/types.jl::Shape" && n.kind == NodeKind::Type)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/types.jl::Animal" && n.kind == NodeKind::Type)
        );
    }

    #[test]
    fn test_julia_extract_macro() {
        let source = r#"
macro assert_equal(a, b)
    return :($a == $b || error("Not equal"))
end
"#;
        let result = parse_and_extract("src/macros.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/macros.jl::assert_equal" && n.kind == NodeKind::Function)
        );

        // Check macro attribute
        let macro_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/macros.jl::assert_equal")
            .unwrap();
        assert_eq!(macro_node.attributes["macro"], true);
    }

    #[test]
    fn test_julia_extract_imports() {
        let source = r#"
using LinearAlgebra
import Statistics
using Base: @show, println
"#;
        let result = parse_and_extract("src/main.jl", source);
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "LinearAlgebra"));
        assert!(imports.iter().any(|e| e.target_fqn == "Statistics"));
        assert!(imports.iter().any(|e| e.target_fqn == "Base"));
    }

    #[test]
    fn test_julia_extract_calls() {
        let source = r#"
function helper()
    return 42
end

function main()
    x = helper()
    println(x)
end
"#;
        let result = parse_and_extract("src/main.jl", source);
        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        // main should call helper
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "src/main.jl::main"
                    && e.target_fqn == "src/main.jl::helper")
        );
    }

    #[test]
    fn test_julia_full_package() {
        let source = r#"
module MyPackage

using LinearAlgebra
import Statistics

abstract type Shape end

struct Circle <: Shape
    radius::Float64
end

mutable struct Point
    x::Float64
    y::Float64
end

function area(c::Circle)
    return π * c.radius^2
end

distance(p1::Point, p2::Point) = sqrt((p1.x - p2.x)^2 + (p1.y - p2.y)^2)

macro debug(expr)
    return :(println("DEBUG: ", $(string(expr)), " = ", $expr))
end

function main()
    c = Circle(5.0)
    println("Area: $(area(c))")
end

export area, distance

end # module
"#;
        let result = parse_and_extract("src/MyPackage.jl", source);

        // Check module
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/MyPackage.jl::MyPackage" && n.kind == NodeKind::Module)
        );

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "LinearAlgebra"));
        assert!(imports.iter().any(|e| e.target_fqn == "Statistics"));

        // Check abstract type
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("Shape") && n.kind == NodeKind::Type)
        );

        // Check structs
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("Circle") && n.kind == NodeKind::Class)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("Point") && n.kind == NodeKind::Class)
        );

        // Check functions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("area") && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("distance") && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("main") && n.kind == NodeKind::Function)
        );

        // Check macro
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn.contains("debug") && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn test_julia_empty_file() {
        let result = parse_and_extract("empty.jl", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_julia_nested_module() {
        let source = r#"
module Outer

module Inner
    function foo()
        return 1
    end
end

end
"#;
        let result = parse_and_extract("src/nested.jl", source);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/nested.jl::Outer" && n.kind == NodeKind::Module)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/nested.jl::Outer::Inner" && n.kind == NodeKind::Module)
        );
        assert!(
            result.nodes.iter().any(
                |n| n.fqn == "src/nested.jl::Outer::Inner::foo" && n.kind == NodeKind::Function
            )
        );
    }

    #[test]
    fn test_julia_complexity_computed() {
        let source = r#"
function complex_func(x)
    if x > 0
        if x > 10
            return "big"
        else
            return "small positive"
        end
    elseif x == 0
        return "zero"
    else
        return "negative"
    end
end
"#;
        let result = parse_and_extract("src/complex.jl", source);
        let func = result
            .nodes
            .iter()
            .find(|n| n.fqn == "src/complex.jl::complex_func")
            .expect("should find complex_func");
        // Complexity should be computed (value > 0)
        assert!(func.attributes.get("complexity").is_some());
    }
}
