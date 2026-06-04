//! Haskell AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (modules, functions, typeclasses, data types, type aliases)
//! and edges (imports, intra-file calls) from a tree-sitter Haskell parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Haskell file.
///
/// Handles:
/// - Module declarations (`module Name where`)
/// - Top-level function definitions (with type signatures)
/// - Type classes (`class Name where`) → NodeKind::Trait
/// - Data types (`data Name = ...`) → NodeKind::Class
/// - Newtypes (`newtype Name = ...`) → NodeKind::Class
/// - Type aliases (`type Name = ...`) → NodeKind::TypeAlias
/// - Imports (`import Module`, `import qualified Module as Alias`)
/// - Intra-file function calls
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
        .set_language(&tree_sitter_haskell::LANGUAGE.into())
        .expect("Haskell grammar should load");
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
            // In tree-sitter-haskell, function definitions are typically
            // `function` nodes or `bind` nodes at the top level.
            if let Some(ast_node) = find_ast_node_at_line(root, node.start_line, "function")
                .or_else(|| find_ast_node_at_line(root, node.start_line, "bind"))
                .or_else(|| find_ast_node_at_line(root, node.start_line, "signature"))
            {
                let c = complexity::compute_full_complexity(ast_node, source, "haskell");
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
/// In tree-sitter-haskell, top-level declarations include:
/// - `function` (function binding with equations)
/// - `signature` (type signature: `name :: Type`)
/// - `adt` (algebraic data type: `data Name = ...`)
/// - `newtype` (newtype declaration: `newtype Name = ...`)
/// - `type_alias` (type synonym: `type Name = ...`)
/// - `class` (type class declaration: `class Name where ...`)
/// - `instance` (instance declaration)
/// - `import` (import statement)
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
            // Module declaration: module Name where
            "header" => {
                extract_module(child, file, source, nodes);
            }
            // Type signature: name :: Type -> Type
            "signature" => {
                extract_signature(child, file, source, nodes, defined_fqns);
            }
            // Function binding (equations): name args = body
            "function" => {
                extract_function(child, file, source, nodes, defined_fqns);
            }
            // Bind (simple pattern binding): name = expr
            "bind" => {
                extract_bind(child, file, source, nodes, defined_fqns);
            }
            // Algebraic data type: data Name = Constructor | ...
            // tree-sitter-haskell uses "data_type" for data declarations
            "data_type" | "adt" => {
                extract_data_type(child, file, source, nodes, defined_fqns);
            }
            // Newtype: newtype Name = Constructor Type
            "newtype" => {
                extract_newtype(child, file, source, nodes, defined_fqns);
            }
            // Type alias: type Name = ExistingType
            // tree-sitter-haskell uses "type_synomym" (note: typo in grammar)
            "type_synomym" | "type_synonym" | "type_alias" => {
                extract_type_alias(child, file, source, nodes, defined_fqns);
            }
            // Type class: class (Context =>) Name a where ...
            "class" => {
                extract_typeclass(child, file, source, nodes, defined_fqns);
            }
            // Declarations node wraps top-level items in tree-sitter-haskell
            "declarations" | "top_splice" | "decls" => {
                collect_definitions(child, file, source, nodes, defined_fqns);
            }
            _ => {
                // Recurse into other node types that might contain definitions
                if child.child_count() > 0 && child.kind() != "import" {
                    collect_definitions(child, file, source, nodes, defined_fqns);
                }
            }
        }
    }
}

/// Extract a module declaration from a `header` node.
/// Pattern: module Name where
fn extract_module(node: tree_sitter::Node, file: &str, source: &[u8], nodes: &mut Vec<Node>) {
    let module_name = get_module_name(node, source);
    if module_name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{module_name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn,
            kind: NodeKind::Module,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({}),
        });
    }
}

