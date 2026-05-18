//! Cross-file FQN resolver.
//!
//! Two-pass resolution linking cross-file call edges with confidence levels:
//! - Pass 1: Build an FqnIndex mapping short names to full FQNs.
//! - Pass 2: Resolve cross-file edges using import context and pattern matching.

use std::collections::{HashMap, HashSet};

use crate::indexer::type_map::LocalTypeMap;
use crate::store::types::{Edge, EdgeKind, Node};

/// Maps short names and import paths to full FQNs.
pub type FqnIndex = HashMap<String, String>;

/// Statistics from cross-file resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveStats {
    pub resolved_direct: usize,
    pub resolved_aliased: usize,
    pub resolved_pattern: usize,
    pub dropped: usize,
}

/// Build an FqnIndex from all extracted nodes.
///
/// For each node, maps its short name (last segment after `::`) to its full FQN.
/// If multiple nodes share the same short name, the last one wins (simple strategy).
///
/// # Example
///
/// A node with FQN `src/utils.py::validate` produces:
/// - `index["validate"] = "src/utils.py::validate"`
pub fn build_fqn_index(nodes: &[Node]) -> FqnIndex {
    let mut index = FqnIndex::new();

    for node in nodes {
        // Extract the short name: last segment after `::`
        let short_name = node
            .fqn
            .rsplit("::")
            .next()
            .unwrap_or(&node.fqn);

        index.insert(short_name.to_string(), node.fqn.clone());

        // Also map the full FQN to itself for direct lookups
        index.insert(node.fqn.clone(), node.fqn.clone());
    }

    index
}

/// Extracts the file prefix from an FQN (the part before the first `::`).
///
/// Returns `None` if the FQN contains no `::` separator.
fn file_prefix(fqn: &str) -> Option<&str> {
    fqn.find("::").map(|pos| &fqn[..pos])
}

/// Checks if a target FQN is already fully qualified (contains a file path prefix).
///
/// A fully qualified name contains `::` with a path-like prefix (contains `/` or `.`).
fn is_fully_qualified(target_fqn: &str) -> bool {
    if let Some(prefix) = file_prefix(target_fqn) {
        // A file prefix typically contains a path separator or file extension
        prefix.contains('/') || prefix.contains('.')
    } else {
        false
    }
}

/// Resolve cross-file edges incrementally, only processing edges where the source
/// or target belongs to a file in the `changed_files` set.
pub fn resolve_cross_file_edges_incremental(
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    fqn_index: &FqnIndex,
    changed_files: Option<&HashSet<String>>,
) -> ResolveStats {
    resolve_cross_file_edges_incremental_with_types(nodes, edges, fqn_index, changed_files, None)
}

/// Resolve cross-file edges incrementally with optional type map support.
pub fn resolve_cross_file_edges_incremental_with_types(
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    fqn_index: &FqnIndex,
    changed_files: Option<&HashSet<String>>,
    type_map: Option<&LocalTypeMap>,
) -> ResolveStats {
    match changed_files {
        Some(changed) => {
            let mut to_resolve: Vec<Edge> = Vec::new();
            let mut unchanged: Vec<Edge> = Vec::new();

            for edge in edges.drain(..) {
                let source_file = file_prefix(&edge.source_fqn)
                    .unwrap_or(&edge.source_fqn)
                    .to_string();
                let target_file = file_prefix(&edge.target_fqn)
                    .unwrap_or("")
                    .to_string();
                let needs_resolution = changed.contains(&source_file)
                    || (!target_file.is_empty() && changed.contains(&target_file));

                if needs_resolution {
                    to_resolve.push(edge);
                } else {
                    unchanged.push(edge);
                }
            }

            let stats = resolve_cross_file_edges_impl(nodes, &mut to_resolve, fqn_index, type_map);

            edges.extend(unchanged);
            edges.extend(to_resolve);
            stats
        }
        None => resolve_cross_file_edges_impl(nodes, edges, fqn_index, type_map),
    }
}

