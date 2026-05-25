//! Bash/Shell AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (functions, aliases) and edges (imports via source/dot,
//! intra-file calls) from a tree-sitter Bash parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::indexer::complexity;
use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Bash file.
///
/// Handles:
/// - Function definitions (`function_name() { ... }` and `function function_name { ... }`)
/// - Aliases (`alias name='...'`) - treated as constants
/// - Imports (`source file.sh`, `. file.sh`) - dot-source commands
/// - Intra-file function calls (simple command names matching defined functions)
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
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .expect("Bash grammar should load");
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
        if node.kind == NodeKind::Function
            && let Some(ast_node) =
                find_ast_node_at_line(root, node.start_line, "function_definition")
        {
            let c = complexity::compute_full_complexity(ast_node, source, "bash");
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

/// Recursively collect function definitions and alias declarations from the AST.
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
            "function_definition" => {
                extract_function(child, file, source, nodes, defined_fqns);
            }
            "variable_assignment" => {
                // Check if this is an alias: `alias name='...'`
                extract_alias(child, file, source, nodes, defined_fqns);
            }
            "command" => {
                // Check for alias commands: `alias name='...'`
                extract_alias_command(child, file, source, nodes, defined_fqns);
            }
            _ => {
                // Recurse into compound statements, if/else, etc.
                if child.child_count() > 0 {
                    collect_definitions(child, file, source, nodes, defined_fqns);
                }
            }
        }
    }
}

/// Extract a function definition node.
///
/// Handles both styles:
/// - `function_name() { ... }`
/// - `function function_name { ... }`
fn extract_function(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // In tree-sitter-bash, function_definition has a "name" field
    let name = if let Some(name_node) = node.child_by_field_name("name") {
        name_node.utf8_text(source).unwrap_or("").trim().to_string()
    } else {
        // Fallback: look for the first word child
        get_first_word_child(node, source)
    };

    if name.is_empty() {
        return;
    }

    // Skip shell keywords that might accidentally match
    if matches!(
        name.as_str(),
        "if" | "for" | "while" | "until" | "case" | "select" | "time"
    ) {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Avoid duplicates
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
        attributes: json!({}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract an alias from a variable_assignment node.
/// In some bash grammars, `alias foo='bar'` may parse as a variable_assignment.
fn extract_alias(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    // Only handle if it looks like an alias assignment
    if !text.starts_with("alias ") {
        return;
    }

    // Parse: alias name='value' or alias name="value"
    let after_alias = text.strip_prefix("alias ").unwrap_or("");
    let name: String = after_alias
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();

    if name.is_empty() {
        return;
    }

    let fqn = format!("{file}::{name}");
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Constant,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!({"alias": true}),
    });

    defined_fqns.push((name, fqn));
}

/// Extract an alias from a command node (e.g., `alias ll='ls -la'`).
/// In tree-sitter-bash, `alias name='value'` often parses as a `command` node
/// where the command name is "alias" and the arguments contain the assignment.
fn extract_alias_command(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    defined_fqns: &mut Vec<(String, String)>,
) {
    // Get the command name (first child or "name" field)
    let cmd_name = get_command_name(node, source);
    if cmd_name != "alias" {
        return;
    }

    // The alias argument is typically the second child or in the arguments
    // Look for text like `name='value'` or `name="value"`
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    let after_alias = text.strip_prefix("alias").unwrap_or("").trim();

    // Parse potentially multiple aliases: alias a='x' b='y'
    for part in split_alias_args(after_alias) {
        let name: String = part
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();

        if name.is_empty() {
            continue;
        }

        let fqn = format!("{file}::{name}");
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        if nodes.iter().any(|n| n.fqn == fqn) {
            continue;
        }

        nodes.push(Node {
            fqn: fqn.clone(),
            kind: NodeKind::Constant,
            file: file.to_string(),
            start_line,
            end_line,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({"alias": true}),
        });

        defined_fqns.push((name, fqn));
    }
}

/// Split alias arguments, handling quoted values.
/// e.g., `ll='ls -la' gs='git status'` -> ["ll='ls -la'", "gs='git status'"]
fn split_alias_args(input: &str) -> Vec<&str> {
    // Simple split: each alias definition contains an '=' sign
    // We split on spaces that are NOT inside quotes
    let mut results = Vec::new();
    let mut start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                let segment = input[start..i].trim();
                if !segment.is_empty() && segment.contains('=') {
                    results.push(segment);
                }
                start = i + 1;
            }
            _ => {}
        }
    }

    // Last segment
    let segment = input[start..].trim();
    if !segment.is_empty() && segment.contains('=') {
        results.push(segment);
    }

    results
}