/// Extract a function from a type signature node.
/// Pattern: name :: Type -> Type
///
/// We create the Function node from the signature so we capture the type info.
/// If a `function` node is found later with the same name, we extend the end_line.
fn extract_signature(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_signature_name(node, source);
    if name.is_empty() || is_operator_name(&name) {
        return;
    }

    // Skip common keywords that might be misidentified
    if matches!(
        name.as_str(),
        "module" | "import" | "type" | "data" | "newtype" | "class" | "instance" | "where"
    ) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    // Get the type annotation text
    let type_text = get_signature_type(node, source);

    if let Some(existing) = nodes.iter_mut().find(|n| n.fqn == fqn) {
        // Update start_line to the signature line (earlier)
        if start_line < existing.start_line {
            existing.start_line = start_line;
        }
        // Add type annotation to attributes
        if !type_text.is_empty()
            && let Some(attrs) = existing.attributes.as_object_mut()
        {
            attrs.insert("type_signature".to_string(), json!(type_text));
        }
    } else {
        let mut attributes = json!({});
        if !type_text.is_empty()
            && let Some(attrs) = attributes.as_object_mut()
        {
            attrs.insert("type_signature".to_string(), json!(type_text));
        }

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Function,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes,
        });

        defined_fqns.push((name, fqn));
    }
}

/// Extract a function from a `function` node (equation-style binding).
/// Pattern: name arg1 arg2 = body
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_function_name(node, source);
    if name.is_empty() || is_operator_name(&name) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    // Haskell allows multiple equations for the same function (pattern matching);
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

    defined_fqns.push((name, fqn));
}

/// Extract a simple binding (pattern binding without arguments).
/// Pattern: name = expr
fn extract_bind(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_bind_name(node, source);
    if name.is_empty() || is_operator_name(&name) {
        return;
    }

    // Skip if it looks like a keyword
    if matches!(
        name.as_str(),
        "module" | "import" | "type" | "data" | "newtype" | "class" | "instance" | "where"
    ) {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    // Don't duplicate if already defined via signature
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

    defined_fqns.push((name, fqn));
}

/// Extract a data type declaration.
/// Pattern: data Name a b = Constructor1 | Constructor2
fn extract_data_type(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_decl_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"data_kind": "data"}),
        });
        defined_fqns.push((name, fqn));
    }
}

/// Extract a newtype declaration.
/// Pattern: newtype Name = Constructor Type
fn extract_newtype(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_decl_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Class,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"data_kind": "newtype"}),
        });
        defined_fqns.push((name, fqn));
    }
}

