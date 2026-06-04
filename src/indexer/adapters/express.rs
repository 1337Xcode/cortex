//! Express framework adapter.
//!
//! Detects `app.use(middleware)` and `router.use(middleware)` → Middleware edges.
//! Detects `router.get/post/put/delete(path, handler)` → Routes edges.
//!
//! Uses regex-based pattern matching on source code. All edges are created with
//! `edge_source = EdgeSource::FrameworkAdapter` and `confidence = 0.8`.

use regex::Regex;
use serde_json::json;
use std::sync::LazyLock;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node};

use super::FrameworkAdapter;

// ---------------------------------------------------------------------------
// Regex patterns (compiled once via LazyLock)
// ---------------------------------------------------------------------------

/// Matches `app.use(middlewareName)` or `router.use(middlewareName)`.
/// Captures the variable name (app/router) and the middleware function name.
/// Handles both named functions and simple references like `app.use(cors())`.
static MIDDLEWARE_USE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)\b(\w+)\.(use)\(\s*([A-Za-z_]\w*)"
    )
    .unwrap()
});

/// Matches route registrations like `app.get('/path', handler)` or
/// `router.post('/path', handler)`. Captures the variable, HTTP method,
/// route path, and handler name (if it's a named function reference).
static ROUTE_HANDLER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)\b(\w+)\.(get|post|put|delete|patch|options|head|all)\(\s*['"`]([^'"`]*)['"`]\s*,\s*([A-Za-z_]\w*)"#
    )
    .unwrap()
});

/// Matches route registrations with inline arrow functions or anonymous functions.
/// e.g. `app.get('/path', (req, res) => { ... })` or `app.get('/path', function(req, res) { ... })`
/// We still create a Routes edge but target an anonymous handler FQN.
static ROUTE_INLINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)\b(\w+)\.(get|post|put|delete|patch|options|head|all)\(\s*['"`]([^'"`]*)['"`]\s*,\s*(?:\(|function\s*\()"#
    )
    .unwrap()
});

/// Confidence value for all framework adapter edges.
const FRAMEWORK_CONFIDENCE: f64 = 0.8;

/// Adapter for Express middleware and route detection.
pub struct ExpressAdapter;

impl ExpressAdapter {
    /// Build a fully-qualified name for a symbol in the given file.
    fn make_fqn(file: &str, name: &str) -> String {
        format!("{}::{}", file, name)
    }

    /// Check if a variable name looks like an Express app or router instance.
    /// Common names: app, router, server, api, route, routes, etc.
    fn is_express_variable(name: &str) -> bool {
        matches!(
            name,
            "app" | "router" | "server" | "api" | "route" | "routes" | "express"
        )
    }

    /// Extract middleware edges from source code.
    fn extract_middleware_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in MIDDLEWARE_USE_RE.captures_iter(source) {
            let var_name = cap.get(1).map_or("", |m| m.as_str());
            let middleware_name = cap.get(3).map_or("", |m| m.as_str());

            // Only process if the variable looks like an Express app/router
            if !Self::is_express_variable(var_name) {
                continue;
            }

            // Skip common non-middleware patterns (e.g., `app.use(express.json())`)
            // We still create an edge for `express` since it's a middleware factory
            if middleware_name.is_empty() {
                continue;
            }

            let source_fqn = Self::make_fqn(file, var_name);
            let target_fqn = Self::make_fqn(file, middleware_name);

            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn,
                kind: EdgeKind::Middleware,
                confidence: FRAMEWORK_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({
                    "pattern": "express.use",
                    "middleware": middleware_name,
                }),
            });
        }

        edges
    }

    /// Check if a name is a JavaScript keyword (not a valid handler name).
    fn is_js_keyword(name: &str) -> bool {
        matches!(
            name,
            "function" | "async" | "await" | "class" | "const" | "let" | "var"
                | "new" | "return" | "if" | "else" | "for" | "while" | "do"
                | "switch" | "case" | "break" | "continue" | "throw" | "try"
                | "catch" | "finally" | "typeof" | "instanceof" | "void"
                | "delete" | "in" | "of" | "this" | "super" | "null"
                | "undefined" | "true" | "false"
        )
    }

    /// Extract route handler edges from source code.
    fn extract_route_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Named handler references: router.get('/path', handlerFn)
        for cap in ROUTE_HANDLER_RE.captures_iter(source) {
            let var_name = cap.get(1).map_or("", |m| m.as_str());
            let method = cap.get(2).map_or("", |m| m.as_str());
            let path = cap.get(3).map_or("", |m| m.as_str());
            let handler_name = cap.get(4).map_or("", |m| m.as_str());

            if !Self::is_express_variable(var_name) {
                continue;
            }

            // Skip JavaScript keywords — these indicate inline functions, not named handlers
            if Self::is_js_keyword(handler_name) {
                continue;
            }

            let source_fqn = Self::make_fqn(file, var_name);
            let target_fqn = Self::make_fqn(file, handler_name);

            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn,
                kind: EdgeKind::Routes,
                confidence: FRAMEWORK_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({
                    "pattern": "express.route",
                    "method": method.to_uppercase(),
                    "path": path,
                    "handler": handler_name,
                }),
            });
        }

        // Inline handlers: router.get('/path', (req, res) => { ... })
        for cap in ROUTE_INLINE_RE.captures_iter(source) {
            let var_name = cap.get(1).map_or("", |m| m.as_str());
            let method = cap.get(2).map_or("", |m| m.as_str());
            let path = cap.get(3).map_or("", |m| m.as_str());

            if !Self::is_express_variable(var_name) {
                continue;
            }

            // Check this isn't already captured by the named handler regex.
            // A match is "already captured" only if the named handler regex matched
            // at the same position AND the handler name is not a JS keyword.
            let match_start = cap.get(0).unwrap().start();
            let already_captured = ROUTE_HANDLER_RE
                .captures_iter(source)
                .any(|named_cap| {
                    let same_pos = named_cap.get(0).unwrap().start() == match_start;
                    let handler = named_cap.get(4).map_or("", |m| m.as_str());
                    same_pos && !Self::is_js_keyword(handler)
                });

            if already_captured {
                continue;
            }

            let source_fqn = Self::make_fqn(file, var_name);
            let anonymous_name = format!("anonymous_{}_{}", method, path.replace('/', "_"));
            let target_fqn = Self::make_fqn(file, &anonymous_name);

            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn,
                kind: EdgeKind::Routes,
                confidence: FRAMEWORK_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({
                    "pattern": "express.route",
                    "method": method.to_uppercase(),
                    "path": path,
                    "handler": "<anonymous>",
                }),
            });
        }

        edges
    }
}