/// Collect source/dot import commands from the AST.
fn collect_imports(node: tree_sitter::Node, file: &str, source: &[u8], edges: &mut Vec<Edge>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            let cmd_name = get_command_name(child, source);
            if cmd_name == "source" || cmd_name == "." {
                // Get the argument (the file being sourced)
                if let Some(target) = get_command_first_arg(child, source)
                    && !target.is_empty()
                {
                    edges.push(Edge {
                        id: None,
                        source_fqn: file.to_string(),
                        target_fqn: target,
                        kind: EdgeKind::Imports,
                        confidence: 1.0,
                        attributes: json!({}),
                    });
                }
            }
        }

        // Recurse into compound statements, subshells, etc.
        if child.child_count() > 0 && child.kind() != "function_definition" {
            collect_imports(child, file, source, edges);
        }
    }
}

/// Collect intra-file function calls.
/// A call is a `command` node whose command name matches a defined function.
fn collect_calls(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    defined_fqns: &[(String, String)],
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            let cmd_name = get_command_name(child, source);
            if !cmd_name.is_empty() {
                // Check if this command name matches a defined function
                if let Some((_simple, target_fqn)) =
                    defined_fqns.iter().find(|(simple, _)| simple == &cmd_name)
                {
                    // Determine the enclosing function for the source_fqn
                    let source_fqn = find_enclosing_function_fqn(child, file, source)
                        .unwrap_or_else(|| file.to_string());

                    // Avoid self-calls from the function definition itself
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
        }

        // Recurse
        if child.child_count() > 0 {
            collect_calls(child, file, source, defined_fqns, edges);
        }
    }
}

/// Get the command name from a `command` node.
/// In tree-sitter-bash, a command node has a "name" field or the first child is the command_name.
fn get_command_name(node: tree_sitter::Node, source: &[u8]) -> String {
    // Try the "name" field first
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).unwrap_or("").trim().to_string();
    }

    // Fallback: first child that is a "command_name" or "word"
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" => {
                // command_name may contain a "word" child
                if let Some(word) = child.child(0) {
                    return word.utf8_text(source).unwrap_or("").trim().to_string();
                }
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            "word" => {
                return child.utf8_text(source).unwrap_or("").trim().to_string();
            }
            _ => {}
        }
    }

    String::new()
}

/// Get the first argument of a command node (after the command name).
fn get_command_first_arg(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut found_name = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" | "word" if !found_name => {
                found_name = true;
            }
            "word" | "string" | "raw_string" | "concatenation" | "simple_expansion"
                if found_name =>
            {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                // Strip surrounding quotes if present
                let cleaned = text.trim_matches('\'').trim_matches('"').to_string();
                return Some(cleaned);
            }
            _ if found_name && child.kind() != "comment" => {
                // Any other argument node after the command name
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    let cleaned = text.trim_matches('\'').trim_matches('"').to_string();
                    return Some(cleaned);
                }
            }
            _ => {}
        }
    }

    None
}

/// Get the first word/identifier child of a node (fallback for function name extraction).
fn get_first_word_child(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "word" || child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("").trim().to_string();
        }
    }
    String::new()
}