/// Extract a type alias declaration.
/// Pattern: type Name = ExistingType
fn extract_type_alias(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_type_decl_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::TypeAlias,
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

/// Extract a type class declaration.
/// Pattern: class (Context =>) Name a where ...
fn extract_typeclass(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let name = get_class_name(node, source);
    if name.is_empty() {
        return;
    }

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    let fqn = format!("{file}::{name}");

    if !nodes.iter().any(|n| n.fqn == fqn) {
        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Trait,
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

/// Collect import edges from the AST.
///
/// In tree-sitter-haskell, import statements are `import` nodes with structure:
/// - `import Module`
/// - `import qualified Module`
/// - `import Module as Alias`
/// - `import Module (specific, items)`
/// - `import Module hiding (items)`
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import"
            && let Some(target) = get_import_module(child, source)
        {
            // Check for qualified import
            let is_qualified = is_qualified_import(child, source);
            let alias = get_import_alias(child, source);

            // Avoid duplicate imports
            if !edges.iter().any(|e| {
                e.kind == EdgeKind::Imports && e.source_fqn == file && e.target_fqn == target
            }) {
                let mut attrs = json!({});
                if is_qualified && let Some(a) = attrs.as_object_mut() {
                    a.insert("qualified".to_string(), json!(true));
                }
                if let Some(ref al) = alias
                    && let Some(a) = attrs.as_object_mut()
                {
                    a.insert("alias".to_string(), json!(al));
                }

                edges.push(Edge {
                    id: None,
                    source_fqn: file.to_string(),
                    target_fqn: target,
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    edge_source: crate::store::confidence::EdgeSource::AstDirect,
                    attributes: attrs,
                });
            }
        }

        // Recurse into children (imports may be inside declarations blocks)
        if child.child_count() > 0 && child.kind() != "import" {
            collect_imports(child, file, source, edges);
        }
    }
}

/// Collect intra-file function calls.
///
/// In Haskell, function application is juxtaposition: `f x` applies f to x.
/// In tree-sitter-haskell, this is represented as `apply` or `exp_apply` nodes.
/// We also look for `variable` nodes used in application position.
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
        // Function application: f x y
        "apply" | "exp_apply" | "function_application" => {
            // The first child is the function being called
            if let Some(func_node) = node.child(0) {
                let call_name = get_applied_function_name(func_node, source);
                if !call_name.is_empty() && !is_haskell_keyword(&call_name) {
                    let source_fqn = find_enclosing_function_fqn(node, file, source)
                        .unwrap_or_else(|| file.to_string());

                    // Check for qualified call (Module.function)
                    if call_name.contains('.') {
                        let parts: Vec<&str> = call_name.rsplitn(2, '.').collect();
                        if parts.len() == 2 {
                            let method = parts[0];
                            let receiver = parts[1];

                            if let Some((_, target_fqn)) =
                                defined_fqns.iter().find(|(simple, _)| simple == method)
                            {
                                if source_fqn != *target_fqn {
                                    edges.push(Edge {
                                        id: None,
                                        source_fqn: source_fqn.clone(),
                                        target_fqn: target_fqn.clone(),
                                        kind: EdgeKind::Calls,
                                        confidence: 1.0,
                                        edge_source: crate::store::confidence::EdgeSource::AstDirect,
                                        attributes: json!({"receiver": receiver, "call_type": "qualified"}),
                                    });
                                }
                            } else {
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
                            // Don't recurse into this node's children for calls
                            // (we already handled the application)
                            return;
                        }
                    }

                    // Simple function call
                    if let Some((_, target_fqn)) =
                        defined_fqns.iter().find(|(simple, _)| simple == &call_name)
                        && source_fqn != *target_fqn
                    {
                        // Avoid duplicate edges
                        if !edges.iter().any(|e| {
                            e.kind == EdgeKind::Calls
                                && e.source_fqn == source_fqn
                                && e.target_fqn == *target_fqn
                        }) {
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
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(child, file, source, defined_fqns, edges);
    }
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Get the module name from a `header` node.
/// The header typically contains: `module` keyword, module name, optional exports, `where`.
fn get_module_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Look for a `module` child or a qualified module name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "module" | "module_id" | "qualified_module" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "module" {
                    return text;
                }
            }
            _ => {
                // Try to find a capitalized name (module names start with uppercase)
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && text.chars().next().is_some_and(|c| c.is_uppercase())
                    && text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                    && text != "where"
                {
                    return text;
                }
            }
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(after_module) = text.strip_prefix("module") {
        let name: String = after_module
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if !name.is_empty() {
            return name;
        }
    }

    String::new()
}

/// Get the function name from a type signature node.
/// Pattern: name :: Type
fn get_signature_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // The first child of a signature is typically the name (variable/identifier)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable" | "name" | "identifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != "::" {
                    return text;
                }
            }
            // Some grammar versions use a `name` field
            _ => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                // First non-empty, non-operator token that starts with lowercase
                if !text.is_empty()
                    && text != "::"
                    && text
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_lowercase() || c == '_')
                    && text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
                {
                    return text;
                }
            }
        }
        // Only check the first meaningful child
        if child.kind() != "comment" {
            break;
        }
    }

    // Fallback: extract from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(name_end) = text.find("::") {
        let name = text[..name_end].trim();
        if !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_lowercase() || c == '_')
        {
            // Handle parenthesized operators like (++) :: ...
            if name.starts_with('(') && name.ends_with(')') {
                return String::new(); // Skip operators
            }
            return name.to_string();
        }
    }

    String::new()
}

/// Get the type annotation text from a signature node.
fn get_signature_type(node: tree_sitter::Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(pos) = text.find("::") {
        let type_text = text[pos + 2..].trim();
        return type_text.to_string();
    }
    String::new()
}

/// Get the function name from a `function` node.
/// In tree-sitter-haskell, a function node contains match/equation patterns.
fn get_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the first child which should be the function name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable" | "name" | "identifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    return text;
                }
            }
            // In some grammar versions, the first child is a `match` or `equation`
            "match" | "equation" | "clause" => {
                // Get the name from the match/equation
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    match inner_child.kind() {
                        "variable" | "name" | "identifier" | "prefix_id" => {
                            let text = inner_child
                                .utf8_text(source)
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if !text.is_empty()
                                && text
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_lowercase() || c == '_')
                            {
                                return text;
                            }
                        }
                        _ => {}
                    }
                    // Only check the first meaningful child
                    if inner_child.kind() != "comment" {
                        break;
                    }
                }
            }
            _ => {
                // Check if it's a lowercase identifier
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && text
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_lowercase() || c == '_')
                    && text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
                {
                    return text;
                }
            }
        }
        // Only check the first meaningful child
        if child.kind() != "comment" {
            break;
        }
    }

    // Fallback: extract from text (first word)
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let first_word: String = text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
        .collect();
    if !first_word.is_empty()
        && first_word
            .chars()
            .next()
            .is_some_and(|c| c.is_lowercase() || c == '_')
    {
        return first_word;
    }

    String::new()
}

