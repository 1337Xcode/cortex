//! Django framework adapter.
//!
//! Detects `urlpatterns` with `path()`/`re_path()` → Routes edges.
//! Detects `@login_required`, `@permission_required` → Middleware edges.

use regex::Regex;
use serde_json::json;
use std::sync::LazyLock;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node};

use super::FrameworkAdapter;

/// Confidence value for all framework adapter edges (ConfidenceTier::Medium).
const ADAPTER_CONFIDENCE: f64 = 0.8;

/// Regex matching `path('...', view_func)` or `path("...", view_func)` in urlpatterns.
/// Captures the URL pattern and the view function reference.
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)\bpath\(\s*(?:r?['"]([^'"]*)['"]\s*,\s*)([A-Za-z_][A-Za-z0-9_.]*)"#,
    )
    .expect("invalid path regex")
});

/// Regex matching `re_path(r'...', view_func)` or `re_path("...", view_func)` in urlpatterns.
static RE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)\bre_path\(\s*(?:r?['"]([^'"]*)['"]\s*,\s*)([A-Za-z_][A-Za-z0-9_.]*)"#,
    )
    .expect("invalid re_path regex")
});

/// Regex matching `@login_required` decorator on a function.
/// Captures the function name on the next `def` line.
static LOGIN_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)@login_required\b[^\n]*\n(?:\s*@[^\n]*\n)*\s*def\s+([A-Za-z_][A-Za-z0-9_]*)"#)
        .expect("invalid login_required regex")
});

/// Regex matching `@permission_required(...)` decorator on a function.
/// Captures the function name on the next `def` line.
static PERMISSION_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)@permission_required\b[^\n]*\n(?:\s*@[^\n]*\n)*\s*def\s+([A-Za-z_][A-Za-z0-9_]*)"#,
    )
    .expect("invalid permission_required regex")
});

/// Adapter for Django URL routing and decorator detection.
pub struct DjangoAdapter;

impl DjangoAdapter {
    /// Build a fully-qualified name for a symbol in the given file.
    fn make_fqn(file: &str, name: &str) -> String {
        format!("{}::{}", file, name)
    }

    /// Extract Routes edges from `path()` and `re_path()` calls in urlpatterns.
    fn extract_route_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        // The source FQN for routes is the URL configuration module itself.
        let module_fqn = file.to_string();

        for cap in PATH_RE.captures_iter(source) {
            let view_ref = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            if !view_ref.is_empty() {
                let url_pattern = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                edges.push(Edge {
                    id: None,
                    source_fqn: module_fqn.clone(),
                    target_fqn: Self::resolve_view_fqn(file, view_ref),
                    kind: EdgeKind::Routes,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({ "url_pattern": url_pattern }),
                });
            }
        }

        for cap in RE_PATH_RE.captures_iter(source) {
            let view_ref = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            if !view_ref.is_empty() {
                let url_pattern = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                edges.push(Edge {
                    id: None,
                    source_fqn: module_fqn.clone(),
                    target_fqn: Self::resolve_view_fqn(file, view_ref),
                    kind: EdgeKind::Routes,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({ "url_pattern": url_pattern }),
                });
            }
        }

        edges
    }

    /// Extract Middleware edges from `@login_required` and `@permission_required` decorators.
    fn extract_middleware_edges(&self, file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in LOGIN_REQUIRED_RE.captures_iter(source) {
            let func_name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if !func_name.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: "django.contrib.auth.decorators::login_required".to_string(),
                    target_fqn: Self::make_fqn(file, func_name),
                    kind: EdgeKind::Middleware,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({ "decorator": "login_required" }),
                });
            }
        }

        for cap in PERMISSION_REQUIRED_RE.captures_iter(source) {
            let func_name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if !func_name.is_empty() {
                edges.push(Edge {
                    id: None,
                    source_fqn: "django.contrib.auth.decorators::permission_required".to_string(),
                    target_fqn: Self::make_fqn(file, func_name),
                    kind: EdgeKind::Middleware,
                    confidence: ADAPTER_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({ "decorator": "permission_required" }),
                });
            }
        }

        edges
    }

    /// Resolve a view function reference to a fully-qualified name.
    ///
    /// If the reference contains a dot (e.g., `views.home`), it's treated as a
    /// module-qualified reference. Otherwise, it's local to the current file.
    fn resolve_view_fqn(file: &str, view_ref: &str) -> String {
        if view_ref.contains('.') {
            // Module-qualified reference: convert dots to path separators.
            // e.g., "views.home" → "views::home"
            view_ref.replace('.', "::")
        } else {
            // Local reference within the same file.
            format!("{}::{}", file, view_ref)
        }
    }
}

