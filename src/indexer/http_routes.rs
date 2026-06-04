//! HTTP route detection via regex-based pattern matching.
//!
//! Detects route registrations for common web frameworks and creates Route nodes
//! with method/path attributes and HttpLink edges for HTTP client calls.
//!
//! Supported frameworks:
//! - Python: FastAPI, Flask
//! - JavaScript/TypeScript: Express
//! - Go: Gin, Echo
//!
//! This module uses a regex-based approach on source text rather than full AST
//! traversal for simplicity. More sophisticated AST-based detection can be added later.

use regex::Regex;
use serde_json::json;

use crate::store::types::{Edge, EdgeKind, Node, NodeKind};

/// Detect HTTP route definitions and HTTP client calls in source code.
///
/// Modifies the provided node and edge vectors in-place, adding:
/// - Route nodes with method/path/framework attributes
/// - HttpLink edges from client call sites to matching Route nodes
pub fn detect_routes(
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    file: &str,
    source: &str,
    lang: &str,
) {
    let detected_routes = match lang {
        "python" | "py" => detect_python_routes(file, source),
        "javascript" | "js" | "typescript" | "ts" | "tsx" => detect_express_routes(file, source),
        "go" => detect_go_routes(file, source),
        "java" => detect_spring_routes(file, source),
        "ruby" | "rb" => detect_rails_routes(file, source),
        _ => Vec::new(),
    };

    // Also detect Django routes from Python files
    if lang == "python" || lang == "py" {
        let django_routes = detect_django_routes(file, source);
        nodes.extend(django_routes);
    }

    // Add detected route nodes
    nodes.extend(detected_routes.clone());

    // Detect HTTP client calls and create HttpLink edges
    let client_calls = match lang {
        "python" | "py" => detect_python_http_clients(file, source),
        "javascript" | "js" | "typescript" | "ts" | "tsx" => detect_js_http_clients(file, source),
        "go" => detect_go_http_clients(file, source),
        _ => Vec::new(),
    };

    // For each client call, try to match against known routes (including newly detected ones)
    let all_routes: Vec<&Node> = nodes.iter().filter(|n| n.kind == NodeKind::Route).collect();

    for (caller_fqn, url_path, line) in &client_calls {
        if let Some((target_fqn, confidence)) = find_matching_route(&all_routes, url_path) {
            edges.push(Edge {
                id: None,
                source_fqn: caller_fqn.clone(),
                target_fqn,
                kind: EdgeKind::HttpLink,
                confidence,
                edge_source: crate::store::confidence::EdgeSource::AstDirect,
                attributes: json!({
                    "url": url_path,
                    "source_line": line,
                }),
            });
        }
    }
}

/// Detect Python route patterns (FastAPI and Flask).
fn detect_python_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    // FastAPI patterns: @app.get("/path"), @app.post("/path"), @router.get("/path"), etc.
    let fastapi_re =
        Regex::new(r#"@(?:app|router)\.(get|post|put|delete|patch)\(\s*["']([^"']+)["']"#).unwrap();

    // Flask patterns: @app.route("/path", methods=["GET"]), @blueprint.route(...)
    let flask_route_re = Regex::new(
        r#"@(?:app|blueprint)\.route\(\s*["']([^"']+)["'](?:\s*,\s*methods\s*=\s*\[["'](\w+)["']\])?"#,
    )
    .unwrap();

    let lines: Vec<&str> = source.lines().collect();

    // Detect FastAPI routes
    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = fastapi_re.captures(line_text) {
            let method = caps.get(1).unwrap().as_str().to_uppercase();
            let path = caps.get(2).unwrap().as_str().to_string();

            // Find the handler function name (next def line after decorator)
            let handler_name = find_next_function_name(&lines, line_idx);
            let handler_fqn = match &handler_name {
                Some(name) => format!("{file}::{name}"),
                None => format!("{file}::route_{}", path.replace('/', "_")),
            };

            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": "fastapi",
                    "handler": handler_fqn,
                }),
            });
        }

        // Detect Flask routes
        if let Some(caps) = flask_route_re.captures(line_text) {
            let path = caps.get(1).unwrap().as_str().to_string();
            let method = caps
                .get(2)
                .map(|m| m.as_str().to_uppercase())
                .unwrap_or_else(|| "GET".to_string());

            let handler_name = find_next_function_name(&lines, line_idx);
            let handler_fqn = match &handler_name {
                Some(name) => format!("{file}::{name}"),
                None => format!("{file}::route_{}", path.replace('/', "_")),
            };

            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": "flask",
                    "handler": handler_fqn,
                }),
            });
        }
    }

    routes
}

