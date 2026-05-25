//! Terraform/HCL AST extractor (tree-sitter based).
//!
//! Extracts structural nodes (resources, data sources, modules, variables, outputs,
//! locals, providers) and edges (module source imports) from a tree-sitter HCL parse tree.

use serde_json::json;
use tree_sitter::Tree;

use crate::store::types::{Edge, EdgeKind, ExtractionResult, Node, NodeKind};

/// Extract structural nodes and edges from a parsed Terraform/HCL file.
///
/// Handles:
/// - Resources (`resource "type" "name" { ... }`) → NodeKind::Class
/// - Data sources (`data "type" "name" { ... }`) → NodeKind::Class
/// - Modules (`module "name" { source = "..." }`) → NodeKind::Module + import edge
/// - Variables (`variable "name" { ... }`) → NodeKind::Constant
/// - Outputs (`output "name" { ... }`) → NodeKind::Constant
/// - Locals (`locals { key = value ... }`) → NodeKind::Constant per key
/// - Providers (`provider "name" { ... }`) → NodeKind::Class
/// - Terraform settings (`terraform { ... }`) → NodeKind::Module
pub fn extract(tree: &Tree, file: &str, source: &str) -> ExtractionResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    collect_blocks(root, file, source_bytes, &mut nodes, &mut edges);

    ExtractionResult { nodes, edges }
}

/// Deprecated wrapper for backward compatibility with the regex-based pipeline.
/// New code should use `extract()` with a pre-parsed tree.
#[deprecated(note = "Use extract() with a tree-sitter Tree instead")]
pub fn extract_regex(file: &str, source: &str) -> ExtractionResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_hcl::LANGUAGE.into())
        .expect("HCL grammar should load");
    match parser.parse(source, None) {
        Some(tree) => extract(&tree, file, source),
        None => ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        },
    }
}

// ─── Block Collection ───────────────────────────────────────────────────────

/// Recursively collect HCL blocks from the AST.
///
/// In tree-sitter-hcl, the top-level structure is a `config_file` or `body`
/// containing `block` nodes. Each block has:
/// - An identifier (the block type: resource, data, module, variable, output, locals, provider, terraform)
/// - Zero or more string literal labels
/// - A body containing attributes and nested blocks
fn collect_blocks(
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
            "block" => {
                process_block(child, file, source, nodes, edges);
            }
            _ => {
                // Recurse into container nodes (config_file, body, etc.)
                if child.child_count() > 0 {
                    collect_blocks(child, file, source, nodes, edges);
                }
            }
        }
    }
}

/// Process a single HCL block node.
///
/// Determines the block type from the first identifier child and dispatches
/// to the appropriate handler.
fn process_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let text = node.utf8_text(source).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return;
    }

    // Extract the block type (first word before any quotes or braces)
    let block_type = extract_block_type(&text);

    match block_type.as_str() {
        "resource" => extract_resource_block(node, file, source, &text, nodes),
        "data" => extract_data_block(node, file, source, &text, nodes),
        "module" => extract_module_block(node, file, source, &text, nodes, edges),
        "variable" => extract_variable_block(node, file, source, &text, nodes),
        "output" => extract_output_block(node, file, source, &text, nodes),
        "locals" => extract_locals_block(node, file, source, nodes),
        "provider" => extract_provider_block(node, file, source, &text, nodes),
        "terraform" => extract_terraform_block(node, file, source, nodes),
        _ => {
            // Unknown block type - skip
        }
    }
}

/// Extract the block type keyword from block text.
/// Returns the first identifier (e.g., "resource", "data", "module").
fn extract_block_type(text: &str) -> String {
    text.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .to_string()
}

// ─── Resource Blocks ────────────────────────────────────────────────────────

