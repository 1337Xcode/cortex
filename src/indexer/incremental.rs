//! Tree-sitter incremental re-parse with micro-delta application.
//!
//! Caches previous parse trees per file and uses tree-sitter's incremental
//! parsing API to efficiently re-parse only changed portions of a file.
//! Diffs old vs new extraction results to produce a micro-delta containing
//! only the symbols that actually changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tree_sitter::{InputEdit, Parser, Point, Tree};

use crate::indexer::languages;
use crate::indexer::parser::SupportedLanguage;
use crate::store::types::{Edge, ExtractionResult, Node};

/// A cached entry for a previously parsed file: the tree and the source text.
#[derive(Clone)]
struct CacheEntry {
    tree: Tree,
    source: String,
    language: SupportedLanguage,
}

/// Caches previous tree-sitter parse trees per file for incremental re-parsing.
///
/// When a file changes, the cached tree is used with `tree.edit()` and
/// `parser.parse(new_source, Some(&old_tree))` to achieve O(changed nodes)
/// re-parsing instead of O(file).
pub struct TreeCache {
    entries: HashMap<PathBuf, CacheEntry>,
}

impl TreeCache {
    /// Create a new empty tree cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Store a parsed tree and source for a file.
    pub fn insert(&mut self, path: PathBuf, tree: Tree, source: String, language: SupportedLanguage) {
        self.entries.insert(path, CacheEntry { tree, source, language });
    }

    /// Check if a file has a cached tree.
    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    /// Remove a file from the cache (e.g., on deletion).
    pub fn remove(&mut self, path: &Path) {
        self.entries.remove(path);
    }