/// Get the name from a `bind` node (simple pattern binding).
fn get_bind_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Similar to get_function_name but for simple bindings
    get_function_name(node, source)
}

/// Get the type name from a data/newtype/type_alias declaration.
/// The name is the first uppercase identifier after the keyword.
/// In tree-sitter-haskell, the type name is a `name` child node.
fn get_type_decl_name(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    let mut skip_first = true; // Skip the keyword node (data/newtype/type)

    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip keyword nodes
        if matches!(child_kind, "data" | "newtype" | "type")
            || matches!(text.as_str(), "data" | "newtype" | "type")
        {
            skip_first = false;
            continue;
        }

        // The `name` node contains the type name
        if (child_kind == "name"
            || child_kind == "type_name"
            || child_kind == "constructor"
            || child_kind == "constructor_identifier")
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
        {
            return text;
        }

        // Also check for any uppercase identifier that isn't a keyword
        if !skip_first
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !matches!(text.as_str(), "where" | "deriving")
        {
            return text;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    for prefix in &["data ", "newtype ", "type "] {
        if let Some(after) = text.strip_prefix(prefix) {
            let name: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
                return name;
            }
        }
    }

    String::new()
}

/// Get the class name from a `class` node.
/// Pattern: class (Context =>) ClassName typevar where ...
/// In tree-sitter-haskell, the class name is a `name` child node.
fn get_class_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try AST-based extraction first - look for `name` child
    let mut cursor = node.walk();
    let mut found_class_keyword = false;

    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        let text = child.utf8_text(source).unwrap_or("").trim().to_string();

        // Skip the "class" keyword
        if child_kind == "class" || text == "class" {
            found_class_keyword = true;
            continue;
        }

        // Skip context (anything before =>)
        if text.contains("=>") {
            continue;
        }

        // The `name` node after the keyword is the class name
        if found_class_keyword
            && child_kind == "name"
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
        {
            return text;
        }

        // Also check for uppercase identifiers
        if found_class_keyword
            && !text.is_empty()
            && text.chars().next().is_some_and(|c| c.is_uppercase())
            && text.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !matches!(text.as_str(), "where" | "class")
        {
            return text;
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();

    // Remove "class" keyword
    let after_class = if let Some(rest) = text.strip_prefix("class") {
        rest.trim_start()
    } else {
        return String::new();
    };

    // If there's a context (contains =>), skip past it
    let after_context = if let Some(pos) = after_class.find("=>") {
        after_class[pos + 2..].trim_start()
    } else {
        after_class
    };

    // The class name is the first uppercase word
    let name: String = after_context
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
        return name;
    }

    String::new()
}

/// Get the module name from an import statement.
fn get_import_module(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for a module identifier in the import node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "module" | "module_id" | "qualified_module" | "import_module" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty()
                    && text != "import"
                    && text != "qualified"
                    && text.chars().next().is_some_and(|c| c.is_uppercase())
                {
                    return Some(text);
                }
            }
            _ => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                // Module names start with uppercase and may contain dots
                if !text.is_empty()
                    && text != "import"
                    && text != "qualified"
                    && text != "as"
                    && text != "hiding"
                    && text.chars().next().is_some_and(|c| c.is_uppercase())
                    && text
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                {
                    return Some(text);
                }
            }
        }
    }

    // Fallback: parse from text
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let after_import = text.strip_prefix("import")?.trim_start();
    let after_qualified = if after_import.starts_with("qualified") {
        after_import.strip_prefix("qualified")?.trim_start()
    } else {
        after_import
    };

    let module_name: String = after_qualified
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();

    if module_name.is_empty() {
        None
    } else {
        Some(module_name)
    }
}