impl FrameworkAdapter for ExpressAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::Express
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        _existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Only process JavaScript/TypeScript files
        if !file.ends_with(".js")
            && !file.ends_with(".ts")
            && !file.ends_with(".mjs")
            && !file.ends_with(".cjs")
        {
            return edges;
        }

        edges.extend(self.extract_middleware_edges(file, source));
        edges.extend(self.extract_route_edges(file, source));

        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal tree-sitter tree for testing.
    /// The Express adapter doesn't use the tree, so we just need a valid parse.
    fn make_dummy_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn detects_app_use_middleware() {
        let source = r#"
const express = require('express');
const cors = require('cors');
const app = express();

app.use(cors);
app.use(helmet);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/app.js", source, &tree, &[]);

        assert_eq!(edges.len(), 2);

        // First middleware: cors
        assert_eq!(edges[0].kind, EdgeKind::Middleware);
        assert_eq!(edges[0].source_fqn, "src/app.js::app");
        assert_eq!(edges[0].target_fqn, "src/app.js::cors");
        assert_eq!(edges[0].confidence, 0.8);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);

        // Second middleware: helmet
        assert_eq!(edges[1].kind, EdgeKind::Middleware);
        assert_eq!(edges[1].source_fqn, "src/app.js::app");
        assert_eq!(edges[1].target_fqn, "src/app.js::helmet");
    }

    #[test]
    fn detects_router_use_middleware() {
        let source = r#"
const router = express.Router();
router.use(authMiddleware);
router.use(validateInput);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/routes.js", source, &tree, &[]);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, EdgeKind::Middleware);
        assert_eq!(edges[0].source_fqn, "src/routes.js::router");
        assert_eq!(edges[0].target_fqn, "src/routes.js::authMiddleware");

        assert_eq!(edges[1].kind, EdgeKind::Middleware);
        assert_eq!(edges[1].source_fqn, "src/routes.js::router");
        assert_eq!(edges[1].target_fqn, "src/routes.js::validateInput");
    }

    #[test]
    fn detects_route_with_named_handler() {
        let source = r#"
const router = express.Router();
router.get('/users', getUsers);
router.post('/users', createUser);
router.put('/users/:id', updateUser);
router.delete('/users/:id', deleteUser);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/users.ts", source, &tree, &[]);

        assert_eq!(edges.len(), 4);

        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].source_fqn, "src/users.ts::router");
        assert_eq!(edges[0].target_fqn, "src/users.ts::getUsers");
        assert_eq!(edges[0].attributes["method"], "GET");
        assert_eq!(edges[0].attributes["path"], "/users");

        assert_eq!(edges[1].kind, EdgeKind::Routes);
        assert_eq!(edges[1].target_fqn, "src/users.ts::createUser");
        assert_eq!(edges[1].attributes["method"], "POST");

        assert_eq!(edges[2].kind, EdgeKind::Routes);
        assert_eq!(edges[2].target_fqn, "src/users.ts::updateUser");
        assert_eq!(edges[2].attributes["method"], "PUT");
        assert_eq!(edges[2].attributes["path"], "/users/:id");

        assert_eq!(edges[3].kind, EdgeKind::Routes);
        assert_eq!(edges[3].target_fqn, "src/users.ts::deleteUser");
        assert_eq!(edges[3].attributes["method"], "DELETE");
    }

    #[test]
    fn detects_app_route_with_named_handler() {
        let source = r#"
const app = express();
app.get('/health', healthCheck);
app.post('/login', loginHandler);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/server.js", source, &tree, &[]);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].source_fqn, "src/server.js::app");
        assert_eq!(edges[0].target_fqn, "src/server.js::healthCheck");
        assert_eq!(edges[0].attributes["method"], "GET");
        assert_eq!(edges[0].attributes["path"], "/health");

        assert_eq!(edges[1].target_fqn, "src/server.js::loginHandler");
        assert_eq!(edges[1].attributes["method"], "POST");
    }

    #[test]
    fn detects_route_with_inline_arrow_function() {
        let source = r#"
const app = express();
app.get('/status', (req, res) => {
    res.json({ status: 'ok' });
});
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/app.js", source, &tree, &[]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].source_fqn, "src/app.js::app");
        assert_eq!(edges[0].attributes["method"], "GET");
        assert_eq!(edges[0].attributes["path"], "/status");
        assert_eq!(edges[0].attributes["handler"], "<anonymous>");
    }

    #[test]
    fn detects_route_with_inline_function_keyword() {
        let source = r#"
const router = express.Router();
router.post('/submit', function(req, res) {
    res.send('done');
});
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/form.js", source, &tree, &[]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].source_fqn, "src/form.js::router");
        assert_eq!(edges[0].attributes["method"], "POST");
        assert_eq!(edges[0].attributes["path"], "/submit");
        assert_eq!(edges[0].attributes["handler"], "<anonymous>");
    }

    #[test]
    fn ignores_non_express_variables() {
        let source = r#"
const myObj = {};
myObj.use(something);
myObj.get('/path', handler);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/other.js", source, &tree, &[]);

        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn ignores_non_js_files() {
        let source = r#"
app.use(cors);
router.get('/users', getUsers);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/app.py", source, &tree, &[]);

        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn handles_typescript_files() {
        let source = r#"
const app = express();
app.use(bodyParser);
app.get('/api/data', fetchData);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/index.ts", source, &tree, &[]);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, EdgeKind::Middleware);
        assert_eq!(edges[1].kind, EdgeKind::Routes);
    }

    #[test]
    fn handles_mjs_and_cjs_files() {
        let source = r#"
const app = express();
app.use(logger);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;

        let edges_mjs = adapter.extract_edges("src/app.mjs", source, &tree, &[]);
        assert_eq!(edges_mjs.len(), 1);

        let edges_cjs = adapter.extract_edges("src/app.cjs", source, &tree, &[]);
        assert_eq!(edges_cjs.len(), 1);
    }

    #[test]
    fn mixed_middleware_and_routes() {
        let source = r#"
const app = express();
app.use(cors);
app.use(helmet);
app.get('/api/users', listUsers);
app.post('/api/users', createUser);
app.use(errorHandler);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/app.js", source, &tree, &[]);

        let middleware_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Middleware).collect();
        let route_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();

        assert_eq!(middleware_edges.len(), 3); // cors, helmet, errorHandler
        assert_eq!(route_edges.len(), 2); // listUsers, createUser
    }

    #[test]
    fn all_edges_have_correct_confidence_and_source() {
        let source = r#"
const app = express();
app.use(cors);
app.get('/test', handler);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/app.js", source, &tree, &[]);

        for edge in &edges {
            assert_eq!(edge.confidence, 0.8);
            assert_eq!(edge.edge_source, EdgeSource::FrameworkAdapter);
            assert!(edge.id.is_none());
        }
    }

    #[test]
    fn framework_returns_express() {
        let adapter = ExpressAdapter;
        assert_eq!(adapter.framework(), FrameworkKind::Express);
    }

    #[test]
    fn empty_source_produces_no_edges() {
        let source = "";
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/empty.js", source, &tree, &[]);
        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn handles_server_variable_name() {
        let source = r#"
const server = express();
server.use(morgan);
server.get('/ping', pingHandler);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/server.js", source, &tree, &[]);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].source_fqn, "src/server.js::server");
        assert_eq!(edges[1].source_fqn, "src/server.js::server");
    }

    #[test]
    fn handles_api_variable_name() {
        let source = r#"
const api = express.Router();
api.get('/v1/items', getItems);
"#;
        let tree = make_dummy_tree(source);
        let adapter = ExpressAdapter;
        let edges = adapter.extract_edges("src/api.js", source, &tree, &[]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_fqn, "src/api.js::api");
        assert_eq!(edges[0].target_fqn, "src/api.js::getItems");
    }
}