    /// Get the cached source for a file.
    pub fn get_source(&self, path: &Path) -> Option<&str> {
        self.entries.get(path).map(|e| e.source.as_str())
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Perform an incremental re-parse of a file.
    ///
    /// Given the new source text, this method:
    /// 1. Retrieves the cached old tree and old source
    /// 2. Computes the InputEdit describing the text change
    /// 3. Calls `tree.edit()` to inform tree-sitter of the change
    /// 4. Re-parses with `parser.parse(new_source, Some(&old_tree))`
    ///
    /// Returns the new tree, or None if the file is not in the cache
    /// (in which case a full parse should be performed).
    pub fn incremental_reparse(
        &mut self,
        path: &Path,
        new_source: &str,
    ) -> Option<Tree> {
        let entry = self.entries.get_mut(path)?;
        let old_source = &entry.source;

        // Compute the edit between old and new source
        let edit = compute_input_edit(old_source, new_source);

        // Apply the edit to the old tree
        entry.tree.edit(&edit);

        // Get the language grammar for re-parsing
        let lang_name = entry.language.as_str();
        let ts_language = languages::language_for_name(lang_name)?;

        // Create parser and set language
        let mut parser = Parser::new();
        parser.set_language(&ts_language).ok()?;

        // Incremental re-parse: pass old tree as reference
        let new_tree = parser.parse(new_source, Some(&entry.tree))?;

        // Update the cache with the new tree and source
        entry.tree = new_tree.clone();
        entry.source = new_source.to_string();

        Some(new_tree)
    }
}

impl Default for TreeCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of diffing old vs new extraction results.
/// Contains only the symbols that actually changed.
#[derive(Debug, Clone)]
pub struct MicroDelta {
    /// Nodes that were added (new symbols not in old extraction).
    pub nodes_added: Vec<Node>,
    /// Nodes that were modified (same FQN but different content).
    pub nodes_modified: Vec<Node>,
    /// FQNs of nodes that were removed (in old but not in new).
    pub nodes_removed: Vec<String>,
    /// New edges to add.
    pub edges_to_add: Vec<Edge>,
    /// Edges to remove (by source_fqn, target_fqn pair).
    pub edges_to_remove: Vec<(String, String)>,
}

impl MicroDelta {
    /// Returns true if no changes were detected.
    pub fn is_empty(&self) -> bool {
        self.nodes_added.is_empty()
            && self.nodes_modified.is_empty()
            && self.nodes_removed.is_empty()
            && self.edges_to_add.is_empty()
            && self.edges_to_remove.is_empty()
    }
}

/// Diff old vs new extraction results to produce a micro-delta.
///
/// Compares nodes by FQN. A node is considered "modified" if its FQN exists
/// in both old and new but its start_line, end_line, or attributes differ.
/// Edges are compared by (source_fqn, target_fqn, kind) tuple.
pub fn diff_extractions(old: &ExtractionResult, new: &ExtractionResult) -> MicroDelta {
    let old_node_map: HashMap<&str, &Node> = old.nodes.iter().map(|n| (n.fqn.as_str(), n)).collect();
    let new_node_map: HashMap<&str, &Node> = new.nodes.iter().map(|n| (n.fqn.as_str(), n)).collect();

    let mut nodes_added = Vec::new();
    let mut nodes_modified = Vec::new();
    let mut nodes_removed = Vec::new();

    // Find added and modified nodes
    for (fqn, new_node) in &new_node_map {
        match old_node_map.get(fqn) {
            Some(old_node) => {
                // Check if the node was modified
                if node_differs(old_node, new_node) {
                    nodes_modified.push((*new_node).clone());
                }
            }
            None => {
                // New node
                nodes_added.push((*new_node).clone());
            }
        }
    }

    // Find removed nodes
    for fqn in old_node_map.keys() {
        if !new_node_map.contains_key(fqn) {
            nodes_removed.push(fqn.to_string());
        }
    }

    // Diff edges
    let old_edge_set: HashMap<(&str, &str, &str), &Edge> = old
        .edges
        .iter()
        .map(|e| {
            let kind_str = edge_kind_str(&e.kind);
            ((e.source_fqn.as_str(), e.target_fqn.as_str(), kind_str), e)
        })
        .collect();

    let new_edge_set: HashMap<(&str, &str, &str), &Edge> = new
        .edges
        .iter()
        .map(|e| {
            let kind_str = edge_kind_str(&e.kind);
            ((e.source_fqn.as_str(), e.target_fqn.as_str(), kind_str), e)
        })
        .collect();

    let mut edges_to_add = Vec::new();
    let mut edges_to_remove = Vec::new();

    for (key, edge) in &new_edge_set {
        if !old_edge_set.contains_key(key) {
            edges_to_add.push((*edge).clone());
        }
    }

    for (key, _edge) in &old_edge_set {
        if !new_edge_set.contains_key(key) {
            edges_to_remove.push((key.0.to_string(), key.1.to_string()));
        }
    }

    MicroDelta {
        nodes_added,
        nodes_modified,
        nodes_removed,
        edges_to_add,
        edges_to_remove,
    }
}

/// Check if two nodes differ in their structural content.
fn node_differs(old: &Node, new: &Node) -> bool {
    old.start_line != new.start_line
        || old.end_line != new.end_line
        || old.kind != new.kind
        || old.attributes != new.attributes
}

/// Get a string representation of an edge kind for comparison.
fn edge_kind_str(kind: &crate::store::types::EdgeKind) -> &'static str {
    use crate::store::types::EdgeKind;
    match kind {
        EdgeKind::Calls => "Calls",
        EdgeKind::Imports => "Imports",
        EdgeKind::Inherits => "Inherits",
        EdgeKind::Implements => "Implements",
        EdgeKind::HttpLink => "HttpLink",
        EdgeKind::DataFlow => "DataFlow",
    }
}