/// Detect Express route patterns: app.get("/path", handler), router.post("/path", handler)
fn detect_express_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    let express_re =
        Regex::new(r#"(?:app|router)\.(get|post|put|delete|patch)\(\s*["']([^"']+)["']"#).unwrap();

    // Vercel/Next.js: export default (async) function handler(req, res)
    // Route path derived from file path (api/chat.ts -> /api/chat)
    let vercel_re =
        Regex::new(r#"export\s+default\s+(?:async\s+)?function\s+\w*handler\b"#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = express_re.captures(line_text) {
            let method = caps.get(1).unwrap().as_str().to_uppercase();
            let path = caps.get(2).unwrap().as_str().to_string();
            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": "express",
                }),
            });
        }

        if vercel_re.is_match(line_text) {
            let path = format!("/{}", file.trim_end_matches(".ts").trim_end_matches(".js"));
            let route_fqn = format!("{file}::route::ANY:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": "ANY",
                    "path": path,
                    "framework": "vercel",
                }),
            });
        }
    }

    routes
}

/// Detect Go route patterns for Gin and Echo.
/// Gin: r.GET("/path", handler), r.POST("/path", handler)
/// Echo: e.GET("/path", handler), e.POST("/path", handler)
fn detect_go_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    // Gin/Echo patterns: variable.METHOD("/path", handler)
    // Methods are uppercase in Go: GET, POST, PUT, DELETE, PATCH
    let go_route_re = Regex::new(r#"\w+\.(GET|POST|PUT|DELETE|PATCH)\(\s*"([^"]+)""#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = go_route_re.captures(line_text) {
            let method = caps.get(1).unwrap().as_str().to_string();
            let path = caps.get(2).unwrap().as_str().to_string();

            // Determine framework by looking for gin or echo imports in the source
            let framework = if source.contains("github.com/gin-gonic/gin") {
                "gin"
            } else if source.contains("github.com/labstack/echo") {
                "echo"
            } else {
                "go-http"
            };

            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": framework,
                }),
            });
        }
    }

    routes
}

/// Detect Django URL patterns: path("/api/...", view), re_path(...)
fn detect_django_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    // Django path() patterns: path("api/users/", views.user_list, name="user-list")
    let path_re = Regex::new(r#"path\(\s*["']([^"']+)["']"#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = path_re.captures(line_text) {
            let path = caps.get(1).unwrap().as_str().to_string();

            // Normalize path to start with /
            let normalized_path = if path.starts_with('/') {
                path.clone()
            } else {
                format!("/{}", path)
            };

            // Django doesn't specify method in URL conf, default to ALL
            let route_fqn = format!("{file}::route::ALL:{normalized_path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": "ALL",
                    "path": normalized_path,
                    "framework": "django",
                }),
            });
        }
    }

    routes
}

/// Detect Spring route patterns: @GetMapping("/path"), @PostMapping("/path"),
/// @RequestMapping(value="/path", method=RequestMethod.GET)
fn detect_spring_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    // @GetMapping, @PostMapping, @PutMapping, @DeleteMapping, @PatchMapping
    let mapping_re =
        Regex::new(r#"@(Get|Post|Put|Delete|Patch)Mapping\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#)
            .unwrap();

    // @RequestMapping with method
    let request_mapping_re = Regex::new(
        r#"@RequestMapping\(\s*(?:value\s*=\s*)?["']([^"']+)["'](?:.*method\s*=\s*RequestMethod\.(\w+))?"#,
    )
    .unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = mapping_re.captures(line_text) {
            let method = caps.get(1).unwrap().as_str().to_uppercase();
            let path = caps.get(2).unwrap().as_str().to_string();

            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": "spring",
                }),
            });
        } else if let Some(caps) = request_mapping_re.captures(line_text) {
            let path = caps.get(1).unwrap().as_str().to_string();
            let method = caps
                .get(2)
                .map(|m| m.as_str().to_uppercase())
                .unwrap_or_else(|| "ALL".to_string());

            let route_fqn = format!("{file}::route::{method}:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": path,
                    "framework": "spring",
                }),
            });
        }
    }

    routes
}