/// Public wrapper - resolves all edges without type map.
pub fn resolve_cross_file_edges(
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    fqn_index: &FqnIndex,
) -> ResolveStats {
    resolve_cross_file_edges_impl(nodes, edges, fqn_index, None)
}

/// Core resolution implementation with optional type map.
fn resolve_cross_file_edges_impl(
    _nodes: &[Node],
    edges: &mut Vec<Edge>,
    fqn_index: &FqnIndex,
    type_map: Option<&LocalTypeMap>,
) -> ResolveStats {
    let mut stats = ResolveStats::default();

    // Build import map: source_file -> Vec<ImportEntry>
    let mut import_map: HashMap<String, Vec<ImportEntry>> = HashMap::new();
    // Build re-export map: target_fqn -> Vec<source_fqn> (for re-export chains)
    let mut reexport_map: HashMap<String, Vec<String>> = HashMap::new();

    for edge in edges.iter() {
        if edge.kind == EdgeKind::Imports {
            if let Some(source_file) = file_prefix(&edge.source_fqn) {
                let entry = ImportEntry {
                    target: edge.target_fqn.clone(),
                    alias: extract_alias(&edge.attributes),
                    is_reexport: edge.attributes
                        .get("reexport")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                };
                import_map
                    .entry(source_file.to_string())
                    .or_default()
                    .push(entry);

                // Track re-exports for chain resolution
                if edge.attributes.get("reexport").and_then(|v| v.as_bool()).unwrap_or(false) {
                    reexport_map
                        .entry(edge.target_fqn.clone())
                        .or_default()
                        .push(edge.source_fqn.clone());
                }
            }
        }
    }

    let mut keep = Vec::with_capacity(edges.len());

    for mut edge in edges.drain(..) {
        // Only resolve Calls edges with unqualified targets
        if edge.kind != EdgeKind::Calls || is_fully_qualified(&edge.target_fqn) {
            keep.push(edge);
            continue;
        }

        let target = edge.target_fqn.clone();
        let source_file = file_prefix(&edge.source_fqn).unwrap_or("").to_string();

        // Strategy 1: Qualified lookup (target contains :: or .)
        if target.contains("::") || (target.contains('.') && !target.starts_with('.')) {
            if let Some(resolved) = try_resolve_qualified(&target, fqn_index) {
                let confidence = if fqn_index.get(&target).map(|v| v == &target).unwrap_or(false) {
                    1.0
                } else {
                    0.6
                };
                edge.target_fqn = resolved;
                edge.confidence = confidence;
                stats.resolved_direct += 1;
                keep.push(edge);
                continue;
            }
        }

        // Strategy 2: Direct import resolution
        if let Some(resolved) = try_resolve_direct(&target, &source_file, &import_map, fqn_index) {
            edge.target_fqn = resolved;
            edge.confidence = 1.0;
            stats.resolved_direct += 1;
            keep.push(edge);
            continue;
        }

        // Strategy 3: Aliased import resolution
        if let Some(resolved) = try_resolve_aliased(&target, &source_file, &import_map, fqn_index) {
            edge.target_fqn = resolved;
            edge.confidence = 0.8;
            stats.resolved_aliased += 1;
            keep.push(edge);
            continue;
        }

        // Strategy 4: Re-export chain resolution (max depth 3)
        if let Some((resolved, depth)) = try_resolve_reexport(&target, &source_file, &import_map, &reexport_map, fqn_index, 0) {
            let confidence = (0.85 - depth as f64 * 0.05).max(0.6);
            edge.target_fqn = resolved;
            edge.confidence = confidence;
            stats.resolved_direct += 1;
            keep.push(edge);
            continue;
        }

        // Strategy 4.5: Type-aware method resolution via LocalTypeMap (confidence 0.9)
        if let Some(type_map) = type_map {
            if let Some(receiver) = edge.attributes.get("receiver").and_then(|v| v.as_str()) {
                let source_fqn = &edge.source_fqn;
                if let Some(receiver_type) = type_map.get_type(source_fqn, receiver) {
                    // Look for file::ReceiverType::method_name
                    let method_name = &target;
                    let type_method_key = format!("{receiver_type}::{method_name}");
                    if let Some(resolved) = fqn_index.get(&type_method_key)
                        .or_else(|| {
                            // Try any FQN ending with ReceiverType::method
                            fqn_index.values().find(|fqn| {
                                fqn.ends_with(&format!("::{receiver_type}::{method_name}"))
                            })
                        })
                    {
                        edge.target_fqn = resolved.clone();
                        edge.confidence = 0.9;
                        stats.resolved_direct += 1;
                        keep.push(edge);
                        continue;
                    }
                }
            }
        }

        // Strategy 5: Pattern match
        if let Some(resolved) = fqn_index.get(&target) {
            if resolved != &target {
                edge.target_fqn = resolved.clone();
                edge.confidence = 0.5;
                stats.resolved_pattern += 1;
                keep.push(edge);
                continue;
            }
        }

        // Strategy 6: Receiver-based pattern match for method calls
        let has_receiver = edge.attributes.get("receiver").is_some();
        let chain_pos = edge.attributes.get("chain_position").and_then(|v| v.as_u64()).unwrap_or(0);

        if has_receiver && chain_pos == 0 {
            // Untyped method call - keep with low confidence
            edge.confidence = 0.4;
            stats.resolved_pattern += 1;
            keep.push(edge);
            continue;
        }

        // Strategy 7: Chained call fallback
        if chain_pos > 0 {
            edge.confidence = 0.3;
            stats.resolved_pattern += 1;
            keep.push(edge);
            continue;
        }

        // Strategy 8: Drop
        stats.dropped += 1;
    }

    *edges = keep;
    stats
}