/// Compute a tree-sitter InputEdit from old source to new source.
///
/// This uses a simple approach: find the first and last differing bytes
/// to determine the edit range. For single-function edits this is very
/// efficient as it narrows down to just the changed region.
fn compute_input_edit(old_source: &str, new_source: &str) -> InputEdit {
    let old_bytes = old_source.as_bytes();
    let new_bytes = new_source.as_bytes();

    // Find the first byte that differs
    let start_byte = old_bytes
        .iter()
        .zip(new_bytes.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| old_bytes.len().min(new_bytes.len()));

    // Find the last byte that differs (from the end)
    let old_suffix_len = old_bytes[start_byte..]
        .iter()
        .rev()
        .zip(new_bytes[start_byte..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let old_end_byte = old_bytes.len() - old_suffix_len;
    let new_end_byte = new_bytes.len() - old_suffix_len;

    // Compute positions (row, column) for the edit points
    let start_position = byte_offset_to_point(old_source, start_byte);
    let old_end_position = byte_offset_to_point(old_source, old_end_byte);
    let new_end_position = byte_offset_to_point(new_source, new_end_byte);

    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position,
    }
}

/// Convert a byte offset in source text to a tree-sitter Point (row, column).
fn byte_offset_to_point(source: &str, byte_offset: usize) -> Point {
    let prefix = &source[..byte_offset.min(source.len())];
    let row = prefix.matches('\n').count();
    let last_newline = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let column = byte_offset - last_newline;
    Point { row, column }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::parser;
    use crate::store::types::{EdgeKind, NodeKind};

    #[test]
    fn test_tree_cache_insert_and_contains() {
        let mut cache = TreeCache::new();
        let path = PathBuf::from("test.py");
        let source = "def hello(): pass";

        let abs_path = PathBuf::from("test.py");
        let (lang, tree) = parser::parse(&abs_path, source).unwrap();

        cache.insert(path.clone(), tree, source.to_string(), lang);

        assert!(cache.contains(&path));
        assert!(!cache.contains(Path::new("other.py")));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get_source(&path), Some(source));
    }

    #[test]
    fn test_tree_cache_remove() {
        let mut cache = TreeCache::new();
        let path = PathBuf::from("test.py");
        let source = "def hello(): pass";

        let abs_path = PathBuf::from("test.py");
        let (lang, tree) = parser::parse(&abs_path, source).unwrap();

        cache.insert(path.clone(), tree, source.to_string(), lang);
        assert!(cache.contains(&path));

        cache.remove(&path);
        assert!(!cache.contains(&path));
        assert!(cache.is_empty());
    }

    #[test]
    fn test_incremental_reparse_modifies_function() {
        let mut cache = TreeCache::new();
        let path = PathBuf::from("test.py");

        let old_source = "def hello():\n    print('hello')\n\ndef world():\n    print('world')\n";
        let new_source = "def hello():\n    print('hi there')\n\ndef world():\n    print('world')\n";

        let (lang, tree) = parser::parse(&path, old_source).unwrap();
        cache.insert(path.clone(), tree, old_source.to_string(), lang);

        // Perform incremental re-parse
        let new_tree = cache.incremental_reparse(&path, new_source);
        assert!(new_tree.is_some());

        let new_tree = new_tree.unwrap();
        // The tree should be valid and parse the new source correctly
        assert_eq!(new_tree.root_node().kind(), "module");
        assert!(!new_tree.root_node().has_error());

        // Cache should be updated
        assert_eq!(cache.get_source(&path), Some(new_source));
    }

    #[test]
    fn test_incremental_reparse_not_in_cache() {
        let mut cache = TreeCache::new();
        let path = PathBuf::from("not_cached.py");

        let result = cache.incremental_reparse(&path, "def foo(): pass");
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_input_edit_simple_change() {
        let old = "def hello():\n    print('hello')\n";
        let new = "def hello():\n    print('hi')\n";

        let edit = compute_input_edit(old, new);

        // The edit should start somewhere in the string literal
        assert!(edit.start_byte > 0);
        assert!(edit.start_byte < old.len());
    }

    #[test]
    fn test_compute_input_edit_insertion() {
        let old = "def foo():\n    pass\n";
        let new = "def foo():\n    pass\n\ndef bar():\n    pass\n";

        let edit = compute_input_edit(old, new);

        // Start byte should be at the end of the old content
        assert_eq!(edit.start_byte, old.len());
        assert_eq!(edit.old_end_byte, old.len());
        assert!(edit.new_end_byte > edit.old_end_byte);
    }

    #[test]
    fn test_compute_input_edit_deletion() {
        let old = "def foo():\n    pass\n\ndef bar():\n    pass\n";
        let new = "def foo():\n    pass\n";

        let edit = compute_input_edit(old, new);

        assert!(edit.old_end_byte > edit.new_end_byte);
    }

    #[test]
    fn test_byte_offset_to_point() {
        let source = "line1\nline2\nline3\n";

        let p0 = byte_offset_to_point(source, 0);
        assert_eq!(p0.row, 0);
        assert_eq!(p0.column, 0);

        // Start of line2 (after "line1\n" = 6 bytes)
        let p1 = byte_offset_to_point(source, 6);
        assert_eq!(p1.row, 1);
        assert_eq!(p1.column, 0);

        // Middle of line2 (at 'i' in "line2")
        let p2 = byte_offset_to_point(source, 7);
        assert_eq!(p2.row, 1);
        assert_eq!(p2.column, 1);
    }

    #[test]
    fn test_diff_extractions_no_change() {
        let extraction = ExtractionResult {
            nodes: vec![Node {
                fqn: "test.py::hello".to_string(),
                kind: NodeKind::Function,
                file: "test.py".to_string(),
                start_line: 1,
                end_line: 2,
                file_hash: "hash1".to_string(),
                indexed_at: 1000,
                attributes: serde_json::json!({}),
            }],
            edges: vec![],
        };

        let delta = diff_extractions(&extraction, &extraction);
        assert!(delta.is_empty());
    }

    #[test]
    fn test_diff_extractions_node_added() {
        let old = ExtractionResult {
            nodes: vec![Node {
                fqn: "test.py::hello".to_string(),
                kind: NodeKind::Function,
                file: "test.py".to_string(),
                start_line: 1,
                end_line: 2,
                file_hash: "hash1".to_string(),
                indexed_at: 1000,
                attributes: serde_json::json!({}),
            }],
            edges: vec![],
        };

        let new = ExtractionResult {
            nodes: vec![
                Node {
                    fqn: "test.py::hello".to_string(),
                    kind: NodeKind::Function,
                    file: "test.py".to_string(),
                    start_line: 1,
                    end_line: 2,
                    file_hash: "hash2".to_string(),
                    indexed_at: 1001,
                    attributes: serde_json::json!({}),
                },
                Node {
                    fqn: "test.py::world".to_string(),
                    kind: NodeKind::Function,
                    file: "test.py".to_string(),
                    start_line: 4,
                    end_line: 5,
                    file_hash: "hash2".to_string(),
                    indexed_at: 1001,
                    attributes: serde_json::json!({}),
                },
            ],
            edges: vec![],
        };

        let delta = diff_extractions(&old, &new);
        assert_eq!(delta.nodes_added.len(), 1);
        assert_eq!(delta.nodes_added[0].fqn, "test.py::world");
        assert!(delta.nodes_modified.is_empty());
        assert!(delta.nodes_removed.is_empty());
    }

    #[test]
    fn test_diff_extractions_node_removed() {
        let old = ExtractionResult {
            nodes: vec![
                Node {
                    fqn: "test.py::hello".to_string(),
                    kind: NodeKind::Function,
                    file: "test.py".to_string(),
                    start_line: 1,
                    end_line: 2,
                    file_hash: "hash1".to_string(),
                    indexed_at: 1000,
                    attributes: serde_json::json!({}),
                },
                Node {
                    fqn: "test.py::world".to_string(),
                    kind: NodeKind::Function,
                    file: "test.py".to_string(),
                    start_line: 4,
                    end_line: 5,
                    file_hash: "hash1".to_string(),
                    indexed_at: 1000,
                    attributes: serde_json::json!({}),
                },
            ],
            edges: vec![],
        };

        let new = ExtractionResult {
            nodes: vec![Node {
                fqn: "test.py::hello".to_string(),
                kind: NodeKind::Function,
                file: "test.py".to_string(),
                start_line: 1,
                end_line: 2,
                file_hash: "hash2".to_string(),
                indexed_at: 1001,
                attributes: serde_json::json!({}),
            }],
            edges: vec![],
        };

        let delta = diff_extractions(&old, &new);
        assert!(delta.nodes_added.is_empty());
        assert!(delta.nodes_modified.is_empty());
        assert_eq!(delta.nodes_removed.len(), 1);
        assert_eq!(delta.nodes_removed[0], "test.py::world");
    }

    #[test]
    fn test_diff_extractions_node_modified() {
        let old = ExtractionResult {
            nodes: vec![Node {
                fqn: "test.py::hello".to_string(),
                kind: NodeKind::Function,
                file: "test.py".to_string(),
                start_line: 1,
                end_line: 2,
                file_hash: "hash1".to_string(),
                indexed_at: 1000,
                attributes: serde_json::json!({}),
            }],
            edges: vec![],
        };

        let new = ExtractionResult {
            nodes: vec![Node {
                fqn: "test.py::hello".to_string(),
                kind: NodeKind::Function,
                file: "test.py".to_string(),
                start_line: 1,
                end_line: 5, // end_line changed (function body grew)
                file_hash: "hash2".to_string(),
                indexed_at: 1001,
                attributes: serde_json::json!({}),
            }],
            edges: vec![],
        };

        let delta = diff_extractions(&old, &new);
        assert!(delta.nodes_added.is_empty());
        assert_eq!(delta.nodes_modified.len(), 1);
        assert_eq!(delta.nodes_modified[0].fqn, "test.py::hello");
        assert!(delta.nodes_removed.is_empty());
    }

    #[test]
    fn test_diff_extractions_edge_changes() {
        let old = ExtractionResult {
            nodes: vec![],
            edges: vec![Edge {
                id: None,
                source_fqn: "test.py::hello".to_string(),
                target_fqn: "test.py::world".to_string(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: serde_json::json!({}),
            }],
        };

        let new = ExtractionResult {
            nodes: vec![],
            edges: vec![Edge {
                id: None,
                source_fqn: "test.py::hello".to_string(),
                target_fqn: "test.py::foo".to_string(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                attributes: serde_json::json!({}),
            }],
        };

        let delta = diff_extractions(&old, &new);
        assert_eq!(delta.edges_to_add.len(), 1);
        assert_eq!(delta.edges_to_add[0].target_fqn, "test.py::foo");
        assert_eq!(delta.edges_to_remove.len(), 1);
        assert_eq!(delta.edges_to_remove[0], ("test.py::hello".to_string(), "test.py::world".to_string()));
    }

    /// Integration test: modify one function in a multi-function file,
    /// verify only that function's node is detected as changed in the micro-delta.
    ///
    /// We modify beta's body without changing line count so that gamma's
    /// position doesn't shift. This tests the core incremental behavior:
    /// only the actually-changed function appears in the delta.
    #[test]
    fn test_incremental_single_function_change() {
        use crate::indexer::languages::python::extract;

        let mut cache = TreeCache::new();
        let path = PathBuf::from("multi.py");

        let old_source = concat!(
            "def alpha():\n",
            "    return 1\n",
            "\n",
            "def beta():\n",
            "    return 2\n",
            "\n",
            "def gamma():\n",
            "    return 3\n",
        );

        // Modify only beta's body (same line count, different content)
        let new_source = concat!(
            "def alpha():\n",
            "    return 1\n",
            "\n",
            "def beta():\n",
            "    return 99\n",
            "\n",
            "def gamma():\n",
            "    return 3\n",
        );

        // Parse old source and cache it
        let (lang, old_tree) = parser::parse(&path, old_source).unwrap();
        cache.insert(path.clone(), old_tree.clone(), old_source.to_string(), lang);

        // Extract from old tree
        let old_extraction = extract(&old_tree, "multi.py", old_source);

        // Incremental re-parse
        let new_tree = cache.incremental_reparse(&path, new_source).unwrap();

        // Extract from new tree
        let new_extraction = extract(&new_tree, "multi.py", new_source);

        // Diff
        let delta = diff_extractions(&old_extraction, &new_extraction);

        // No nodes should be added or removed (same structure)
        assert!(delta.nodes_added.is_empty(), "no nodes should be added");
        assert!(delta.nodes_removed.is_empty(), "no nodes should be removed");

        // Only beta should be modified (its complexity attribute may change)
        // alpha and gamma should NOT be in modified since their lines didn't shift
        let modified_fqns: Vec<&str> = delta.nodes_modified.iter().map(|n| n.fqn.as_str()).collect();

        // alpha should not be modified
        assert!(
            !modified_fqns.contains(&"multi.py::alpha"),
            "alpha should not be modified"
        );

        // gamma should not be modified
        assert!(
            !modified_fqns.contains(&"multi.py::gamma"),
            "gamma should not be modified"
        );
    }
}