/// Detect Rails route patterns: resources :orders, get "/path", to: "controller#action"
fn detect_rails_routes(file: &str, source: &str) -> Vec<Node> {
    let mut routes = Vec::new();

    // resources :name
    let resources_re = Regex::new(r#"resources\s+:(\w+)"#).unwrap();

    // get/post/put/delete/patch "/path", to: "controller#action"
    let verb_route_re = Regex::new(r#"(get|post|put|delete|patch)\s+["']([^"']+)["']"#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = resources_re.captures(line_text) {
            let resource = caps.get(1).unwrap().as_str();
            let path = format!("/{}", resource);

            // RESTful resources generate multiple routes; represent as ALL
            let route_fqn = format!("{file}::route::ALL:{path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": "ALL",
                    "path": path,
                    "framework": "rails",
                    "resource": resource,
                }),
            });
        } else if let Some(caps) = verb_route_re.captures(line_text) {
            let method = caps.get(1).unwrap().as_str().to_uppercase();
            let path = caps.get(2).unwrap().as_str().to_string();

            let normalized_path = if path.starts_with('/') {
                path.clone()
            } else {
                format!("/{}", path)
            };

            let route_fqn = format!("{file}::route::{method}:{normalized_path}");
            routes.push(Node {
                fqn: route_fqn,
                kind: NodeKind::Route,
                file: file.to_string(),
                start_line: (line_idx + 1) as u32,
                end_line: (line_idx + 1) as u32,
                file_hash: String::new(),
                indexed_at: 0,
                attributes: json!({
                    "method": method,
                    "path": normalized_path,
                    "framework": "rails",
                }),
            });
        }
    }

    routes
}

/// Detect Python HTTP client calls: requests.get("url"), requests.post("url")
fn detect_python_http_clients(file: &str, source: &str) -> Vec<(String, String, u32)> {
    let mut calls = Vec::new();

    let requests_re =
        Regex::new(r#"requests\.(get|post|put|delete|patch)\(\s*["']([^"']+)["']"#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = requests_re.captures(line_text) {
            let url = caps.get(2).unwrap().as_str().to_string();
            let path = extract_path_from_url(&url);

            // Determine caller FQN by finding enclosing function
            let caller_fqn = find_enclosing_python_function(&lines, line_idx, file);

            calls.push((caller_fqn, path, (line_idx + 1) as u32));
        }
    }

    calls
}

/// Detect JavaScript/TypeScript HTTP client calls: fetch("url"), axios.get("url")
fn detect_js_http_clients(file: &str, source: &str) -> Vec<(String, String, u32)> {
    let mut calls = Vec::new();

    let fetch_re = Regex::new(r#"fetch\(\s*["']([^"']+)["']"#).unwrap();
    let axios_re =
        Regex::new(r#"axios\.(get|post|put|delete|patch)\(\s*["']([^"']+)["']"#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = fetch_re.captures(line_text) {
            let url = caps.get(1).unwrap().as_str().to_string();
            let path = extract_path_from_url(&url);
            let caller_fqn = find_enclosing_js_function(&lines, line_idx, file);
            calls.push((caller_fqn, path, (line_idx + 1) as u32));
        }

        if let Some(caps) = axios_re.captures(line_text) {
            let url = caps.get(2).unwrap().as_str().to_string();
            let path = extract_path_from_url(&url);
            let caller_fqn = find_enclosing_js_function(&lines, line_idx, file);
            calls.push((caller_fqn, path, (line_idx + 1) as u32));
        }
    }

    calls
}