/// Extract a resource block: `resource "type" "name" { ... }`
///
/// Produces a NodeKind::Class with FQN: `file::resource.type.name`
fn extract_resource_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
) {
    let labels = extract_string_labels(text);
    if labels.len() < 2 {
        return;
    }

    let resource_type = &labels[0];
    let resource_name = &labels[1];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::resource.{resource_type}.{resource_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    // Try to extract lifecycle, depends_on, or other notable attributes
    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("resource"));
    attrs.insert("resource_type".to_string(), json!(resource_type));

    // Check for count or for_each
    let body_text = node.utf8_text(source).unwrap_or("");
    if body_text.contains("count") {
        attrs.insert("has_count".to_string(), json!(true));
    }
    if body_text.contains("for_each") {
        attrs.insert("has_for_each".to_string(), json!(true));
    }

    nodes.push(Node {
        fqn,
        kind: NodeKind::Class,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
}

// ─── Data Source Blocks ─────────────────────────────────────────────────────

/// Extract a data source block: `data "type" "name" { ... }`
///
/// Produces a NodeKind::Class with FQN: `file::data.type.name`
fn extract_data_block(
    node: tree_sitter::Node,
    file: &str,
    _source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
) {
    let labels = extract_string_labels(text);
    if labels.len() < 2 {
        return;
    }

    let data_type = &labels[0];
    let data_name = &labels[1];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::data.{data_type}.{data_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("data"));
    attrs.insert("data_type".to_string(), json!(data_type));

    nodes.push(Node {
        fqn,
        kind: NodeKind::Class,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
}

// ─── Module Blocks ──────────────────────────────────────────────────────────

/// Extract a module block: `module "name" { source = "..." ... }`
///
/// Produces a NodeKind::Module with FQN: `file::module.name`
/// Also emits an Imports edge for the `source` attribute if present.
fn extract_module_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let labels = extract_string_labels(text);
    if labels.is_empty() {
        return;
    }

    let module_name = &labels[0];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::module.{module_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("module"));

    // Extract the source attribute value for the import edge
    let body_text = node.utf8_text(source).unwrap_or("");
    if let Some(module_source) = extract_attribute_value(body_text, "source") {
        attrs.insert("source".to_string(), json!(module_source));

        // Emit an import edge from this module to its source
        if !edges.iter().any(|e| {
            e.kind == EdgeKind::Imports && e.source_fqn == fqn && e.target_fqn == module_source
        }) {
            edges.push(Edge {
                id: None,
                source_fqn: fqn.clone(),
                target_fqn: module_source,
                kind: EdgeKind::Imports,
                confidence: 1.0,
                attributes: json!({}),
            });
        }
    }

    // Check for version constraint
    if let Some(version) = extract_attribute_value(body_text, "version") {
        attrs.insert("version".to_string(), json!(version));
    }

    nodes.push(Node {
        fqn,
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
}

// ─── Variable Blocks ────────────────────────────────────────────────────────

/// Extract a variable block: `variable "name" { ... }`
///
/// Produces a NodeKind::Constant with FQN: `file::var.name`
fn extract_variable_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
) {
    let labels = extract_string_labels(text);
    if labels.is_empty() {
        return;
    }

    let var_name = &labels[0];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::var.{var_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("variable"));

    // Extract type and default if present
    let body_text = node.utf8_text(source).unwrap_or("");
    if let Some(var_type) = extract_attribute_value(body_text, "type") {
        attrs.insert("var_type".to_string(), json!(var_type));
    }
    if let Some(default) = extract_attribute_value(body_text, "default") {
        attrs.insert("default".to_string(), json!(default));
    }
    if let Some(description) = extract_attribute_value(body_text, "description") {
        attrs.insert("description".to_string(), json!(description));
    }

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

// ─── Output Blocks ──────────────────────────────────────────────────────────

/// Extract an output block: `output "name" { value = ... }`
///
/// Produces a NodeKind::Constant with FQN: `file::output.name`
fn extract_output_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
) {
    let labels = extract_string_labels(text);
    if labels.is_empty() {
        return;
    }

    let output_name = &labels[0];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::output.{output_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("output"));
    attrs.insert("output".to_string(), json!(true));

    // Extract description if present
    let body_text = node.utf8_text(source).unwrap_or("");
    if let Some(description) = extract_attribute_value(body_text, "description") {
        attrs.insert("description".to_string(), json!(description));
    }
    if body_text.contains("sensitive") {
        attrs.insert("sensitive".to_string(), json!(true));
    }

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

// ─── Locals Blocks ──────────────────────────────────────────────────────────

/// Extract a locals block: `locals { key1 = value1 key2 = value2 ... }`
///
/// Produces a NodeKind::Constant for each key in the locals block.
/// FQN: `file::local.key`
fn extract_locals_block(node: tree_sitter::Node, file: &str, source: &[u8], nodes: &mut Vec<Node>) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let body_text = node.utf8_text(source).unwrap_or("");

    // Extract local value names from the block body.
    // Locals are defined as top-level attributes within the block:
    //   locals {
    //     name1 = expression
    //     name2 = expression
    //   }
    let local_names = extract_locals_keys(body_text);

    for local_name in &local_names {
        let fqn = format!("{file}::local.{local_name}");

        if nodes.iter().any(|n| n.fqn == fqn) {
            continue;
        }

        let mut attrs = serde_json::Map::new();
        attrs.insert("terraform_type".to_string(), json!("local"));

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

// ─── Provider Blocks ────────────────────────────────────────────────────────

/// Extract a provider block: `provider "name" { ... }`
///
/// Produces a NodeKind::Class with FQN: `file::provider.name`
fn extract_provider_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    text: &str,
    nodes: &mut Vec<Node>,
) {
    let labels = extract_string_labels(text);
    if labels.is_empty() {
        return;
    }

    let provider_name = &labels[0];

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::provider.{provider_name}");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("provider"));

    // Extract region or alias if present
    let body_text = node.utf8_text(source).unwrap_or("");
    if let Some(region) = extract_attribute_value(body_text, "region") {
        attrs.insert("region".to_string(), json!(region));
    }
    if let Some(alias) = extract_attribute_value(body_text, "alias") {
        attrs.insert("alias".to_string(), json!(alias));
    }

    nodes.push(Node {
        fqn,
        kind: NodeKind::Class,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
}

// ─── Terraform Settings Block ───────────────────────────────────────────────

/// Extract a terraform settings block: `terraform { ... }`
///
/// Produces a NodeKind::Module with FQN: `file::terraform`
fn extract_terraform_block(
    node: tree_sitter::Node,
    file: &str,
    source: &[u8],
    nodes: &mut Vec<Node>,
) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let fqn = format!("{file}::terraform");

    if nodes.iter().any(|n| n.fqn == fqn) {
        return;
    }

    let mut attrs = serde_json::Map::new();
    attrs.insert("terraform_type".to_string(), json!("terraform"));

    // Extract required_version if present
    let body_text = node.utf8_text(source).unwrap_or("");
    if let Some(version) = extract_attribute_value(body_text, "required_version") {
        attrs.insert("required_version".to_string(), json!(version));
    }

    nodes.push(Node {
        fqn,
        kind: NodeKind::Module,
        file: file.to_string(),
        start_line,
        end_line,
        file_hash: String::new(),
        indexed_at: 0,
        attributes: json!(attrs),
    });
}

// ─── Utility Functions ──────────────────────────────────────────────────────

/// Extract quoted string labels from block text.
///
/// Given `resource "aws_instance" "web" {`, returns `["aws_instance", "web"]`.
/// Given `module "vpc" {`, returns `["vpc"]`.
fn extract_string_labels(text: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();

    for ch in text.chars() {
        match ch {
            '"' => {
                if in_quote {
                    // End of quoted string
                    if !current.is_empty() {
                        labels.push(current.clone());
                        current.clear();
                    }
                    in_quote = false;
                } else {
                    in_quote = true;
                }
            }
            '{' if !in_quote => {
                // Stop at the opening brace
                break;
            }
            _ if in_quote => {
                current.push(ch);
            }
            _ => {}
        }
    }

    labels
}

/// Extract the value of a simple attribute from block body text.
///
/// Looks for patterns like `attribute_name = "value"` or `attribute_name = value`.
/// Returns the value as a string (without quotes for string values).
fn extract_attribute_value(body_text: &str, attr_name: &str) -> Option<String> {
    for line in body_text.lines() {
        let trimmed = line.trim();
        // Match: attr_name = "value" or attr_name = value
        if let Some(rest) = trimmed.strip_prefix(attr_name) {
            let rest = rest.trim_start();
            if let Some(after_eq) = rest.strip_prefix('=') {
                let value = after_eq.trim();
                // Remove surrounding quotes if present
                if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                    return Some(value[1..value.len() - 1].to_string());
                }
                // Return raw value (for non-string types like `string`, `number`, etc.)
                // Stop at end of line, trim trailing comments
                let value = value.split('#').next().unwrap_or(value).trim();
                if !value.is_empty() && value != "{" {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Extract local value keys from a locals block body.
///
/// Given:
/// ```hcl
/// locals {
///   name1 = "value1"
///   name2 = expression
/// }
/// ```
/// Returns `["name1", "name2"]`.
fn extract_locals_keys(body_text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_block = false;
    let mut brace_depth = 0;

    for line in body_text.lines() {
        let trimmed = line.trim();

        // Compute brace depth at the START of this line (before processing braces on this line)
        let depth_before = brace_depth;

        // Track brace depth
        for ch in trimmed.chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    if brace_depth == 1 {
                        in_block = true;
                    }
                }
                '}' => {
                    brace_depth -= 1;
                }
                _ => {}
            }
        }

        // Extract keys at depth 1 (directly inside the locals block).
        // Use depth_before == 1 so that lines like `key = {` are captured
        // (the opening brace on that line increases depth to 2, but the key is at depth 1).
        if in_block && depth_before == 1 && trimmed.contains('=') && !trimmed.starts_with('#') {
            // Extract the key name (identifier before the `=`)
            if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim();
                // Validate it looks like an identifier
                if !key.is_empty()
                    && key.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && key
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic() || c == '_')
                {
                    keys.push(key.to_string());
                }
            }
        }
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse HCL source and run the extractor.
    fn parse_and_extract(file: &str, source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_hcl::LANGUAGE.into())
            .expect("HCL grammar should load");
        let tree = parser.parse(source, None).expect("parse should succeed");
        extract(&tree, file, source)
    }

    #[test]
    fn test_extract_resource_blocks() {
        let source = r#"
resource "aws_instance" "web" {
  ami           = "ami-12345"
  instance_type = "t2.micro"

  tags = {
    Name = "web-server"
  }
}

resource "aws_security_group" "web_sg" {
  name = "web-sg"

  ingress {
    from_port = 80
    to_port   = 80
    protocol  = "tcp"
  }
}
"#;
        let result = parse_and_extract("infra/main.tf", source);

        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/main.tf::resource.aws_instance.web" && n.kind == NodeKind::Class
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/main.tf::resource.aws_security_group.web_sg"
                && n.kind == NodeKind::Class
        }));

        // Check attributes
        let web_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/main.tf::resource.aws_instance.web")
            .unwrap();
        assert_eq!(web_node.attributes["terraform_type"], "resource");
        assert_eq!(web_node.attributes["resource_type"], "aws_instance");
    }

    #[test]
    fn test_extract_data_blocks() {
        let source = r#"
data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"]

  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd/ubuntu-focal-20.04-amd64-server-*"]
  }
}

data "aws_vpc" "default" {
  default = true
}
"#;
        let result = parse_and_extract("infra/data.tf", source);

        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/data.tf::data.aws_ami.ubuntu" && n.kind == NodeKind::Class
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/data.tf::data.aws_vpc.default" && n.kind == NodeKind::Class
        }));

        let ami_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/data.tf::data.aws_ami.ubuntu")
            .unwrap();
        assert_eq!(ami_node.attributes["terraform_type"], "data");
        assert_eq!(ami_node.attributes["data_type"], "aws_ami");
    }

    #[test]
    fn test_extract_module_blocks_with_source_import() {
        let source = r#"
module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "3.0.0"

  name = "my-vpc"
  cidr = "10.0.0.0/16"
}

