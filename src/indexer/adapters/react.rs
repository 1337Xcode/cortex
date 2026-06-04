//! React framework adapter.
//!
//! Detects JSX component renders → Renders edges.
//! Detects `useContext(SomeContext)` → Injects edges.
//!
//! Uses regex-based pattern matching on JSX/TSX source code.
//! Only PascalCase component names are detected (not HTML elements like `<div>`).

use regex::Regex;
use serde_json::json;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node, NodeKind};

use super::FrameworkAdapter;

/// Confidence value for all framework adapter edges.
const ADAPTER_CONFIDENCE: f64 = 0.8;

/// Adapter for React component composition and context detection.
pub struct ReactAdapter;

impl ReactAdapter {
    /// Find the enclosing component FQN for a given line number.
    ///
    /// Returns the FQN of the narrowest function/class component that contains
    /// the given line. Falls back to the file path as a module-level FQN.
    fn find_enclosing_component_by_line(
        file: &str,
        line: u32,
        existing_nodes: &[Node],
    ) -> String {
        let mut best_match: Option<&Node> = None;
        let mut best_span = u32::MAX;

        for node in existing_nodes {
            // Only consider functions and classes as potential React components
            let is_component = matches!(
                node.kind,
                NodeKind::Function | NodeKind::Class | NodeKind::Method
            );
            if !is_component {
                continue;
            }

            // Check if this node contains the line
            if node.start_line <= line && node.end_line >= line {
                let span = node.end_line - node.start_line;
                // Prefer the narrowest enclosing node
                if span < best_span {
                    best_span = span;
                    best_match = Some(node);
                }
            }
        }

        match best_match {
            Some(node) => node.fqn.clone(),
            None => file.to_string(),
        }
    }

    /// Detect JSX component renders in the source code.
    ///
    /// Matches patterns like `<ComponentName` where ComponentName starts with
    /// an uppercase letter (PascalCase). HTML elements like `<div>` are excluded.
    fn detect_jsx_renders(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Match JSX opening tags with PascalCase names (React components).
        // Pattern: < followed by an uppercase letter, then word chars (including dots for namespaced).
        // Excludes HTML elements which start with lowercase.
        let re = Regex::new(r"<([A-Z][A-Za-z0-9_]*)(?:\s|>|/)").unwrap();

        for cap in re.captures_iter(source) {
            let component_name = &cap[1];
            let match_start = cap.get(0).unwrap().start();

            // Compute the line number of this match
            let line = source[..match_start].matches('\n').count() as u32 + 1;

            // Find the enclosing component (parent)
            let parent_fqn =
                Self::find_enclosing_component_by_line(file, line, existing_nodes);

            // Build target FQN: try to find the component in existing nodes first
            let target_fqn = existing_nodes
                .iter()
                .find(|n| n.fqn.ends_with(&format!("::{component_name}")))
                .map(|n| n.fqn.clone())
                .unwrap_or_else(|| format!("{file}::{component_name}"));

            // Don't create self-referencing edges
            if parent_fqn == target_fqn {
                continue;
            }

            // Avoid duplicate edges (same parent → same child)
            let already_exists = edges.iter().any(|e: &Edge| {
                e.source_fqn == parent_fqn
                    && e.target_fqn == target_fqn
                    && e.kind == EdgeKind::Renders
            });
            if already_exists {
                continue;
            }

            edges.push(Edge {
                id: None,
                source_fqn: parent_fqn,
                target_fqn,
                kind: EdgeKind::Renders,
                confidence: ADAPTER_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({ "component": component_name }),
            });
        }

        edges
    }

    /// Detect `useContext(SomeContext)` calls in the source code.
    ///
    /// Creates Injects edges from the enclosing component to the context.
    fn detect_use_context(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Match useContext(ContextName) patterns.
        // The context name must be a valid identifier (typically PascalCase).
        let re = Regex::new(r"useContext\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)").unwrap();

        for cap in re.captures_iter(source) {
            let context_name = &cap[1];
            let match_start = cap.get(0).unwrap().start();

            // Compute the line number of this match
            let line = source[..match_start].matches('\n').count() as u32 + 1;

            // Find the enclosing component
            let component_fqn =
                Self::find_enclosing_component_by_line(file, line, existing_nodes);

            // Build target FQN for the context
            let target_fqn = existing_nodes
                .iter()
                .find(|n| n.fqn.ends_with(&format!("::{context_name}")))
                .map(|n| n.fqn.clone())
                .unwrap_or_else(|| format!("{file}::{context_name}"));

            // Don't create self-referencing edges
            if component_fqn == target_fqn {
                continue;
            }

            // Avoid duplicate edges
            let already_exists = edges.iter().any(|e: &Edge| {
                e.source_fqn == component_fqn
                    && e.target_fqn == target_fqn
                    && e.kind == EdgeKind::Injects
            });
            if already_exists {
                continue;
            }

            edges.push(Edge {
                id: None,
                source_fqn: component_fqn,
                target_fqn,
                kind: EdgeKind::Injects,
                confidence: ADAPTER_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({ "context": context_name }),
            });
        }

        edges
    }
}