/// Detect Go HTTP client calls: http.Get("url")
fn detect_go_http_clients(file: &str, source: &str) -> Vec<(String, String, u32)> {
    let mut calls = Vec::new();

    let http_get_re = Regex::new(r#"http\.(Get|Post|Head)\(\s*"([^"]+)""#).unwrap();

    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        if let Some(caps) = http_get_re.captures(line_text) {
            let url = caps.get(2).unwrap().as_str().to_string();
            let path = extract_path_from_url(&url);
            let caller_fqn = find_enclosing_go_function(&lines, line_idx, file);
            calls.push((caller_fqn, path, (line_idx + 1) as u32));
        }
    }

    calls
}

/// Extract the path component from a URL string.
/// e.g., "http://localhost:8000/api/orders" -> "/api/orders"
/// e.g., "/api/orders" -> "/api/orders"
fn extract_path_from_url(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        // Find the path after the host
        if let Some(slash_idx) = url[8..].find('/') {
            return url[8 + slash_idx..].to_string();
        }
        // URL with no path
        "/".to_string()
    } else if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{url}")
    }
}

/// Find the next function definition name after a given line (for Python decorators).
fn find_next_function_name(lines: &[&str], start_line: usize) -> Option<String> {
    let def_re = Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)").unwrap();

    for line in lines.iter().skip(start_line + 1).take(5) {
        if let Some(caps) = def_re.captures(line) {
            return Some(caps.get(1).unwrap().as_str().to_string());
        }
    }
    None
}

/// Find the enclosing Python function for a given line.
fn find_enclosing_python_function(lines: &[&str], line_idx: usize, file: &str) -> String {
    let def_re = Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)").unwrap();

    for i in (0..line_idx).rev() {
        if let Some(caps) = def_re.captures(lines[i]) {
            let name = caps.get(1).unwrap().as_str();
            return format!("{file}::{name}");
        }
    }
    file.to_string()
}

/// Find the enclosing JavaScript/TypeScript function for a given line.
fn find_enclosing_js_function(lines: &[&str], line_idx: usize, file: &str) -> String {
    let func_re = Regex::new(r"(?:function\s+(\w+)|(?:const|let|var)\s+(\w+)\s*=)").unwrap();

    for i in (0..line_idx).rev() {
        if let Some(caps) = func_re.captures(lines[i]) {
            let name = caps.get(1).or_else(|| caps.get(2)).unwrap().as_str();
            return format!("{file}::{name}");
        }
    }
    file.to_string()
}

/// Find the enclosing Go function for a given line.
fn find_enclosing_go_function(lines: &[&str], line_idx: usize, file: &str) -> String {
    let func_re = Regex::new(r"^func\s+(?:\(\w+\s+\*?\w+\)\s+)?(\w+)").unwrap();

    for i in (0..line_idx).rev() {
        if let Some(caps) = func_re.captures(lines[i]) {
            let name = caps.get(1).unwrap().as_str();
            return format!("{file}::{name}");
        }
    }
    file.to_string()
}

/// Find a matching route node for a given URL path.
/// Returns (target_fqn, confidence) if a match is found.
fn find_matching_route(routes: &[&Node], url_path: &str) -> Option<(String, f64)> {
    // Try exact match first (confidence 1.0)
    for route in routes {
        if let Some(route_path) = route.attributes.get("path").and_then(|v| v.as_str())
            && route_path == url_path
        {
            return Some((route.fqn.clone(), 1.0));
        }
    }

    // Try parameterized match (confidence 0.8)
    // e.g., route "/users/:id" matches client call "/users/123"
    for route in routes {
        if let Some(route_path) = route.attributes.get("path").and_then(|v| v.as_str())
            && paths_match_parameterized(route_path, url_path)
        {
            return Some((route.fqn.clone(), 0.8));
        }
    }

    // Try partial/prefix match (confidence 0.5)
    for route in routes {
        if let Some(route_path) = route.attributes.get("path").and_then(|v| v.as_str())
            && (url_path.starts_with(route_path) || route_path.starts_with(url_path))
        {
            return Some((route.fqn.clone(), 0.5));
        }
    }

    None
}

