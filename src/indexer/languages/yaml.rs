//! YAML AST extractor (tree-sitter based).
//!
//! Extracts structural nodes from YAML configuration files using tree-sitter parsing.
//! Designed to make YAML files navigable in the graph (Kubernetes manifests,
//! Docker Compose files, CI/CD configs, etc.).
//!
//! Handles:
//! - Top-level mapping keys (e.g., `services:`, `apiVersion:`) → NodeKind::Module
//! - Second-level nested keys (e.g., `services.web:`, `spec.containers:`) → NodeKind::Class
//! - YAML anchors (`&anchor_name`) → NodeKind::Constant
//! - Multi-document YAML (separated by `---`) → each document processed independently

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed YAML file.
///
/// FQN format:
/// - Top-level keys: `file::key_name`
/// - Nested keys: `file::top_key::nested_key`
/// - Anchors: `file::&anchor_name`
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    collect_yaml_nodes(root, file, source_bytes, &mut nodes, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Deprecated wrapper for backward compatibility with the regex-based pipeline.
/// New code should use `extract()` with a pre-parsed tree.
#[deprecated(note = "Use extract() with a tree-sitter Tree instead")]
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_yaml::LANGUAGE.into())
        .expect("YAML grammar should load");
    match parser.parse(source, None) {
        Some(tree) => extract(&tree, file, source),
        None => ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        },
    }
}

// ─── Node Collection ────────────────────────────────────────────────────────

/// Recursively collect YAML structural nodes from the AST.
///
/// In tree-sitter-yaml, the top-level structure is a `stream` containing
/// `document` nodes. Each document contains a `block_node` which typically
/// holds a `block_mapping` with `block_mapping_pair` entries.
fn collect_yaml_nodes(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        match kind {
            // A YAML document (between --- separators or the implicit single document)
            "document" => {
                process_document(child, file, source, nodes, edges);
            }
            // Top-level stream may contain documents directly
            "block_node" | "block_mapping" => {
                process_top_level_mapping(child, file, source, nodes, edges);
            }
            _ => {
                // Recurse into other container nodes
                if child.child_count() > 0 {
                    collect_yaml_nodes(child, file, source, nodes, edges);
                }
            }
        }
    }
}

/// Process a single YAML document node.
///
/// A document typically contains a block_node -> block_mapping with top-level pairs.
fn process_document(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        match kind {
            "block_node" | "block_mapping" => {
                process_top_level_mapping(child, file, source, nodes, edges);
            }
            _ => {
                if child.child_count() > 0 {
                    process_document(child, file, source, nodes, edges);
                }
            }
        }
    }
}

/// Process a top-level block mapping, extracting top-level keys as Module nodes.
///
/// Walks through block_mapping_pair nodes at the top level.
fn process_top_level_mapping(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        match kind {
            "block_mapping_pair" => {
                process_top_level_pair(child, file, source, nodes, edges);
            }
            "block_mapping" => {
                // Recurse into nested block_mapping (can happen in some AST structures)
                process_top_level_mapping(child, file, source, nodes, edges);
            }
            _ => {}
        }
    }
}

/// Process a top-level key-value pair.
///
/// Extracts the key as a NodeKind::Module and processes nested keys and anchors.
fn process_top_level_pair(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    // Extract the key from the pair
    let key_name = match extract_pair_key(node, source) {
        Some(k) => k,
        None => return,
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::{key_name}");

    // Avoid duplicate nodes
    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("yaml_type".to_string(), json!("top_level_key"));

    nodes.push(Node {
        fqn: fqn.clone(),
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });

    // Process the value side for nested keys and anchors
    if let Some(value_node) = get_pair_value(node) {
        // Check for anchors in the value
        extract_anchors_from_node(value_node, file, source, &fqn, nodes, edges);

        // Extract second-level nested keys
        extract_nested_keys(value_node, file, source, &key_name, nodes, edges);
    }
}

/// Extract second-level keys from a value node.
///
/// These are keys nested one level below a top-level key.
/// For example, under `services:`, the keys `web:` and `db:` become Class nodes.
fn extract_nested_keys(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    parent_key: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        match kind {
            "block_mapping_pair" => {
                if let Some(nested_key) = extract_pair_key(child, source) {
                    let start_line = child.start_position().row as u32 + 1;
                    let end_line = child.end_position().row as u32 + 1;
                    let fqn = format!("{file}::{parent_key}::{nested_key}");

                    if !nodes.iter().any(|n| n.fqn == fqn) {
                        let mut attrs = serde_json::Map::new();
                        attrs.insert("yaml_type".to_string(), json!("nested_key"));
                        attrs.insert("parent_key".to_string(), json!(parent_key));

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
                    }

                    // Check for anchors in nested values
                    if let Some(value_node) = get_pair_value(child) {
                        extract_anchors_from_node(value_node, file, source, &fqn, nodes, edges);
                    }
                }
            }
            "block_mapping" | "block_node" => {
                // Recurse into block_mapping/block_node to find pairs
                extract_nested_keys(child, file, source, parent_key, nodes, edges);
            }
            _ => {}
        }
    }
}