impl FrameworkAdapter for ReactAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::React
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Detect JSX component renders → Renders edges
        edges.extend(self.detect_jsx_renders(file, source, existing_nodes));

        // Detect useContext(SomeContext) → Injects edges
        edges.extend(self.detect_use_context(file, source, existing_nodes));

        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper to create a minimal Node for testing.
    fn make_node(fqn: &str, kind: NodeKind, file: &str, start: u32, end: u32) -> Node {
        Node {
            fqn: fqn.to_string(),
            kind,
            file: file.to_string(),
            start_line: start,
            end_line: end,
            file_hash: "test".to_string(),
            indexed_at: 0,
            attributes: json!({}),
        }
    }

    /// Helper to create a tree-sitter tree for a given source (using TypeScript parser).
    fn parse_tsx(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        let lang = tree_sitter_typescript::LANGUAGE_TSX;
        parser
            .set_language(&lang.into())
            .expect("Failed to set TSX language");
        parser.parse(source, None).expect("Failed to parse")
    }

    #[test]
    fn detects_jsx_component_render() {
        let source = r#"
function App() {
    return (
        <div>
            <Header />
            <Sidebar />
        </div>
    );
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            9,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        // Should detect Header and Sidebar renders
        assert_eq!(edges.len(), 2);

        let header_edge = edges.iter().find(|e| e.target_fqn.contains("Header")).unwrap();
        assert_eq!(header_edge.source_fqn, "src/App.tsx::App");
        assert_eq!(header_edge.target_fqn, "src/App.tsx::Header");
        assert_eq!(header_edge.kind, EdgeKind::Renders);
        assert_eq!(header_edge.confidence, 0.8);
        assert_eq!(header_edge.edge_source, EdgeSource::FrameworkAdapter);

        let sidebar_edge = edges.iter().find(|e| e.target_fqn.contains("Sidebar")).unwrap();
        assert_eq!(sidebar_edge.source_fqn, "src/App.tsx::App");
        assert_eq!(sidebar_edge.target_fqn, "src/App.tsx::Sidebar");
        assert_eq!(sidebar_edge.kind, EdgeKind::Renders);
        assert_eq!(sidebar_edge.confidence, 0.8);
    }

    #[test]
    fn ignores_html_elements() {
        let source = r#"
function App() {
    return (
        <div>
            <span>Hello</span>
            <p>World</p>
        </div>
    );
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            9,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        // HTML elements (lowercase) should not produce edges
        assert!(edges.is_empty());
    }

    #[test]
    fn detects_use_context() {
        let source = r#"
function Dashboard() {
    const theme = useContext(ThemeContext);
    const auth = useContext(AuthContext);
    return <div>{theme}</div>;
}
"#;
        let file = "src/Dashboard.tsx";
        let nodes = vec![make_node(
            "src/Dashboard.tsx::Dashboard",
            NodeKind::Function,
            file,
            2,
            6,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        // Should detect ThemeContext and AuthContext injects
        let inject_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Injects)
            .collect();
        assert_eq!(inject_edges.len(), 2);

        let theme_edge = inject_edges
            .iter()
            .find(|e| e.target_fqn.contains("ThemeContext"))
            .unwrap();
        assert_eq!(theme_edge.source_fqn, "src/Dashboard.tsx::Dashboard");
        assert_eq!(theme_edge.target_fqn, "src/Dashboard.tsx::ThemeContext");
        assert_eq!(theme_edge.kind, EdgeKind::Injects);
        assert_eq!(theme_edge.confidence, 0.8);
        assert_eq!(theme_edge.edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn resolves_target_from_existing_nodes() {
        let source = r#"
function App() {
    return <Header />;
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![
            make_node("src/App.tsx::App", NodeKind::Function, file, 2, 4),
            make_node(
                "src/components/Header.tsx::Header",
                NodeKind::Function,
                "src/components/Header.tsx",
                1,
                20,
            ),
        ];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert_eq!(edges.len(), 1);
        // Should resolve to the existing node's FQN
        assert_eq!(
            edges[0].target_fqn,
            "src/components/Header.tsx::Header"
        );
    }

    #[test]
    fn deduplicates_multiple_renders_of_same_component() {
        let source = r#"
function App() {
    return (
        <div>
            <Button />
            <Button />
            <Button />
        </div>
    );
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            9,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        // Should only produce one Renders edge per unique (parent, child) pair
        let button_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.target_fqn.contains("Button"))
            .collect();
        assert_eq!(button_edges.len(), 1);
    }

    #[test]
    fn handles_component_with_props() {
        let source = r#"
function Page() {
    return <Card title="Hello" size={3} />;
}
"#;
        let file = "src/Page.tsx";
        let nodes = vec![make_node(
            "src/Page.tsx::Page",
            NodeKind::Function,
            file,
            2,
            4,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_fqn, "src/Page.tsx::Card");
        assert_eq!(edges[0].kind, EdgeKind::Renders);
    }

    #[test]
    fn handles_nested_components() {
        let source = r#"
function Outer() {
    return (
        <Layout>
            <Inner />
        </Layout>
    );
}
"#;
        let file = "src/Outer.tsx";
        let nodes = vec![make_node(
            "src/Outer.tsx::Outer",
            NodeKind::Function,
            file,
            2,
            8,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert_eq!(edges.len(), 2);
        let targets: Vec<&str> = edges.iter().map(|e| e.target_fqn.as_str()).collect();
        assert!(targets.contains(&"src/Outer.tsx::Layout"));
        assert!(targets.contains(&"src/Outer.tsx::Inner"));
    }

    #[test]
    fn no_self_referencing_edges() {
        let source = r#"
function MyComponent() {
    return <MyComponent />;
}
"#;
        let file = "src/MyComponent.tsx";
        let nodes = vec![make_node(
            "src/MyComponent.tsx::MyComponent",
            NodeKind::Function,
            file,
            2,
            4,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        // Self-referencing edges should be excluded
        assert!(edges.is_empty());
    }

    #[test]
    fn combined_renders_and_context() {
        let source = r#"
function App() {
    const user = useContext(UserContext);
    return (
        <div>
            <Navbar user={user} />
            <Content />
        </div>
    );
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            10,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        let renders: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Renders).collect();
        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();

        assert_eq!(renders.len(), 2); // Navbar, Content
        assert_eq!(injects.len(), 1); // UserContext
        assert_eq!(injects[0].target_fqn, "src/App.tsx::UserContext");
    }

    #[test]
    fn framework_returns_react() {
        let adapter = ReactAdapter;
        assert_eq!(adapter.framework(), FrameworkKind::React);
    }

    #[test]
    fn empty_source_produces_no_edges() {
        let source = "";
        let file = "src/Empty.tsx";
        let nodes = vec![];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert!(edges.is_empty());
    }

    #[test]
    fn non_jsx_code_produces_no_edges() {
        let source = r#"
function add(a: number, b: number): number {
    return a + b;
}

const result = add(1, 2);
"#;
        let file = "src/utils.ts";
        let nodes = vec![make_node(
            "src/utils.ts::add",
            NodeKind::Function,
            file,
            2,
            4,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert!(edges.is_empty());
    }

    #[test]
    fn use_context_with_spaces() {
        let source = r#"
function Widget() {
    const ctx = useContext(  AppContext  );
    return <div />;
}
"#;
        let file = "src/Widget.tsx";
        let nodes = vec![make_node(
            "src/Widget.tsx::Widget",
            NodeKind::Function,
            file,
            2,
            5,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 1);
        assert_eq!(injects[0].target_fqn, "src/Widget.tsx::AppContext");
    }

    #[test]
    fn edge_attributes_contain_component_name() {
        let source = r#"
function App() {
    return <Header />;
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            4,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].attributes["component"], "Header");
    }

    #[test]
    fn edge_attributes_contain_context_name() {
        let source = r#"
function App() {
    const val = useContext(ThemeContext);
    return <div />;
}
"#;
        let file = "src/App.tsx";
        let nodes = vec![make_node(
            "src/App.tsx::App",
            NodeKind::Function,
            file,
            2,
            5,
        )];

        let tree = parse_tsx(source);
        let adapter = ReactAdapter;
        let edges = adapter.extract_edges(file, source, &tree, &nodes);

        let inject = edges.iter().find(|e| e.kind == EdgeKind::Injects).unwrap();
        assert_eq!(inject.attributes["context"], "ThemeContext");
    }
}