/// Check if a parameterized route path matches a concrete URL path.
/// e.g., "/users/:id" matches "/users/123"
/// e.g., "/users/{id}" matches "/users/123"
fn paths_match_parameterized(route_path: &str, url_path: &str) -> bool {
    let route_segments: Vec<&str> = route_path.split('/').collect();
    let url_segments: Vec<&str> = url_path.split('/').collect();

    if route_segments.len() != url_segments.len() {
        return false;
    }

    for (route_seg, url_seg) in route_segments.iter().zip(url_segments.iter()) {
        if route_seg.starts_with(':') || route_seg.starts_with('{') || *route_seg == *url_seg {
            continue;
        }
        return false;
    }

    // At least one segment must be a parameter for this to be a parameterized match
    route_segments
        .iter()
        .any(|s| s.starts_with(':') || s.starts_with('{'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Test: Flask route detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_flask_route_detected() {
        let source = r#"
from flask import Flask

app = Flask(__name__)

@app.route("/orders", methods=["GET"])
def get_orders():
    return jsonify(orders)

@app.route("/orders", methods=["POST"])
def create_order():
    return jsonify(order), 201
"#;
        let file = "src/routes/orders.py";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "python");

        // Should detect 2 route nodes
        assert_eq!(nodes.len(), 2);

        // Check first route
        let get_route = nodes
            .iter()
            .find(|n| n.attributes.get("method").and_then(|v| v.as_str()) == Some("GET"))
            .expect("GET route should be detected");
        assert_eq!(get_route.kind, NodeKind::Route);
        assert_eq!(
            get_route.attributes.get("path").and_then(|v| v.as_str()),
            Some("/orders")
        );
        assert_eq!(
            get_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("flask")
        );
        assert_eq!(
            get_route.attributes.get("handler").and_then(|v| v.as_str()),
            Some("src/routes/orders.py::get_orders")
        );

        // Check second route
        let post_route = nodes
            .iter()
            .find(|n| n.attributes.get("method").and_then(|v| v.as_str()) == Some("POST"))
            .expect("POST route should be detected");
        assert_eq!(post_route.kind, NodeKind::Route);
        assert_eq!(
            post_route.attributes.get("path").and_then(|v| v.as_str()),
            Some("/orders")
        );
        assert_eq!(
            post_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("flask")
        );
    }

    // -----------------------------------------------------------------------
    // Test: FastAPI route detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_fastapi_route_detected() {
        let source = r#"
from fastapi import FastAPI

app = FastAPI()

@app.get("/api/users")
async def list_users():
    return users

@app.post("/api/users")
async def create_user(user: UserCreate):
    return user

@router.get("/api/items")
async def list_items():
    return items
"#;
        let file = "src/api/users.py";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "python");

        assert_eq!(nodes.len(), 3);

        let get_users = nodes
            .iter()
            .find(|n| {
                n.attributes.get("path").and_then(|v| v.as_str()) == Some("/api/users")
                    && n.attributes.get("method").and_then(|v| v.as_str()) == Some("GET")
            })
            .expect("GET /api/users should be detected");
        assert_eq!(
            get_users
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("fastapi")
        );
        assert_eq!(
            get_users.attributes.get("handler").and_then(|v| v.as_str()),
            Some("src/api/users.py::list_users")
        );
    }

    // -----------------------------------------------------------------------
    // Test: Express route detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_express_route_detected() {
        let source = r#"
const express = require('express');
const app = express();

app.get("/users", (req, res) => {
    res.json(users);
});

app.post("/users", createUser);