/// Check if an import is qualified.
fn is_qualified_import(node: tree_sitter::Node, source: &[u8]) -> bool {
    let text = node.utf8_text(source).unwrap_or("");
    text.contains("qualified")
}

/// Get the alias from an import statement (import Module as Alias).
fn get_import_alias(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if let Some(as_pos) = text.find(" as ") {
        let after_as = text[as_pos + 4..].trim();
        let alias: String = after_as
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if !alias.is_empty() {
            return Some(alias);
        }
    }
    None
}

/// Get the function name from an application node's function position.
fn get_applied_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
    match node.kind() {
        "variable" | "identifier" | "name" => {
            node.utf8_text(source).unwrap_or("").trim().to_string()
        }
        // Qualified name: Module.function
        "qualified" | "qualified_variable" => {
            node.utf8_text(source).unwrap_or("").trim().to_string()
        }
        _ => {
            // Try to get text if it looks like an identifier
            let text = node.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty()
                && text
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_lowercase() || c == '_')
                && text
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '\'' || c == '.')
            {
                text
            } else {
                String::new()
            }
        }
    }
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
            "function" => {
                let name = get_function_name(parent, source);
                if !name.is_empty() {
                    return Some(format!("{file}::{name}"));
                }
            }
            "bind" => {
                let name = get_bind_name(parent, source);
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

/// Check if a name is an operator (contains only operator characters).
fn is_operator_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Parenthesized operators like (++)
    if name.starts_with('(') && name.ends_with(')') {
        return true;
    }
    // Pure operator symbols
    name.chars().all(|c| "!#$%&*+./<=>?@\\^|-~:".contains(c))
}