module "eks" {
  source = "./modules/eks"

  cluster_name = "my-cluster"
}
"#;
        let result = parse_and_extract("infra/modules.tf", source);

        // Check module nodes
        assert!(
            result
                .nodes
                .iter()
                .any(|n| { n.fqn == "infra/modules.tf::module.vpc" && n.kind == NodeKind::Module })
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| { n.fqn == "infra/modules.tf::module.eks" && n.kind == NodeKind::Module })
        );

        // Check import edges for module sources
        let imports: Vec<&Edge> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert!(imports.iter().any(|e| {
            e.source_fqn == "infra/modules.tf::module.vpc"
                && e.target_fqn == "terraform-aws-modules/vpc/aws"
        }));
        assert!(imports.iter().any(|e| {
            e.source_fqn == "infra/modules.tf::module.eks" && e.target_fqn == "./modules/eks"
        }));
    }

    #[test]
    fn test_extract_variable_blocks() {
        let source = r#"
variable "region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}

variable "instance_type" {
  description = "EC2 instance type"
  type        = string
}

variable "enable_monitoring" {
  type    = bool
  default = true
}
"#;
        let result = parse_and_extract("infra/variables.tf", source);

        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/variables.tf::var.region" && n.kind == NodeKind::Constant
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/variables.tf::var.instance_type" && n.kind == NodeKind::Constant
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/variables.tf::var.enable_monitoring" && n.kind == NodeKind::Constant
        }));

        // Check attributes
        let region_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/variables.tf::var.region")
            .unwrap();
        assert_eq!(region_node.attributes["terraform_type"], "variable");
    }

    #[test]
    fn test_extract_output_blocks() {
        let source = r#"
output "instance_ip" {
  description = "The public IP of the instance"
  value       = aws_instance.web.public_ip
}

output "vpc_id" {
  value     = module.vpc.vpc_id
  sensitive = true
}
"#;
        let result = parse_and_extract("infra/outputs.tf", source);

        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/outputs.tf::output.instance_ip" && n.kind == NodeKind::Constant
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/outputs.tf::output.vpc_id" && n.kind == NodeKind::Constant
        }));

        // Check output attribute
        let ip_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/outputs.tf::output.instance_ip")
            .unwrap();
        assert_eq!(ip_node.attributes["terraform_type"], "output");
        assert_eq!(ip_node.attributes["output"], true);
    }

    #[test]
    fn test_extract_locals_block() {
        let source = r#"
locals {
  common_tags = {
    Environment = "production"
    Team        = "platform"
  }
  name_prefix = "myapp"
  region      = var.region
}
"#;
        let result = parse_and_extract("infra/locals.tf", source);

        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/locals.tf::local.common_tags" && n.kind == NodeKind::Constant
        }));
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/locals.tf::local.name_prefix" && n.kind == NodeKind::Constant
        }));
        assert!(
            result.nodes.iter().any(|n| {
                n.fqn == "infra/locals.tf::local.region" && n.kind == NodeKind::Constant
            })
        );
    }

    #[test]
    fn test_extract_provider_block() {
        let source = r#"
provider "aws" {
  region = "us-east-1"
  alias  = "east"
}

provider "google" {
  project = "my-project"
  region  = "us-central1"
}
"#;
        let result = parse_and_extract("infra/providers.tf", source);

        assert!(
            result.nodes.iter().any(|n| {
                n.fqn == "infra/providers.tf::provider.aws" && n.kind == NodeKind::Class
            })
        );
        assert!(result.nodes.iter().any(|n| {
            n.fqn == "infra/providers.tf::provider.google" && n.kind == NodeKind::Class
        }));

        let aws_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/providers.tf::provider.aws")
            .unwrap();
        assert_eq!(aws_node.attributes["terraform_type"], "provider");
    }

    #[test]
    fn test_extract_terraform_settings_block() {
        let source = r#"
terraform {
  required_version = ">= 1.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 4.0"
    }
  }

  backend "s3" {
    bucket = "my-terraform-state"
    key    = "state.tfstate"
    region = "us-east-1"
  }
}
"#;
        let result = parse_and_extract("infra/versions.tf", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| { n.fqn == "infra/versions.tf::terraform" && n.kind == NodeKind::Module })
        );

        let tf_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/versions.tf::terraform")
            .unwrap();
        assert_eq!(tf_node.attributes["terraform_type"], "terraform");
    }

    #[test]
    fn test_extract_resource_with_count() {
        let source = r#"
resource "aws_instance" "cluster" {
  count         = 3
  ami           = "ami-12345"
  instance_type = "t2.micro"
}
"#;
        let result = parse_and_extract("infra/cluster.tf", source);

        let node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/cluster.tf::resource.aws_instance.cluster")
            .unwrap();
        assert_eq!(node.attributes["has_count"], true);
    }

    #[test]
    fn test_extract_resource_with_for_each() {
        let source = r#"
resource "aws_iam_user" "users" {
  for_each = toset(["alice", "bob", "charlie"])
  name     = each.key
}
"#;
        let result = parse_and_extract("infra/users.tf", source);

        let node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "infra/users.tf::resource.aws_iam_user.users")
            .unwrap();
        assert_eq!(node.attributes["has_for_each"], true);
    }

    #[test]
    fn test_empty_file() {
        let result = parse_and_extract("empty.tf", "");
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_comprehensive_terraform_file() {
        let source = r#"
terraform {
  required_version = ">= 1.0"
}

provider "aws" {
  region = "us-east-1"
}

variable "region" {
  description = "AWS region"
  default     = "us-east-1"
}

variable "instance_type" {
  description = "EC2 instance type"
  type        = string
}

data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"]
}

resource "aws_instance" "web" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = var.instance_type

  tags = {
    Name = "web-server"
  }
}