router.get("/products", listProducts);
"#;
        let file = "src/routes/users.ts";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "typescript");

        assert_eq!(nodes.len(), 3);

        let get_users = nodes
            .iter()
            .find(|n| {
                n.attributes.get("path").and_then(|v| v.as_str()) == Some("/users")
                    && n.attributes.get("method").and_then(|v| v.as_str()) == Some("GET")
            })
            .expect("GET /users should be detected");
        assert_eq!(get_users.kind, NodeKind::Route);
        assert_eq!(
            get_users
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("express")
        );

        let post_users = nodes
            .iter()
            .find(|n| {
                n.attributes.get("path").and_then(|v| v.as_str()) == Some("/users")
                    && n.attributes.get("method").and_then(|v| v.as_str()) == Some("POST")
            })
            .expect("POST /users should be detected");
        assert_eq!(post_users.kind, NodeKind::Route);

        let get_products = nodes
            .iter()
            .find(|n| n.attributes.get("path").and_then(|v| v.as_str()) == Some("/products"))
            .expect("GET /products should be detected");
        assert_eq!(get_products.kind, NodeKind::Route);
    }

    // -----------------------------------------------------------------------
    // Test: Gin route detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_gin_route_detected() {
        let source = r#"
package main

import "github.com/gin-gonic/gin"

func main() {
    r := gin.Default()
    r.GET("/api/health", healthCheck)
    r.POST("/api/orders", createOrder)
}
"#;
        let file = "cmd/server/main.go";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "go");

        assert_eq!(nodes.len(), 2);

        let get_health = nodes
            .iter()
            .find(|n| n.attributes.get("path").and_then(|v| v.as_str()) == Some("/api/health"))
            .expect("GET /api/health should be detected");
        assert_eq!(get_health.kind, NodeKind::Route);
        assert_eq!(
            get_health.attributes.get("method").and_then(|v| v.as_str()),
            Some("GET")
        );
        assert_eq!(
            get_health
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("gin")
        );
    }

    // -----------------------------------------------------------------------
    // Test: Echo route detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_echo_route_detected() {
        let source = r#"
package main

import "github.com/labstack/echo"

func main() {
    e := echo.New()
    e.GET("/users", getUsers)
    e.DELETE("/users/:id", deleteUser)
}
"#;
        let file = "cmd/api/main.go";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "go");

        assert_eq!(nodes.len(), 2);

        let get_users = nodes
            .iter()
            .find(|n| n.attributes.get("path").and_then(|v| v.as_str()) == Some("/users"))
            .expect("GET /users should be detected");
        assert_eq!(
            get_users
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("echo")
        );
    }

    // -----------------------------------------------------------------------
    // Test: HTTP client creates HttpLink
    // -----------------------------------------------------------------------

    #[test]
    fn test_http_client_creates_httplink() {
        let source = r#"
import requests

def fetch_orders():
    response = requests.get("http://localhost:8000/orders")
    return response.json()

def create_order(data):
    response = requests.post("http://localhost:8000/orders")
    return response.json()
"#;
        let file = "src/client.py";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Pre-populate with a matching route node
        nodes.push(Node {
            fqn: "src/routes/orders.py::route::GET:/orders".to_string(),
            kind: NodeKind::Route,
            file: "src/routes/orders.py".to_string(),
            start_line: 5,
            end_line: 5,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({
                "method": "GET",
                "path": "/orders",
                "framework": "flask",
            }),
        });

        detect_routes(&mut nodes, &mut edges, file, source, "python");

        // Should have created HttpLink edges
        let http_links: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::HttpLink)
            .collect();
        assert!(
            !http_links.is_empty(),
            "HttpLink edges should be created for matching client calls"
        );

        // Check the first HttpLink
        let link = http_links
            .iter()
            .find(|e| e.source_fqn == "src/client.py::fetch_orders")
            .expect("HttpLink from fetch_orders should exist");
        assert_eq!(link.target_fqn, "src/routes/orders.py::route::GET:/orders");
        assert_eq!(link.confidence, 1.0); // Exact path match
    }

    // -----------------------------------------------------------------------
    // Test: JavaScript HTTP client creates HttpLink
    // -----------------------------------------------------------------------

    #[test]
    fn test_js_http_client_creates_httplink() {
        let source = r#"
async function loadUsers() {
    const response = await fetch("http://api.example.com/users");
    return response.json();
}

async function loadItems() {
    const response = await axios.get("http://api.example.com/items");
    return response.json();
}
"#;
        let file = "src/api/client.ts";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Pre-populate with matching route nodes
        nodes.push(Node {
            fqn: "src/routes/users.ts::route::GET:/users".to_string(),
            kind: NodeKind::Route,
            file: "src/routes/users.ts".to_string(),
            start_line: 3,
            end_line: 3,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({
                "method": "GET",
                "path": "/users",
                "framework": "express",
            }),
        });

        detect_routes(&mut nodes, &mut edges, file, source, "typescript");

        let http_links: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::HttpLink)
            .collect();
        assert!(
            !http_links.is_empty(),
            "HttpLink edges should be created for fetch/axios calls"
        );

        let fetch_link = http_links
            .iter()
            .find(|e| e.source_fqn == "src/api/client.ts::loadUsers")
            .expect("HttpLink from loadUsers should exist");
        assert_eq!(
            fetch_link.target_fqn,
            "src/routes/users.ts::route::GET:/users"
        );
        assert_eq!(fetch_link.confidence, 1.0);
    }

    // -----------------------------------------------------------------------
    // Test: Non-route decorators are not falsely detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_non_route_decorators_not_detected() {
        let source = r#"