/// Check if a name is a Haskell keyword that should not be treated as a function call.
fn is_haskell_keyword(name: &str) -> bool {
    matches!(
        name,
        "module"
            | "import"
            | "qualified"
            | "as"
            | "hiding"
            | "where"
            | "let"
            | "in"
            | "do"
            | "if"
            | "then"
            | "else"
            | "case"
            | "of"
            | "data"
            | "newtype"
            | "type"
            | "class"
            | "instance"
            | "deriving"
            | "default"
            | "infixl"
            | "infixr"
            | "infix"
            | "foreign"
            | "forall"
            | "return"
            | "pure"
            | "otherwise"
            | "True"
            | "False"
            | "Nothing"
            | "Just"
            | "Left"
            | "Right"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse Haskell source and run the extractor.
    fn parse_haskell(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_haskell::LANGUAGE.into())
            .expect("Haskell grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, "src/Main.hs", source)
    }

    #[test]
    fn test_module_extraction() {
        let source = "module Data.List.Utils where\n\nfoo :: Int -> Int\nfoo x = x + 1\n";
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Data.List.Utils" && n.kind == NodeKind::Module),
            "Should find Data.List.Utils module. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_function_with_signature() {
        let source = r#"module Main where

greet :: String -> String
greet name = "Hello, " ++ name

main :: IO ()
main = do
  putStrLn (greet "World")
"#;
        let result = parse_haskell(source);

        // Check functions (via type signatures)
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::greet" && n.kind == NodeKind::Function),
            "Should find greet function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::main" && n.kind == NodeKind::Function),
            "Should find main function. Nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_data_type_extraction() {
        let source = r#"module Types where

data User = User
  { userName :: String
  , userAge  :: Int
  }

data Color = Red | Green | Blue
"#;
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::User" && n.kind == NodeKind::Class),
            "Should find User data type as Class. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Color" && n.kind == NodeKind::Class),
            "Should find Color data type as Class. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_newtype_extraction() {
        let source = r#"module Types where

newtype UserId = UserId Int
"#;
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::UserId" && n.kind == NodeKind::Class),
            "Should find UserId newtype as Class. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_type_alias_extraction() {
        let source = r#"module Types where

type Name = String
type Age = Int
"#;
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Name" && n.kind == NodeKind::TypeAlias),
            "Should find Name type alias. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_typeclass_extraction() {
        let source = r#"module Lib where

class Printable a where
  prettyPrint :: a -> String
  debugPrint :: a -> String
"#;
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Printable" && n.kind == NodeKind::Trait),
            "Should find Printable typeclass as Trait. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_typeclass_with_context() {
        let source = r#"module Lib where

class Eq a => Ord a where
  compare :: a -> a -> Ordering
"#;
        let result = parse_haskell(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Ord" && n.kind == NodeKind::Trait),
            "Should find Ord typeclass with context as Trait. Nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_imports() {
        let source = r#"module Main where

import Data.Map qualified as Map
import Control.Monad
import Data.List (sort, nub)
import Prelude hiding (map)
"#;
        let result = parse_haskell(source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "Data.Map"),
            "Should find Data.Map import. Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Control.Monad"),
            "Should find Control.Monad import. Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
        assert!(
            imports.iter().any(|e| e.target_fqn == "Data.List"),
            "Should find Data.List import. Imports: {:?}",
            imports.iter().map(|e| &e.target_fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_qualified_import_attributes() {
        let source = r#"module Main where

import qualified Data.Map as Map
"#;
        let result = parse_haskell(source);

        let import = result
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Imports && e.target_fqn == "Data.Map");

        assert!(import.is_some(), "Should find Data.Map import");
        if let Some(imp) = import {
            assert_eq!(
                imp.attributes.get("qualified").and_then(|v| v.as_bool()),
                Some(true),
                "Should be marked as qualified"
            );
            assert_eq!(
                imp.attributes.get("alias").and_then(|v| v.as_str()),
                Some("Map"),
                "Should have alias 'Map'"
            );
        }
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"module Main where

helper :: Int -> Int
helper x = x + 1

main :: IO ()
main = print (helper 42)
"#;
        let result = parse_haskell(source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // main should call helper
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn.ends_with("::main") && e.target_fqn.ends_with("::helper")),
            "main should call helper. Calls found: {:?}",
            calls
                .iter()
                .map(|e| format!("{} -> {}", e.source_fqn, e.target_fqn))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_haskell("");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_combined_extraction() {
        let source = r#"module Main where

import Data.Map qualified as Map
import Control.Monad

data User = User
  { userName :: String
  , userAge  :: Int
  }

newtype UserId = UserId Int

class Printable a where
  prettyPrint :: a -> String

type Name = String

greet :: String -> String
greet name = "Hello, " ++ name

main :: IO ()
main = do
  putStrLn (greet "World")
"#;
        let result = parse_haskell(source);

        // Check module
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Main" && n.kind == NodeKind::Module),
            "Should find Main module"
        );

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.len() >= 2, "Should have at least 2 imports");

        // Check data type
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::User" && n.kind == NodeKind::Class),
            "Should find User data type"
        );

        // Check newtype
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::UserId" && n.kind == NodeKind::Class),
            "Should find UserId newtype"
        );

        // Check typeclass
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Printable" && n.kind == NodeKind::Trait),
            "Should find Printable typeclass"
        );

        // Check type alias
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::Name" && n.kind == NodeKind::TypeAlias),
            "Should find Name type alias"
        );

        // Check functions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::greet" && n.kind == NodeKind::Function),
            "Should find greet function"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::main" && n.kind == NodeKind::Function),
            "Should find main function"
        );
    }

    #[test]
    fn test_multiple_function_equations() {
        let source = r#"module Main where

factorial :: Int -> Int
factorial 0 = 1
factorial n = n * factorial (n - 1)
"#;
        let result = parse_haskell(source);

        // Should only create one node for factorial (multiple equations merged)
        let factorial_nodes: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.fqn.ends_with("::factorial"))
            .collect();

        assert_eq!(
            factorial_nodes.len(),
            1,
            "Should have exactly one factorial node (multiple equations merged). Found: {:?}",
            factorial_nodes
        );
    }

    #[test]
    fn test_complexity_computed() {
        let source = r#"module Main where

complex :: Int -> String
complex x =
  if x > 0
    then case x of
      1 -> "one"
      2 -> "two"
      _ -> "other"
    else "negative"
"#;
        let result = parse_haskell(source);

        let func = result.nodes.iter().find(|n| n.fqn.ends_with("::complex"));

        assert!(func.is_some(), "Should find complex function");
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

    #[test]
    fn test_extract_regex_backward_compat() {
        let source = r#"module Main where

greet :: String -> String
greet name = "Hello, " ++ name
"#;
        #[allow(deprecated)]
        let result = extract_regex("src/Main.hs", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "src/Main.hs::greet" && n.kind == NodeKind::Function),
            "extract_regex should still work for backward compatibility"
        );
    }
}
