//! FastAPI framework adapter.
//!
//! Detects `Depends(X)` patterns and creates `Injects` edges.
//! Detects `@app.get`, `@app.post`, `@router.get/post/put/delete` and creates `Routes` edges.
//! Handles transitive Depends chains.

use regex::Regex;
use serde_json::json;
use std::sync::LazyLock;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node};

use super::FrameworkAdapter;

/// Confidence value for all framework adapter edges (MEDIUM tier = 0.8).
const ADAPTER_CONFIDENCE: f64 = 0.8;

/// Regex matching a Python function definition: `def function_name(`
/// Captures the function name in group 1.
static DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*(?:async\s+)?def\s+(\w+)\s*\(").unwrap());

/// Regex matching `Depends(identifier)` patterns in function parameters.
/// Captures the dependency name in group 1.
static DEPENDS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Depends\(\s*(\w+)\s*\)").unwrap());

/// Regex matching FastAPI route decorators:
/// `@app.get(...)`, `@app.post(...)`, `@router.get(...)`, etc.
/// Captures the HTTP method in group 1.
static ROUTE_DECORATOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[ \t]*@\w+\.(get|post|put|delete|patch|options|head)\s*\("#).unwrap()
});

/// Adapter for FastAPI dependency injection and route detection.
pub struct FastApiAdapter;

impl FrameworkAdapter for FastApiAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::FastAPI
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        _existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Extract Depends() injection edges
        edges.extend(self.extract_depends_edges(file, source));

        // Extract route decorator edges
        edges.extend(self.extract_route_edges(file, source));

        edges
    }
}

impl FastApiAdapter {
    /// Extract `Injects` edges from `Depends(X)` patterns.
    ///
    /// For each function definition, finds all `Depends(dep_name)` in its
    /// parameter list and creates an Injects edge from the function to the
    /// dependency. This handles transitive chains: if `get_db` depends on
    /// `get_session` via `Depends(get_session)`, we create an edge from
    /// `get_db` to `get_session`.
    fn extract_depends_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Find all function definitions and their byte ranges
        let functions = self.find_function_ranges(source);

        for (func_name, func_start, func_end) in &functions {
            let func_body = &source[*func_start..*func_end];

            // Find all Depends() calls within this function's signature/body
            for dep_cap in DEPENDS_RE.captures_iter(func_body) {
                let dep_name = &dep_cap[1];

                // Create FQN for source (the function using Depends) and target (the dependency)
                let source_fqn = format!("{}::{}", file, func_name);
                let target_fqn = format!("{}::{}", file, dep_name);

                edges.push(Edge {
                    id: None,
                    source_fqn,
                    target_fqn,
                    kind: EdgeKind::Injects,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({
                        "framework": "fastapi",
                        "pattern": "Depends"
                    }),
                });
            }
        }