from functools import wraps

@wraps(func)
def decorator(f):
    return f

@property
def name(self):
    return self._name

@staticmethod
def helper():
    pass
"#;
        let file = "src/utils.py";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "python");

        assert!(
            nodes.is_empty(),
            "Non-route decorators should not create Route nodes"
        );
        assert!(edges.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: Parameterized route matching
    // -----------------------------------------------------------------------

    #[test]
    fn test_parameterized_route_matching() {
        assert!(paths_match_parameterized("/users/:id", "/users/123"));
        assert!(paths_match_parameterized("/users/{id}", "/users/123"));
        assert!(paths_match_parameterized(
            "/users/:id/orders/:order_id",
            "/users/1/orders/42"
        ));
        assert!(!paths_match_parameterized("/users/:id", "/products/123"));
        assert!(!paths_match_parameterized("/users/:id", "/users/123/extra"));
        // Non-parameterized paths should not match via this function
        assert!(!paths_match_parameterized("/users", "/users"));
    }

    // -----------------------------------------------------------------------
    // Test: URL path extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_path_from_url() {
        assert_eq!(
            extract_path_from_url("http://localhost:8000/api/orders"),
            "/api/orders"
        );
        assert_eq!(
            extract_path_from_url("https://api.example.com/users"),
            "/users"
        );
        assert_eq!(extract_path_from_url("/api/orders"), "/api/orders");
        assert_eq!(extract_path_from_url("http://localhost"), "/");
    }

    // -----------------------------------------------------------------------
    // Test: Unsupported language produces no results
    // -----------------------------------------------------------------------

    #[test]
    fn test_unsupported_language_no_results() {
        let source = "some random code";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, "file.rb", source, "ruby");

        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: Go HTTP client creates HttpLink
    // -----------------------------------------------------------------------

    #[test]
    fn test_go_http_client_creates_httplink() {
        let source = r#"
package main

import "net/http"

func fetchData() {
    resp, err := http.Get("http://localhost:8080/api/data")
    if err != nil {
        return
    }
    defer resp.Body.Close()
}
"#;
        let file = "cmd/client/main.go";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Pre-populate with a matching route
        nodes.push(Node {
            fqn: "cmd/server/main.go::route::GET:/api/data".to_string(),
            kind: NodeKind::Route,
            file: "cmd/server/main.go".to_string(),
            start_line: 8,
            end_line: 8,
            file_hash: String::new(),
            indexed_at: 0,
            attributes: json!({
                "method": "GET",
                "path": "/api/data",
                "framework": "gin",
            }),
        });

        detect_routes(&mut nodes, &mut edges, file, source, "go");

        let http_links: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::HttpLink)
            .collect();
        assert!(
            !http_links.is_empty(),
            "HttpLink edge should be created for http.Get call"
        );

        let link = &http_links[0];
        assert_eq!(link.source_fqn, "cmd/client/main.go::fetchData");
        assert_eq!(link.target_fqn, "cmd/server/main.go::route::GET:/api/data");
        assert_eq!(link.confidence, 1.0);
    }

    // -----------------------------------------------------------------------
    // Test: Django routes detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_django_routes_detected() {
        let source = r#"