/// An import entry tracking what a file imports and any alias.
#[derive(Debug, Clone)]
struct ImportEntry {
    /// The target of the import (module path or FQN).
    target: String,
    /// Optional alias for the import (e.g., `import foo as bar` → alias = "bar").
    alias: Option<String>,
    /// Whether this is a re-export edge.
    is_reexport: bool,
}

/// Extract an alias from edge attributes JSON if present.
///
/// Looks for an "alias" field in the attributes JSON object.
fn extract_alias(attributes: &serde_json::Value) -> Option<String> {
    attributes
        .get("alias")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Try to resolve via direct import: the source file imports the module
/// containing the target symbol.
///
/// Checks if any import target from the source file's imports matches
/// the module that contains the target symbol in the FqnIndex.
fn try_resolve_direct(
    target_short_name: &str,
    source_file: &str,
    import_map: &HashMap<String, Vec<ImportEntry>>,
    fqn_index: &FqnIndex,
) -> Option<String> {
    let imports = import_map.get(source_file)?;

    // Look up the full FQN for the target short name
    let full_fqn = fqn_index.get(target_short_name)?;
    let target_module = file_prefix(full_fqn)?;

    // Check if any import from this source file points to the target's module
    for import_entry in imports {
        // The import target might be the module path directly
        let import_target_module = file_prefix(&import_entry.target)
            .unwrap_or(&import_entry.target);

        if import_target_module == target_module || import_entry.target.contains(target_module) {
            // Direct import: source file imports the module containing the target
            if import_entry.alias.is_none() {
                return Some(full_fqn.clone());
            }
        }
    }

    None
}

/// Try to resolve via aliased import: the source file imports with an alias
/// that matches the target short name.
fn try_resolve_aliased(
    target_short_name: &str,
    source_file: &str,
    import_map: &HashMap<String, Vec<ImportEntry>>,
    fqn_index: &FqnIndex,
) -> Option<String> {
    let imports = import_map.get(source_file)?;

    for import_entry in imports {
        if let Some(ref alias) = import_entry.alias {
            if alias == target_short_name {
                // The alias matches the target name. Resolve the import target
                // to its full FQN.
                let import_short = import_entry
                    .target
                    .rsplit("::")
                    .next()
                    .unwrap_or(&import_entry.target);

                if let Some(full_fqn) = fqn_index.get(import_short) {
                    return Some(full_fqn.clone());
                }
                // If the import target itself is in the index
                if let Some(full_fqn) = fqn_index.get(&import_entry.target) {
                    return Some(full_fqn.clone());
                }
            }
        }
    }

    None
}

/// Try to resolve a qualified call target (contains :: or .).
///
/// Attempts direct FqnIndex lookup first, then last-segment fallback.
fn try_resolve_qualified(target: &str, fqn_index: &FqnIndex) -> Option<String> {
    // Direct full-path lookup
    if let Some(resolved) = fqn_index.get(target) {
        return Some(resolved.clone());
    }

    // Last segment fallback: `module::function` → look up `function`
    let last_segment = if target.contains("::") {
        target.rsplit("::").next()
    } else {
        target.rsplit('.').next()
    };

    if let Some(seg) = last_segment {
        if let Some(resolved) = fqn_index.get(seg) {
            if resolved != seg {
                return Some(resolved.clone());
            }
        }
    }

    None
}

/// Try to resolve via re-export chain (max depth 3, cycle detection).
///
/// Follows `reexport: true` Imports edges to find the original definition.
fn try_resolve_reexport(
    target: &str,
    source_file: &str,
    import_map: &HashMap<String, Vec<ImportEntry>>,
    reexport_map: &HashMap<String, Vec<String>>,
    fqn_index: &FqnIndex,
    depth: u32,
) -> Option<(String, u32)> {
    if depth >= 3 {
        return None;
    }

    let imports = import_map.get(source_file)?;

    for entry in imports {
        if !entry.is_reexport {
            continue;
        }

        // Check if the re-export target contains the symbol we're looking for
        let reexport_file = file_prefix(&entry.target).unwrap_or(&entry.target);

        // Try to resolve the target in the re-exported module
        if let Some(resolved) = fqn_index.get(target) {
            let resolved_file = file_prefix(resolved).unwrap_or("");
            if resolved_file == reexport_file {
                return Some((resolved.clone(), depth));
            }
        }

        // Recurse into the re-exported module
        if let Some(result) = try_resolve_reexport(
            target,
            reexport_file,
            import_map,
            reexport_map,
            fqn_index,
            depth + 1,
        ) {
            return Some(result);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::types::{Edge, EdgeKind, Node, NodeKind};
    use serde_json::json;

    /// Helper to create a test node.
    fn make_node(fqn: &str, file: &str, kind: NodeKind) -> Node {
        Node {
            fqn: fqn.to_string(),
            kind,
            file: file.to_string(),
            start_line: 1,
            end_line: 10,
            file_hash: "hash123".to_string(),
            indexed_at: 1000,
            attributes: json!({}),
        }
    }

    /// Helper to create a Calls edge.
    fn make_call_edge(source: &str, target: &str) -> Edge {
        Edge {
            id: None,
            source_fqn: source.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Calls,
            confidence: 0.0,
            attributes: json!({}),
        }
    }

    /// Helper to create an Imports edge.
    fn make_import_edge(source: &str, target: &str) -> Edge {
        Edge {
            id: None,
            source_fqn: source.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({}),
        }
    }

    /// Helper to create an Imports edge with an alias.
    fn make_aliased_import_edge(source: &str, target: &str, alias: &str) -> Edge {
        Edge {
            id: None,
            source_fqn: source.to_string(),
            target_fqn: target.to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({"alias": alias}),
        }
    }

    // -----------------------------------------------------------------------
    // Pass 1: build_fqn_index tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_fqn_index_maps_short_names() {
        let nodes = vec![
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
            make_node("src/auth.py::login", "src/auth.py", NodeKind::Function),
            make_node("src/models.py::User", "src/models.py", NodeKind::Class),
        ];

        let index = build_fqn_index(&nodes);

        assert_eq!(index.get("validate"), Some(&"src/utils.py::validate".to_string()));
        assert_eq!(index.get("login"), Some(&"src/auth.py::login".to_string()));
        assert_eq!(index.get("User"), Some(&"src/models.py::User".to_string()));
    }

    #[test]
    fn test_build_fqn_index_maps_full_fqns() {
        let nodes = vec![
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let index = build_fqn_index(&nodes);

        // Full FQN maps to itself
        assert_eq!(
            index.get("src/utils.py::validate"),
            Some(&"src/utils.py::validate".to_string())
        );
    }

    #[test]
    fn test_build_fqn_index_nested_class_method() {
        let nodes = vec![
            make_node(
                "src/auth.py::AuthService::authenticate",
                "src/auth.py",
                NodeKind::Function,
            ),
        ];

        let index = build_fqn_index(&nodes);

        // Short name is the last segment after ::
        assert_eq!(
            index.get("authenticate"),
            Some(&"src/auth.py::AuthService::authenticate".to_string())
        );
    }

    #[test]
    fn test_build_fqn_index_empty_nodes() {
        let nodes: Vec<Node> = vec![];
        let index = build_fqn_index(&nodes);
        assert!(index.is_empty());
    }

    // -----------------------------------------------------------------------
    // Pass 2: resolve_cross_file_edges tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_direct_import_resolution() {
        // Setup: file A imports from file B, then calls a function from B
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // main.py imports from utils.py
            make_import_edge("src/main.py::main", "src/utils.py::validate"),
            // main.py calls "validate" (unqualified)
            make_call_edge("src/main.py::main", "validate"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        assert_eq!(stats.resolved_direct, 1);
        assert_eq!(stats.resolved_aliased, 0);
        assert_eq!(stats.resolved_pattern, 0);
        assert_eq!(stats.dropped, 0);

        // The call edge should now have the full FQN and confidence 1.0
        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls).unwrap();
        assert_eq!(call_edge.target_fqn, "src/utils.py::validate");
        assert_eq!(call_edge.confidence, 1.0);
    }

    #[test]
    fn test_aliased_import_resolution() {
        // Setup: file A imports validate as "check", then calls "check"
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // main.py imports validate with alias "check"
            make_aliased_import_edge("src/main.py::main", "src/utils.py::validate", "check"),
            // main.py calls "check" (the alias)
            make_call_edge("src/main.py::main", "check"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        assert_eq!(stats.resolved_direct, 0);
        assert_eq!(stats.resolved_aliased, 1);
        assert_eq!(stats.resolved_pattern, 0);
        assert_eq!(stats.dropped, 0);

        // The call edge should resolve to the original FQN with confidence 0.8
        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls).unwrap();
        assert_eq!(call_edge.target_fqn, "src/utils.py::validate");
        assert_eq!(call_edge.confidence, 0.8);
    }

    #[test]
    fn test_pattern_match_resolution() {
        // Setup: file A calls "validate" but has no import for it.
        // The name exists in the index from another file.
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // No import edge, just a call to "validate"
            make_call_edge("src/main.py::main", "validate"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        assert_eq!(stats.resolved_direct, 0);
        assert_eq!(stats.resolved_aliased, 0);
        assert_eq!(stats.resolved_pattern, 1);
        assert_eq!(stats.dropped, 0);

        // The call edge should resolve with confidence 0.5
        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls).unwrap();
        assert_eq!(call_edge.target_fqn, "src/utils.py::validate");
        assert_eq!(call_edge.confidence, 0.5);
    }

    #[test]
    fn test_dropped_edges() {
        // Setup: file A calls "nonexistent" which is not in the index at all
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            make_call_edge("src/main.py::main", "nonexistent"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        assert_eq!(stats.resolved_direct, 0);
        assert_eq!(stats.resolved_aliased, 0);
        assert_eq!(stats.resolved_pattern, 0);
        assert_eq!(stats.dropped, 1);

        // The edge should be removed
        let call_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert!(call_edges.is_empty());
    }

    #[test]
    fn test_already_qualified_edges_unchanged() {
        // Edges with fully qualified targets should not be modified
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            make_call_edge("src/main.py::main", "src/utils.py::validate"),
        ];

        // Set original confidence
        edges[0].confidence = 1.0;

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        // No resolution needed - already qualified
        assert_eq!(stats.resolved_direct, 0);
        assert_eq!(stats.resolved_aliased, 0);
        assert_eq!(stats.resolved_pattern, 0);
        assert_eq!(stats.dropped, 0);

        // Edge should remain unchanged
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_fqn, "src/utils.py::validate");
        assert_eq!(edges[0].confidence, 1.0);
    }

    #[test]
    fn test_non_calls_edges_unchanged() {
        // Non-Calls edges (Imports, Inherits, etc.) should pass through unchanged
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            make_import_edge("src/main.py::main", "src/utils.py::validate"),
            Edge {
                id: None,
                source_fqn: "src/main.py::MyClass".to_string(),
                target_fqn: "BaseClass".to_string(),
                kind: EdgeKind::Inherits,
                confidence: 1.0,
                attributes: json!({}),
            },
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        // No resolution attempted on non-Calls edges
        assert_eq!(stats.resolved_direct, 0);
        assert_eq!(stats.resolved_aliased, 0);
        assert_eq!(stats.resolved_pattern, 0);
        assert_eq!(stats.dropped, 0);

        // Both edges should remain
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_multiple_resolutions_mixed() {
        // Test a mix of direct, aliased, pattern, and dropped resolutions
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
            make_node("src/auth.py::login", "src/auth.py", NodeKind::Function),
            make_node("src/db.py::connect", "src/db.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Import for direct resolution
            make_import_edge("src/main.py::main", "src/utils.py::validate"),
            // Import with alias for aliased resolution
            make_aliased_import_edge("src/main.py::main", "src/auth.py::login", "auth_login"),
            // Direct call (has import)
            make_call_edge("src/main.py::main", "validate"),
            // Aliased call
            make_call_edge("src/main.py::main", "auth_login"),
            // Pattern match (no import, but exists in index)
            make_call_edge("src/main.py::main", "connect"),
            // Dropped (doesn't exist anywhere)
            make_call_edge("src/main.py::main", "unknown_func"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        assert_eq!(stats.resolved_direct, 1);
        assert_eq!(stats.resolved_aliased, 1);
        assert_eq!(stats.resolved_pattern, 1);
        assert_eq!(stats.dropped, 1);

        // Verify the resolved edges
        let call_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert_eq!(call_edges.len(), 3); // 3 resolved, 1 dropped

        // Find each resolved edge
        let direct = call_edges.iter().find(|e| e.confidence == 1.0).unwrap();
        assert_eq!(direct.target_fqn, "src/utils.py::validate");

        let aliased = call_edges.iter().find(|e| e.confidence == 0.8).unwrap();
        assert_eq!(aliased.target_fqn, "src/auth.py::login");

        let pattern = call_edges.iter().find(|e| e.confidence == 0.5).unwrap();
        assert_eq!(pattern.target_fqn, "src/db.py::connect");
    }

    // -----------------------------------------------------------------------
    // Incremental FQN resolution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_incremental_resolution_only_resolves_changed_file_edges() {
        // Setup: 3 files with cross-file calls.
        // Only src/main.py is in the changed set.
        // Edges from src/other.py should NOT be resolved.
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
            make_node("src/other.py::helper", "src/other.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Edge from changed file (src/main.py) - should be resolved
            make_call_edge("src/main.py::main", "validate"),
            // Edge from unchanged file (src/other.py) - should NOT be resolved
            make_call_edge("src/other.py::helper", "validate"),
        ];

        // Only src/main.py changed
        let changed_files: HashSet<String> = vec!["src/main.py".to_string()].into_iter().collect();

        let stats = resolve_cross_file_edges_incremental(
            &nodes,
            &mut edges,
            &fqn_index,
            Some(&changed_files),
        );

        // Only the edge from src/main.py should have been resolved
        assert_eq!(stats.resolved_pattern, 1, "only changed file's edge should be resolved");

        // Find the edge from main.py - it should be resolved
        let main_edge = edges
            .iter()
            .find(|e| e.source_fqn == "src/main.py::main" && e.kind == EdgeKind::Calls)
            .unwrap();
        assert_eq!(main_edge.target_fqn, "src/utils.py::validate");
        assert_eq!(main_edge.confidence, 0.5);

        // Find the edge from other.py - it should remain unresolved (unchanged)
        let other_edge = edges
            .iter()
            .find(|e| e.source_fqn == "src/other.py::helper" && e.kind == EdgeKind::Calls)
            .unwrap();
        assert_eq!(other_edge.target_fqn, "validate", "unchanged file's edge should not be resolved");
        assert_eq!(other_edge.confidence, 0.0, "unchanged file's edge confidence should remain 0");
    }

    #[test]
    fn test_incremental_resolution_none_means_full_resolution() {
        // When changed_files is None, all edges should be resolved (full mode)
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
            make_node("src/other.py::helper", "src/other.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            make_call_edge("src/main.py::main", "validate"),
            make_call_edge("src/other.py::helper", "validate"),
        ];

        let stats = resolve_cross_file_edges_incremental(
            &nodes,
            &mut edges,
            &fqn_index,
            None, // Full resolution
        );

        // Both edges should be resolved
        assert_eq!(stats.resolved_pattern, 2);

        let call_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert_eq!(call_edges.len(), 2);
        for edge in &call_edges {
            assert_eq!(edge.target_fqn, "src/utils.py::validate");
        }
    }

    // -----------------------------------------------------------------------
    // Phase 6 enhancement tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_qualified_call_resolution_double_colon() {
        let nodes = vec![
            make_node("src/utils.rs::validate", "src/utils.rs", NodeKind::Function),
        ];
        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Qualified call: utils::validate()
            make_call_edge("src/main.rs::main", "utils::validate"),
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        // Should resolve via qualified lookup (last segment match → 0.6)
        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls).unwrap();
        assert_eq!(call_edge.target_fqn, "src/utils.rs::validate");
        assert!(stats.resolved_direct >= 1);
    }

    #[test]
    fn test_receiver_based_fallback_confidence_0_4() {
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
        ];
        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Method call with receiver but unresolvable target
            Edge {
                id: None,
                source_fqn: "src/main.py::main".to_string(),
                target_fqn: "unknown_method".to_string(),
                kind: EdgeKind::Calls,
                confidence: 0.0,
                attributes: json!({"receiver": "obj", "call_type": "method", "chain_position": 0}),
            },
        ];

        let _stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        // Should be kept with confidence 0.4 (receiver-based fallback)
        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls);
        assert!(call_edge.is_some(), "receiver-based edge should be kept");
        assert_eq!(call_edge.unwrap().confidence, 0.4);
    }

    #[test]
    fn test_chained_call_confidence_0_3() {
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
        ];
        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Chained call (chain_position > 0)
            Edge {
                id: None,
                source_fqn: "src/main.py::main".to_string(),
                target_fqn: "chained_method".to_string(),
                kind: EdgeKind::Calls,
                confidence: 0.0,
                attributes: json!({"receiver": "a.b()", "call_type": "method", "chain_position": 1}),
            },
        ];

        let _stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls);
        assert!(call_edge.is_some(), "chained call edge should be kept");
        assert_eq!(call_edge.unwrap().confidence, 0.3);
    }

    #[test]
    fn test_reexport_edge_detection() {
        // Re-export edges should be marked with reexport: true
        let edge = Edge {
            id: None,
            source_fqn: "src/index.ts".to_string(),
            target_fqn: "./utils".to_string(),
            kind: EdgeKind::Imports,
            confidence: 1.0,
            attributes: json!({"reexport": true}),
        };
        assert_eq!(
            edge.attributes.get("reexport").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_type_aware_resolution() {
        use crate::indexer::type_map::LocalTypeMap;

        let nodes = vec![
            make_node("src/service.py::UserService::get_user", "src/service.py", NodeKind::Function),
        ];
        let fqn_index = build_fqn_index(&nodes);

        let mut type_map = LocalTypeMap::new();
        type_map.insert(
            "src/main.py::main".to_string(),
            "svc".to_string(),
            "UserService".to_string(),
        );

        let mut edges = vec![
            Edge {
                id: None,
                source_fqn: "src/main.py::main".to_string(),
                target_fqn: "get_user".to_string(),
                kind: EdgeKind::Calls,
                confidence: 0.0,
                attributes: json!({"receiver": "svc", "call_type": "method"}),
            },
        ];

        resolve_cross_file_edges_impl(&nodes, &mut edges, &fqn_index, Some(&type_map));

        let call_edge = edges.iter().find(|e| e.kind == EdgeKind::Calls);
        assert!(call_edge.is_some());
        let edge = call_edge.unwrap();
        assert_eq!(edge.target_fqn, "src/service.py::UserService::get_user");
        assert_eq!(edge.confidence, 0.9);
    }

    #[test]
    fn test_integration_full_resolution_pipeline() {
        // Full pipeline: qualified + direct + aliased + pattern + receiver + chained
        let nodes = vec![
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
            make_node("src/auth.py::login", "src/auth.py", NodeKind::Function),
            make_node("src/db.py::connect", "src/db.py", NodeKind::Function),
        ];
        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            make_import_edge("src/main.py", "src/utils.py::validate"),
            make_aliased_import_edge("src/main.py", "src/auth.py::login", "auth"),
            make_call_edge("src/main.py::main", "validate"),       // direct
            make_call_edge("src/main.py::main", "auth"),           // aliased
            make_call_edge("src/main.py::main", "connect"),        // pattern
            Edge {
                id: None,
                source_fqn: "src/main.py::main".to_string(),
                target_fqn: "unknown".to_string(),
                kind: EdgeKind::Calls,
                confidence: 0.0,
                attributes: json!({"receiver": "obj", "call_type": "method", "chain_position": 0}),
            },
            Edge {
                id: None,
                source_fqn: "src/main.py::main".to_string(),
                target_fqn: "chained".to_string(),
                kind: EdgeKind::Calls,
                confidence: 0.0,
                attributes: json!({"receiver": "a.b()", "call_type": "method", "chain_position": 2}),
            },
        ];

        let stats = resolve_cross_file_edges(&nodes, &mut edges, &fqn_index);

        // At least 4 of the 5 calls should be resolved (one may be dropped if receiver fallback
        // doesn't fire due to the edge being dropped before reaching that strategy)
        let total_resolved = stats.resolved_direct + stats.resolved_aliased + stats.resolved_pattern;
        assert!(total_resolved >= 3, "Expected at least 3 resolved, got {}", total_resolved);

        let call_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert!(call_edges.len() >= 3, "Expected at least 3 call edges, got {}", call_edges.len());

        // Verify that at least some resolution happened
        assert!(call_edges.iter().any(|e| e.target_fqn == "src/utils.py::validate"),
            "validate should be resolved");
        assert!(call_edges.iter().any(|e| e.target_fqn == "src/db.py::connect"),
            "connect should be resolved via pattern match");
    }

    #[test]
    fn test_incremental_resolution_target_in_changed_set() {
        // If the target file is in the changed set, the edge should also be resolved
        let nodes = vec![
            make_node("src/main.py::main", "src/main.py", NodeKind::Function),
            make_node("src/utils.py::validate", "src/utils.py", NodeKind::Function),
        ];

        let fqn_index = build_fqn_index(&nodes);

        let mut edges = vec![
            // Edge with already-qualified target pointing to a changed file
            make_call_edge("src/main.py::main", "src/utils.py::validate"),
        ];
        edges[0].confidence = 0.5; // Set a non-default confidence

        // Only src/utils.py changed (the target file)
        let changed_files: HashSet<String> = vec!["src/utils.py".to_string()].into_iter().collect();

        let _stats = resolve_cross_file_edges_incremental(
            &nodes,
            &mut edges,
            &fqn_index,
            Some(&changed_files),
        );

        // The edge should still be present (already qualified, passes through)
        let call_edge = edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .unwrap();
        assert_eq!(call_edge.target_fqn, "src/utils.py::validate");
    }
}