impl FrameworkAdapter for DjangoAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::Django
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        _existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();
        edges.extend(self.extract_route_edges(file, source));
        edges.extend(self.extract_middleware_edges(file, source));
        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_path_routes() {
        let source = r#"
from django.urls import path
from . import views

urlpatterns = [
    path('home/', views.home),
    path('about/', views.about),
    path('users/<int:id>/', views.user_detail),
]
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("myapp/urls.py", source);

        assert_eq!(edges.len(), 3);

        assert_eq!(edges[0].source_fqn, "myapp/urls.py");
        assert_eq!(edges[0].target_fqn, "views::home");
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].confidence, 0.8);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);

        assert_eq!(edges[1].target_fqn, "views::about");
        assert_eq!(edges[2].target_fqn, "views::user_detail");
    }

    #[test]
    fn detects_re_path_routes() {
        let source = r#"
from django.urls import re_path
from . import views

urlpatterns = [
    re_path(r'^articles/(?P<year>[0-9]{4})/$', views.year_archive),
    re_path(r'^blog/', views.blog_index),
]
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("blog/urls.py", source);

        assert_eq!(edges.len(), 2);

        assert_eq!(edges[0].source_fqn, "blog/urls.py");
        assert_eq!(edges[0].target_fqn, "views::year_archive");
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].confidence, 0.8);

        assert_eq!(edges[1].target_fqn, "views::blog_index");
    }

    #[test]
    fn detects_local_view_references() {
        let source = r#"
urlpatterns = [
    path('', home_view),
    path('login/', login_view),
]
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("urls.py", source);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].target_fqn, "urls.py::home_view");
        assert_eq!(edges[1].target_fqn, "urls.py::login_view");
    }

    #[test]
    fn detects_login_required_decorator() {
        let source = r#"
from django.contrib.auth.decorators import login_required

@login_required
def dashboard(request):
    return render(request, 'dashboard.html')

@login_required
def profile(request):
    return render(request, 'profile.html')
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_middleware_edges("views.py", source);

        assert_eq!(edges.len(), 2);

        assert_eq!(
            edges[0].source_fqn,
            "django.contrib.auth.decorators::login_required"
        );
        assert_eq!(edges[0].target_fqn, "views.py::dashboard");
        assert_eq!(edges[0].kind, EdgeKind::Middleware);
        assert_eq!(edges[0].confidence, 0.8);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);

        assert_eq!(edges[1].target_fqn, "views.py::profile");
    }

    #[test]
    fn detects_permission_required_decorator() {
        let source = r#"
from django.contrib.auth.decorators import permission_required

@permission_required('polls.add_choice')
def add_choice(request):
    pass

@permission_required('admin.can_edit', raise_exception=True)
def admin_edit(request):
    pass
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_middleware_edges("polls/views.py", source);

        assert_eq!(edges.len(), 2);

        assert_eq!(
            edges[0].source_fqn,
            "django.contrib.auth.decorators::permission_required"
        );
        assert_eq!(edges[0].target_fqn, "polls/views.py::add_choice");
        assert_eq!(edges[0].kind, EdgeKind::Middleware);
        assert_eq!(edges[0].confidence, 0.8);

        assert_eq!(edges[1].target_fqn, "polls/views.py::admin_edit");
    }

    #[test]
    fn handles_stacked_decorators() {
        let source = r#"
@login_required
@permission_required('app.view_item')
def protected_view(request):
    pass
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_middleware_edges("views.py", source);

        // login_required should find protected_view (skipping the stacked decorator)
        // permission_required should also find protected_view
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].target_fqn, "views.py::protected_view");
        assert_eq!(edges[0].attributes["decorator"], "login_required");
        assert_eq!(edges[1].target_fqn, "views.py::protected_view");
        assert_eq!(edges[1].attributes["decorator"], "permission_required");
    }

    #[test]
    fn extract_edges_combines_routes_and_middleware() {
        let source = r#"
from django.urls import path
from django.contrib.auth.decorators import login_required

@login_required
def my_view(request):
    pass

urlpatterns = [
    path('my/', my_view),
]
"#;

        let adapter = DjangoAdapter;
        // We need a tree-sitter tree for the trait method, but we don't use it.
        // Create a minimal Python parse for the test.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        let edges = adapter.extract_edges("app/views.py", source, &tree, &[]);

        // Should have 1 route + 1 middleware edge
        assert_eq!(edges.len(), 2);

        let route_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        let middleware_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Middleware)
            .collect();

        assert_eq!(route_edges.len(), 1);
        assert_eq!(middleware_edges.len(), 1);

        assert_eq!(route_edges[0].target_fqn, "app/views.py::my_view");
        assert_eq!(middleware_edges[0].target_fqn, "app/views.py::my_view");
    }

    #[test]
    fn no_edges_for_unrelated_code() {
        let source = r#"
def hello():
    print("Hello, world!")

class MyModel:
    name = models.CharField(max_length=100)
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("models.py", source);
        assert!(edges.is_empty());

        let edges = adapter.extract_middleware_edges("models.py", source);
        assert!(edges.is_empty());
    }

    #[test]
    fn empty_source_produces_no_edges() {
        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("urls.py", "");
        assert!(edges.is_empty());

        let edges = adapter.extract_middleware_edges("views.py", "");
        assert!(edges.is_empty());
    }

    #[test]
    fn path_with_double_quotes() {
        let source = r#"
urlpatterns = [
    path("api/v1/users/", user_list),
]
"#;

        let adapter = DjangoAdapter;
        let edges = adapter.extract_route_edges("api/urls.py", source);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_fqn, "api/urls.py::user_list");
        assert_eq!(edges[0].attributes["url_pattern"], "api/v1/users/");
    }

    #[test]
    fn framework_returns_django() {
        let adapter = DjangoAdapter;
        assert_eq!(adapter.framework(), FrameworkKind::Django);
    }
}