/// Extract YAML anchors from a node and its descendants.
///
/// Anchors are defined with `&name` syntax and produce NodeKind::Constant nodes.
/// Also detects alias references (`*name`) and creates Imports edges.
fn extract_anchors_from_node(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    context_fqn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut stack = vec![node];

    while let Some(current) = stack.pop() {
        let kind = current.kind();

        match kind {
            "anchor" => {
                // Anchor node: &name
                let anchor_text = current.utf8_text(source).unwrap_or("").trim();
                // Strip the leading '&' if present
                let anchor_name = anchor_text.strip_prefix('&').unwrap_or(anchor_text);

                if !anchor_name.is_empty() {
                    let start_line = current.start_position().row as u32 + 1;
                    let end_line = current.end_position().row as u32 + 1;
                    let fqn = format!("{file}::&{anchor_name}");

                    if !nodes.iter().any(|n| n.fqn == fqn) {
                        let mut attrs = serde_json::Map::new();
                        attrs.insert("yaml_type".to_string(), json!("anchor"));
                        attrs.insert("anchor".to_string(), json!(anchor_name));
                        attrs.insert("defined_in".to_string(), json!(context_fqn));

                        nodes.push(Node {
                            fqn,
                            kind: NodeKind::Constant,
                            file: file.to_string(),
                            start_line,
                            end_line,
                            file_hash: String::new(),
                            indexed_at: 0,
                            attributes: json!(attrs),
                        });
                    }
                }
            }
            "alias" => {
                // Alias node: *name (reference to an anchor)
                let alias_text = current.utf8_text(source).unwrap_or("").trim();
                // Strip the leading '*' if present
                let alias_name = alias_text.strip_prefix('*').unwrap_or(alias_text);

                if !alias_name.is_empty() {
                    let target_fqn = format!("{file}::&{alias_name}");

                    // Create an Imports edge from the context to the anchor
                    if !edges.iter().any(|e| {
                        e.kind == EdgeKind::Imports
                            && e.source_fqn == context_fqn
                            && e.target_fqn == target_fqn
                    }) {
                        edges.push(Edge {
                            id: None,
                            source_fqn: context_fqn.to_string(),
                            target_fqn,
                            kind: EdgeKind::Imports,
                            confidence: 0.9,
                            edge_source: crate::store::confidence::EdgeSource::AstDirect,
                            attributes: json!({"yaml_type": "alias_reference"}),
                        });
                    }
                }
            }
            _ => {}
        }

        // Push children onto the stack for traversal
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}

// ─── Utility Functions ──────────────────────────────────────────────────────

/// Extract the key name from a block_mapping_pair node.
///
/// In tree-sitter-yaml, a block_mapping_pair has children where the first
/// meaningful child is the key (often a `flow_node` or plain scalar).
fn extract_pair_key(pair_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = pair_node.walk();

    for child in pair_node.children(&mut cursor) {
        let kind = child.kind();

        // The key in a block_mapping_pair is typically the first child
        // that is a flow_node, plain_scalar, or similar scalar type
        match kind {
            "flow_node" | "block_scalar" => {
                let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                if !text.is_empty() && text != ":" {
                    return Some(sanitize_key(&text));
                }
            }
            _ => {
                // Try to get text from leaf nodes that look like keys
                if child.child_count() == 0 {
                    let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                    // Skip colons, whitespace, and empty strings
                    if !text.is_empty() && text != ":" && text != "-" {
                        return Some(sanitize_key(&text));
                    }
                } else {
                    // Recurse into the child to find the actual key text
                    let key = extract_scalar_text(child, source);
                    if let Some(k) = key
                        && !k.is_empty()
                        && k != ":"
                    {
                        return Some(sanitize_key(&k));
                    }
                }
            }
        }
    }

    None
}

/// Extract scalar text from a node, recursing into children if needed.
fn extract_scalar_text(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // If this is a leaf node, return its text
    if node.child_count() == 0 {
        let text = node.utf8_text(source).unwrap_or("").trim().to_string();
        if !text.is_empty() && text != ":" && text != "-" {
            return Some(text);
        }
        return None;
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip anchor/tag nodes when extracting key text
        if child.kind() == "anchor" || child.kind() == "tag" {
            continue;
        }
        if let Some(text) = extract_scalar_text(child, source) {
            return Some(text);
        }
    }

    None
}

/// Get the value node from a block_mapping_pair.
///
/// The value is typically the second significant child after the key and colon.
fn get_pair_value(pair_node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let child_count = pair_node.child_count();
    if child_count < 2 {
        return None;
    }

    // In tree-sitter-yaml, the value is usually the last child of the pair
    // that is not a ":" token
    let mut cursor = pair_node.walk();
    let children: Vec<tree_sitter::Node> = pair_node.children(&mut cursor).collect();

    // Walk from the end to find the value node
    for child in children.iter().rev() {
        let kind = child.kind();
        // Skip the colon separator and the key
        if kind != ":" || kind == "flow_node" || kind == "block_node" {
            // The value could be a block_node, flow_node, block_scalar, etc.
            if kind == "block_node"
                || kind == "flow_node"
                || kind == "block_scalar"
                || kind == "block_sequence"
                || kind == "flow_sequence"
                || kind == "flow_mapping"
            {
                return Some(*child);
            }
        }
    }

    // Fallback: return the last child if it has children (likely a value container)
    if let Some(last) = children.last()
        && last.child_count() > 0
    {
        return Some(*last);
    }

    None
}

/// Sanitize a YAML key name for use in FQNs.
///
/// Removes surrounding quotes and trims whitespace.
fn sanitize_key(key: &str) -> String {
    let trimmed = key.trim();
    // Remove surrounding quotes (single or double)
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse YAML source and run the extractor.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("YAML grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, file, source)
    }

    #[test]
    fn test_extract_top_level_keys() {
        let source = r#"apiVersion: v1
kind: Service
metadata:
  name: my-service
spec:
  selector:
    app: web
  ports:
    - port: 80
"#;
        let result = parse_and_extract("k8s/service.yml", source);

        // Top-level keys should be Module nodes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/service.yml::apiVersion" && n.kind == NodeKind::Module),
            "Expected top-level key 'apiVersion' as Module. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/service.yml::kind" && n.kind == NodeKind::Module),
            "Expected top-level key 'kind'"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/service.yml::metadata" && n.kind == NodeKind::Module),
            "Expected top-level key 'metadata'"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/service.yml::spec" && n.kind == NodeKind::Module),
            "Expected top-level key 'spec'"
        );
    }

    #[test]
    fn test_extract_nested_keys() {
        let source = r#"services:
  web:
    image: nginx
    ports:
      - "80:80"
  db:
    image: postgres
    environment:
      POSTGRES_DB: mydb
"#;
        let result = parse_and_extract("docker-compose.yml", source);

        // Top-level key
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::services" && n.kind == NodeKind::Module),
            "Expected top-level key 'services'. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );

        // Nested keys should be Class nodes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::services::web" && n.kind == NodeKind::Class),
            "Expected nested key 'services::web' as Class. Got nodes: {:?}",
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
                .any(|n| n.fqn == "docker-compose.yml::services::db" && n.kind == NodeKind::Class),
            "Expected nested key 'services::db' as Class"
        );
    }

    #[test]
    fn test_extract_anchors() {
        let source = r#"defaults: &defaults
  adapter: postgres
  host: localhost
  port: 5432

development:
  database: myapp_dev
  <<: *defaults

production:
  database: myapp_prod
  <<: *defaults
"#;
        let result = parse_and_extract("config/database.yml", source);

        // Anchor should be a Constant node
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "config/database.yml::&defaults" && n.kind == NodeKind::Constant),
            "Expected anchor '&defaults' as Constant. Got nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );

        // Check anchor attribute
        let anchor_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "config/database.yml::&defaults");
        if let Some(node) = anchor_node {
            assert_eq!(node.attributes["yaml_type"], "anchor");
            assert_eq!(node.attributes["anchor"], "defaults");
        }

        // Alias references should create Imports edges
        let alias_edges: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| {
                e.kind == EdgeKind::Imports && e.target_fqn == "config/database.yml::&defaults"
            })
            .collect();
        assert!(
            !alias_edges.is_empty(),
            "Expected alias reference edges to &defaults. Got edges: {:?}",
            result.edges
        );
    }

    #[test]
    fn test_extract_multi_document_yaml() {
        let source = r#"---
apiVersion: v1
kind: ConfigMap
metadata:
  name: config1
---
apiVersion: v1
kind: Secret
metadata:
  name: secret1
"#;
        let result = parse_and_extract("k8s/multi.yml", source);

        // Both documents should have their top-level keys extracted
        // Note: keys with same name across documents will deduplicate by FQN
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/multi.yml::apiVersion" && n.kind == NodeKind::Module),
            "Expected 'apiVersion' from multi-doc. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/multi.yml::kind" && n.kind == NodeKind::Module),
            "Expected 'kind' from multi-doc"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/multi.yml::metadata" && n.kind == NodeKind::Module),
            "Expected 'metadata' from multi-doc"
        );
    }

    #[test]
    fn test_extract_kubernetes_deployment() {
        let source = r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: nginx-deployment
  labels:
    app: nginx
spec:
  replicas: 3
  selector:
    matchLabels:
      app: nginx
  template:
    metadata:
      labels:
        app: nginx
    spec:
      containers:
        - name: nginx
          image: nginx:1.14.2
          ports:
            - containerPort: 80
"#;
        let result = parse_and_extract("k8s/deployment.yml", source);

        // Top-level keys
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/deployment.yml::apiVersion")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/deployment.yml::kind")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/deployment.yml::metadata")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "k8s/deployment.yml::spec")
        );

        // Nested keys under metadata
        assert!(
            result.nodes.iter().any(|n| n.fqn == "k8s/deployment.yml::metadata::name"
                && n.kind == NodeKind::Class),
            "Expected nested key 'metadata::name'. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_docker_compose_with_anchors() {
        let source = r#"version: "3.8"

x-common: &common
  restart: always
  logging:
    driver: json-file

services:
  web:
    <<: *common
    image: nginx
    ports:
      - "80:80"
  api:
    <<: *common
    image: myapp
    ports:
      - "3000:3000"
"#;
        let result = parse_and_extract("docker-compose.yml", source);

        // Top-level keys
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::version")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::services")
        );

        // Anchor
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::&common" && n.kind == NodeKind::Constant),
            "Expected anchor '&common'. Got nodes: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.fqn, &n.kind))
                .collect::<Vec<_>>()
        );

        // Nested service keys
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::services::web" && n.kind == NodeKind::Class),
            "Expected nested 'services::web'. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "docker-compose.yml::services::api" && n.kind == NodeKind::Class),
            "Expected nested 'services::api'"
        );
    }

    #[test]
    fn test_extract_empty_yaml() {
        let result = parse_and_extract("empty.yml", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_extract_comment_only_yaml() {
        let source = r#"# This is a comment
# Another comment
"#;
        let result = parse_and_extract("comments.yml", source);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_extract_ci_config() {
        let source = r#"name: CI
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - run: npm test
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - run: npm run lint
"#;
        let result = parse_and_extract(".github/workflows/ci.yml", source);

        // Top-level keys
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == ".github/workflows/ci.yml::name")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == ".github/workflows/ci.yml::on")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == ".github/workflows/ci.yml::jobs")
        );

        // Nested keys under jobs
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == ".github/workflows/ci.yml::jobs::build"
                    && n.kind == NodeKind::Class),
            "Expected nested 'jobs::build'. Got nodes: {:?}",
            result.nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == ".github/workflows/ci.yml::jobs::lint"
                    && n.kind == NodeKind::Class),
            "Expected nested 'jobs::lint'"
        );
    }

    #[test]
    fn test_deprecated_extract_regex_wrapper() {
        let source = "key: value\nnested:\n  child: data\n";
        #[allow(deprecated)]
        let result = extract_regex("test.yml", source);
        // Should produce at least the top-level keys
        assert!(
            !result.nodes.is_empty(),
            "extract_regex wrapper should produce nodes"
        );
    }
}