from django.urls import path
from . import views

urlpatterns = [
    path("api/users/", views.user_list, name="user-list"),
    path("api/users/<int:pk>/", views.user_detail, name="user-detail"),
    path("api/orders/", views.order_list),
]
"#;
        let file = "myapp/urls.py";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "python");

        // Should detect Django routes (in addition to any FastAPI/Flask routes)
        let django_routes: Vec<&Node> = nodes
            .iter()
            .filter(|n| {
                n.kind == NodeKind::Route
                    && n.attributes.get("framework").and_then(|v| v.as_str()) == Some("django")
            })
            .collect();

        assert!(
            django_routes.len() >= 3,
            "Should detect at least 3 Django routes, found {}",
            django_routes.len()
        );

        // Check that paths are normalized with leading /
        let paths: Vec<&str> = django_routes
            .iter()
            .filter_map(|n| n.attributes.get("path").and_then(|v| v.as_str()))
            .collect();
        assert!(paths.contains(&"/api/users/"));
        assert!(paths.contains(&"/api/orders/"));
    }

    // -----------------------------------------------------------------------
    // Test: Spring routes detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_spring_routes_detected() {
        let source = r#"
package com.example.controller;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api")
public class UserController {

    @GetMapping("/users")
    public List<User> getUsers() {
        return userService.findAll();
    }

    @PostMapping("/users")
    public User createUser(@RequestBody User user) {
        return userService.save(user);
    }

    @DeleteMapping("/users/{id}")
    public void deleteUser(@PathVariable Long id) {
        userService.deleteById(id);
    }
}
"#;
        let file = "src/main/java/com/example/controller/UserController.java";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "java");

        assert!(
            nodes.len() >= 3,
            "Should detect at least 3 Spring routes, found {}",
            nodes.len()
        );

        // Check GET route
        let get_route = nodes
            .iter()
            .find(|n| {
                n.attributes.get("method").and_then(|v| v.as_str()) == Some("GET")
                    && n.attributes.get("path").and_then(|v| v.as_str()) == Some("/users")
            })
            .expect("GET /users should be detected");
        assert_eq!(
            get_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("spring")
        );

        // Check POST route
        let post_route = nodes
            .iter()
            .find(|n| {
                n.attributes.get("method").and_then(|v| v.as_str()) == Some("POST")
                    && n.attributes.get("path").and_then(|v| v.as_str()) == Some("/users")
            })
            .expect("POST /users should be detected");
        assert_eq!(
            post_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("spring")
        );

        // Check DELETE route
        let delete_route = nodes
            .iter()
            .find(|n| n.attributes.get("method").and_then(|v| v.as_str()) == Some("DELETE"))
            .expect("DELETE route should be detected");
        assert_eq!(
            delete_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("spring")
        );
    }

    // -----------------------------------------------------------------------
    // Test: Rails routes detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_rails_routes_detected() {
        let source = r#"
Rails.application.routes.draw do
  resources :orders
  resources :users

  get "/health", to: "health#check"
  post "/api/login", to: "sessions#create"
end
"#;
        let file = "config/routes.rb";
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        detect_routes(&mut nodes, &mut edges, file, source, "ruby");

        assert!(
            nodes.len() >= 4,
            "Should detect at least 4 Rails routes, found {}",
            nodes.len()
        );

        // Check resources route
        let orders_route = nodes
            .iter()
            .find(|n| n.attributes.get("path").and_then(|v| v.as_str()) == Some("/orders"))
            .expect("resources :orders should be detected");
        assert_eq!(
            orders_route
                .attributes
                .get("framework")
                .and_then(|v| v.as_str()),
            Some("rails")
        );

        // Check verb route
        let health_route = nodes
            .iter()
            .find(|n| n.attributes.get("path").and_then(|v| v.as_str()) == Some("/health"))
            .expect("get '/health' should be detected");
        assert_eq!(
            health_route
                .attributes
                .get("method")
                .and_then(|v| v.as_str()),
            Some("GET")
        );
    }
}