        edges
    }

    /// Extract `Routes` edges from route decorators.
    ///
    /// Detects patterns like `@app.get("/path")` or `@router.post("/path")`
    /// followed by a function definition, and creates a Routes edge from
    /// the router/app to the handler function.
    fn extract_route_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Find all route decorators and the function they decorate
        for route_match in ROUTE_DECORATOR_RE.find_iter(source) {
            let decorator_end = route_match.end();
            let http_method = ROUTE_DECORATOR_RE
                .captures(&source[route_match.start()..])
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();

            // Find the next function definition after this decorator
            let remaining = &source[decorator_end..];
            if let Some(def_match) = DEF_RE.captures(remaining) {
                let func_name = &def_match[1];

                // Extract the router/app variable name from the decorator
                let decorator_text = route_match.as_str().trim();
                let router_name = self.extract_router_name(decorator_text);

                let source_fqn = format!("{}::{}", file, router_name);
                let target_fqn = format!("{}::{}", file, func_name);

                edges.push(Edge {
                    id: None,
                    source_fqn,
                    target_fqn,
                    kind: EdgeKind::Routes,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({
                        "framework": "fastapi",
                        "http_method": http_method,
                        "pattern": "route_decorator"
                    }),
                });
            }
        }

        edges
    }

    /// Find all function definitions and their byte ranges in the source.
    ///
    /// Returns a vec of (function_name, start_byte, end_byte) tuples.
    /// The range covers from the `def` keyword to the start of the next
    /// function definition (or end of file).
    fn find_function_ranges(&self, source: &str) -> Vec<(String, usize, usize)> {
        let matches: Vec<_> = DEF_RE
            .captures_iter(source)
            .map(|cap| {
                let full_match = cap.get(0).unwrap();
                let name = cap[1].to_string();
                (name, full_match.start())
            })
            .collect();

        let mut ranges = Vec::new();
        for (i, (name, start)) in matches.iter().enumerate() {
            let end = if i + 1 < matches.len() {
                matches[i + 1].1
            } else {
                source.len()
            };
            ranges.push((name.clone(), *start, end));
        }

        ranges
    }

    /// Extract the router/app variable name from a decorator like `@app.get(`.
    /// Returns the identifier before the dot (e.g., "app", "router").
    fn extract_router_name(&self, decorator_text: &str) -> String {
        // Pattern: @identifier.method(
        // Strip leading @ and whitespace
        let trimmed = decorator_text.trim().trim_start_matches('@');
        // Take everything before the first dot
        trimmed
            .split('.')
            .next()
            .unwrap_or("app")
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal tree-sitter tree for testing.
    /// The adapter uses regex on source text, so the tree content doesn't matter.
    fn make_dummy_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn detects_simple_depends() {
        let source = r#"
from fastapi import Depends

def get_db():
    return Database()

def get_users(db = Depends(get_db)):
    return db.query(User)
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("app/routes.py", source, &tree, &[]);

        // Should find one Injects edge: get_users -> get_db
        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 1);
        assert_eq!(injects[0].source_fqn, "app/routes.py::get_users");
        assert_eq!(injects[0].target_fqn, "app/routes.py::get_db");
        assert_eq!(injects[0].confidence, 0.8);
        assert_eq!(injects[0].edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn detects_multiple_depends_in_one_function() {
        let source = r#"
def handler(
    db = Depends(get_db),
    user = Depends(get_current_user),
    settings = Depends(get_settings)
):
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("app/api.py", source, &tree, &[]);

        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 3);

        let targets: Vec<&str> = injects.iter().map(|e| e.target_fqn.as_str()).collect();
        assert!(targets.contains(&"app/api.py::get_db"));
        assert!(targets.contains(&"app/api.py::get_current_user"));
        assert!(targets.contains(&"app/api.py::get_settings"));
    }

    #[test]
    fn detects_transitive_depends_chain() {
        let source = r#"
def get_session():
    return Session()

def get_db(session = Depends(get_session)):
    return Database(session)

def get_users(db = Depends(get_db)):
    return db.query(User)
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("app/deps.py", source, &tree, &[]);

        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 2);

        // get_db -> get_session (transitive dependency)
        assert!(injects
            .iter()
            .any(|e| e.source_fqn == "app/deps.py::get_db"
                && e.target_fqn == "app/deps.py::get_session"));

        // get_users -> get_db
        assert!(injects
            .iter()
            .any(|e| e.source_fqn == "app/deps.py::get_users"
                && e.target_fqn == "app/deps.py::get_db"));
    }

    #[test]
    fn detects_route_decorators() {
        let source = r#"
from fastapi import APIRouter

router = APIRouter()

@router.get("/users")
def list_users():
    pass

@router.post("/users")
def create_user():
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("app/routes.py", source, &tree, &[]);

        let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        assert_eq!(routes.len(), 2);

        // router -> list_users
        assert!(routes
            .iter()
            .any(|e| e.source_fqn == "app/routes.py::router"
                && e.target_fqn == "app/routes.py::list_users"));

        // router -> create_user
        assert!(routes
            .iter()
            .any(|e| e.source_fqn == "app/routes.py::router"
                && e.target_fqn == "app/routes.py::create_user"));

        // All routes should have correct metadata
        for route in &routes {
            assert_eq!(route.confidence, 0.8);
            assert_eq!(route.edge_source, EdgeSource::FrameworkAdapter);
        }
    }

    #[test]
    fn detects_app_route_decorators() {
        let source = r#"
from fastapi import FastAPI

app = FastAPI()

@app.get("/health")
def health_check():
    return {"status": "ok"}

@app.post("/items")
async def create_item(item: Item):
    return item
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("main.py", source, &tree, &[]);

        let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        assert_eq!(routes.len(), 2);

        assert!(routes
            .iter()
            .any(|e| e.source_fqn == "main.py::app"
                && e.target_fqn == "main.py::health_check"));

        assert!(routes
            .iter()
            .any(|e| e.source_fqn == "main.py::app"
                && e.target_fqn == "main.py::create_item"));
    }

    #[test]
    fn detects_all_http_methods() {
        let source = r#"
@router.get("/a")
def get_handler():
    pass

@router.post("/b")
def post_handler():
    pass

@router.put("/c")
def put_handler():
    pass

@router.delete("/d")
def delete_handler():
    pass

@router.patch("/e")
def patch_handler():
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("api.py", source, &tree, &[]);

        let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        assert_eq!(routes.len(), 5);

        let targets: Vec<&str> = routes.iter().map(|e| e.target_fqn.as_str()).collect();
        assert!(targets.contains(&"api.py::get_handler"));
        assert!(targets.contains(&"api.py::post_handler"));
        assert!(targets.contains(&"api.py::put_handler"));
        assert!(targets.contains(&"api.py::delete_handler"));
        assert!(targets.contains(&"api.py::patch_handler"));
    }

    #[test]
    fn combined_routes_and_depends() {
        let source = r#"
from fastapi import APIRouter, Depends

router = APIRouter()

def get_db():
    pass

@router.get("/users")
def list_users(db = Depends(get_db)):
    return db.query(User)

@router.post("/users")
def create_user(db = Depends(get_db), user: UserCreate = Body(...)):
    return db.create(user)
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("app/users.py", source, &tree, &[]);

        let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();

        assert_eq!(routes.len(), 2);
        assert_eq!(injects.len(), 2);

        // Both handlers inject get_db
        assert!(injects
            .iter()
            .any(|e| e.source_fqn == "app/users.py::list_users"
                && e.target_fqn == "app/users.py::get_db"));
        assert!(injects
            .iter()
            .any(|e| e.source_fqn == "app/users.py::create_user"
                && e.target_fqn == "app/users.py::get_db"));
    }

    #[test]
    fn async_function_with_depends() {
        let source = r#"
async def get_current_user(token: str = Depends(oauth2_scheme)):
    return User(token=token)
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("auth.py", source, &tree, &[]);

        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 1);
        assert_eq!(injects[0].source_fqn, "auth.py::get_current_user");
        assert_eq!(injects[0].target_fqn, "auth.py::oauth2_scheme");
    }

    #[test]
    fn no_edges_for_non_fastapi_code() {
        let source = r#"
def hello():
    print("Hello, world!")

class MyClass:
    def method(self):
        pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("plain.py", source, &tree, &[]);

        assert!(edges.is_empty());
    }

    #[test]
    fn framework_returns_fastapi() {
        let adapter = FastApiAdapter;
        assert_eq!(adapter.framework(), FrameworkKind::FastAPI);
    }

    #[test]
    fn edge_attributes_contain_framework_info() {
        let source = r#"
def handler(db = Depends(get_db)):
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("test.py", source, &tree, &[]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].attributes["framework"], "fastapi");
        assert_eq!(edges[0].attributes["pattern"], "Depends");
    }

    #[test]
    fn route_edge_attributes_contain_http_method() {
        let source = r#"
@app.post("/items")
def create_item():
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("test.py", source, &tree, &[]);

        let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].attributes["http_method"], "post");
        assert_eq!(routes[0].attributes["framework"], "fastapi");
    }

    #[test]
    fn depends_with_whitespace_variations() {
        let source = r#"
def handler(
    a = Depends( get_a ),
    b = Depends(get_b),
    c = Depends(  get_c  )
):
    pass
"#;
        let tree = make_dummy_tree(source);
        let adapter = FastApiAdapter;
        let edges = adapter.extract_edges("test.py", source, &tree, &[]);

        let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(injects.len(), 3);

        let targets: Vec<&str> = injects.iter().map(|e| e.target_fqn.as_str()).collect();
        assert!(targets.contains(&"test.py::get_a"));
        assert!(targets.contains(&"test.py::get_b"));
        assert!(targets.contains(&"test.py::get_c"));
    }
}