resource "aws_security_group" "web_sg" {
  name = "web-sg"
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "3.0.0"
}

output "instance_ip" {
  value = aws_instance.web.public_ip
}

locals {
  env = "production"
}
"#;
        let result = parse_and_extract("infra/main.tf", source);

        // Verify all construct types are extracted
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::terraform")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::provider.aws")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::var.region")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::var.instance_type")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::data.aws_ami.ubuntu")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::resource.aws_instance.web")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::resource.aws_security_group.web_sg")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::module.vpc")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::output.instance_ip")
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "infra/main.tf::local.env")
        );

        // Verify module import edge
        assert!(result.edges.iter().any(|e| {
            e.kind == EdgeKind::Imports && e.target_fqn == "terraform-aws-modules/vpc/aws"
        }));
    }

    #[test]
    fn test_extract_regex_backward_compat() {
        let source = r#"
resource "aws_instance" "web" {
  ami = "ami-12345"
}

module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
}
"#;
        #[allow(deprecated)]
        let result = extract_regex("main.tf", source);

        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.fqn == "main.tf::resource.aws_instance.web")
        );
        assert!(result.nodes.iter().any(|n| n.fqn == "main.tf::module.vpc"));
        assert!(
            result
                .edges
                .iter()
                .any(|e| e.target_fqn == "terraform-aws-modules/vpc/aws")
        );
    }

    #[test]
    fn test_line_numbers_are_accurate() {
        let source = r#"variable "name" {
  type = string
}

resource "aws_instance" "web" {
  ami = "ami-12345"
}
"#;
        let result = parse_and_extract("test.tf", source);

        let var_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.tf::var.name")
            .unwrap();
        assert_eq!(var_node.start_line, 1);
        assert_eq!(var_node.end_line, 3);

        let res_node = result
            .nodes
            .iter()
            .find(|n| n.fqn == "test.tf::resource.aws_instance.web")
            .unwrap();
        assert_eq!(res_node.start_line, 5);
        assert_eq!(res_node.end_line, 7);
    }
}