/// Find the enclosing function_definition for a given node and return its FQN.
fn find_enclosing_function_fqn(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            // Get the function name
            let name = if let Some(name_node) = parent.child_by_field_name("name") {
                name_node.utf8_text(source).unwrap_or("").trim().to_string()
            } else {
                get_first_word_child(parent, source)
            };
            if !name.is_empty() {
                return Some(format!("{file}::{name}"));
            }
        }
        current = parent.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse bash source and run the extractor.
    fn parse_bash(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("Bash grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, "scripts/deploy.sh", source)
    }

    #[test]
    fn test_function_definitions_both_styles() {
        let source = r#"#!/bin/bash

# Style 1: name() { ... }
cleanup() {
  rm -rf /tmp/build
}

# Style 2: function name { ... }
function setup_env {
  export PATH="/usr/local/bin:$PATH"
}

# Style 3: function name() { ... }
function deploy() {
  echo "Deploying..."
}
"#;
        let result = parse_bash(source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::cleanup" && n.kind == NodeKind::Function),
            "Should find cleanup function"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::setup_env" && n.kind == NodeKind::Function),
            "Should find setup_env function"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::deploy" && n.kind == NodeKind::Function),
            "Should find deploy function"
        );
    }

    #[test]
    fn test_aliases() {
        let source = r#"#!/bin/bash

alias ll='ls -la'
alias gs='git status'
alias gp="git push"
"#;
        let result = parse_bash(source);

        let constants: Vec<&Node> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constant)
            .collect();

        assert!(
            constants.iter().any(|n| n.fqn == "scripts/deploy.sh::ll"),
            "Should find ll alias"
        );
        assert!(
            constants.iter().any(|n| n.fqn == "scripts/deploy.sh::gs"),
            "Should find gs alias"
        );
        assert!(
            constants.iter().any(|n| n.fqn == "scripts/deploy.sh::gp"),
            "Should find gp alias"
        );
    }

    #[test]
    fn test_source_dot_imports() {
        let source = r#"#!/bin/bash

source ./config.sh
. /etc/profile.d/env.sh
source "$HOME/.bashrc"
"#;
        let result = parse_bash(source);

        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(
            imports.iter().any(|e| e.target_fqn == "./config.sh"),
            "Should find source ./config.sh import"
        );
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "/etc/profile.d/env.sh"),
            "Should find dot-source import"
        );
    }

    #[test]
    fn test_intra_file_calls() {
        let source = r#"#!/bin/bash

setup_env() {
  export PATH="/usr/local/bin:$PATH"
}

deploy() {
  setup_env
  echo "Deploying..."
}

main() {
  setup_env
  deploy
}

main "$@"
"#;
        let result = parse_bash(source);

        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        // deploy calls setup_env
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "scripts/deploy.sh::deploy"
                    && e.target_fqn == "scripts/deploy.sh::setup_env"),
            "deploy should call setup_env"
        );

        // main calls setup_env
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "scripts/deploy.sh::main"
                    && e.target_fqn == "scripts/deploy.sh::setup_env"),
            "main should call setup_env"
        );

        // main calls deploy
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "scripts/deploy.sh::main"
                    && e.target_fqn == "scripts/deploy.sh::deploy"),
            "main should call deploy"
        );

        // Top-level call to main
        assert!(
            calls.iter().any(|e| e.source_fqn == "scripts/deploy.sh"
                && e.target_fqn == "scripts/deploy.sh::main"),
            "Top-level should call main"
        );
    }

    #[test]
    fn test_empty_file() {
        let result = parse_bash("");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_combined_extraction() {
        let source = r#"#!/bin/bash

source ./config.sh
. /etc/profile.d/env.sh

alias ll='ls -la'

function setup_env {
  export PATH="/usr/local/bin:$PATH"
}

cleanup() {
  rm -rf /tmp/build
}

function deploy() {
  setup_env
  echo "Deploying..."
}

main() {
  setup_env
  deploy
  cleanup
}

main "$@"
"#;
        let result = parse_bash(source);

        // Check imports
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| e.target_fqn == "./config.sh"));
        assert!(
            imports
                .iter()
                .any(|e| e.target_fqn == "/etc/profile.d/env.sh")
        );

        // Check functions
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::setup_env" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::cleanup" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::deploy" && n.kind == NodeKind::Function)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::main" && n.kind == NodeKind::Function)
        );

        // Check alias
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "scripts/deploy.sh::ll" && n.kind == NodeKind::Constant)
        );

        // Check calls
        let calls: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "scripts/deploy.sh::deploy"
                    && e.target_fqn == "scripts/deploy.sh::setup_env")
        );
        assert!(
            calls
                .iter()
                .any(|e| e.source_fqn == "scripts/deploy.sh::main"
                    && e.target_fqn == "scripts/deploy.sh::deploy")
        );
    }
}
