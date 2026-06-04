//! NestJS framework adapter.
//!
//! Detects `@Controller()` → Routes edges from module to controller.
//! Detects `@Injectable()` → Injects edges from constructor consumers to service.
//!
//! # Patterns Detected
//!
//! 1. **Controller registration**: `@Controller('path')` on a class creates a
//!    `Routes` edge from the module (file) to the controller class.
//!
//! 2. **Injectable services**: `@Injectable()` on a class marks it as a service.
//!    When another class declares it as a constructor parameter
//!    (`constructor(private readonly x: ServiceName)`), an `Injects` edge is
//!    created from the consumer class to the injectable service.

use regex::Regex;
use serde_json::json;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node};

use super::FrameworkAdapter;

/// Confidence value for all framework adapter edges (MEDIUM tier = 0.8).
const FRAMEWORK_CONFIDENCE: f64 = 0.8;

/// Adapter for NestJS dependency injection and controller detection.
pub struct NestJsAdapter;

impl FrameworkAdapter for NestJsAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::NestJS
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        _existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // Extract controller routes edges
        edges.extend(extract_controller_routes(file, source));

        // Extract injectable/injection edges
        edges.extend(extract_injection_edges(file, source));

        edges
    }
}

/// Detect `@Controller(...)` decorators and create Routes edges from the
/// module (file-level) to the controller class.
///
/// Pattern: `@Controller('path')` or `@Controller()` followed by `class ClassName`
fn extract_controller_routes(file: &str, source: &str) -> Vec<Edge> {
    let mut edges = Vec::new();

    // Match @Controller(...) followed by a class declaration.
    // The decorator can have optional path argument: @Controller(), @Controller('users'), @Controller("users")
    let controller_re = Regex::new(
        r"@Controller\([^)]*\)\s*(?:export\s+)?class\s+(\w+)",
    )
    .expect("controller regex is valid");

    let module_fqn = format!("{}::module", file);

    for caps in controller_re.captures_iter(source) {
        if let Some(class_name) = caps.get(1) {
            let controller_fqn = format!("{}::{}", file, class_name.as_str());
            edges.push(Edge {
                id: None,
                source_fqn: module_fqn.clone(),
                target_fqn: controller_fqn,
                kind: EdgeKind::Routes,
                confidence: FRAMEWORK_CONFIDENCE,
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: json!({"decorator": "Controller", "framework": "nestjs"}),
            });
        }
    }

    edges
}

/// Detect `@Injectable()` services and constructor injection patterns.
///
/// 1. Find all classes decorated with `@Injectable()` — these are services.
/// 2. Find all constructor parameters in classes that reference injectable types.
/// 3. Create `Injects` edges from the consumer class to the injectable service.
fn extract_injection_edges(file: &str, source: &str) -> Vec<Edge> {
    let mut edges = Vec::new();

    // Step 1: Find all @Injectable() classes
    let injectable_re = Regex::new(
        r"@Injectable\(\)\s*(?:export\s+)?class\s+(\w+)",
    )
    .expect("injectable regex is valid");

    let injectable_classes: Vec<String> = injectable_re
        .captures_iter(source)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .collect();

    // Step 2: Find constructor injection patterns in all classes.
    // Pattern: class ClassName { constructor(...paramType: ServiceName...) }
    // We look for: class X ... constructor( ... paramName: TypeName ... )
    let class_constructor_re = Regex::new(
        r"(?:export\s+)?class\s+(\w+)[^{]*\{[^}]*?constructor\s*\(([^)]*)\)",
    )
    .expect("class constructor regex is valid");

    // Pattern for individual constructor parameters:
    // Matches: `private readonly name: TypeName` or `private name: TypeName`
    // or `readonly name: TypeName` or just `name: TypeName`
    // Also handles access modifiers: private, protected, public
    let param_re = Regex::new(
        r"(?:private|protected|public)?\s*(?:readonly\s+)?(\w+)\s*:\s*(\w+)",
    )
    .expect("param regex is valid");

    for class_caps in class_constructor_re.captures_iter(source) {
        let consumer_class = match class_caps.get(1) {
            Some(m) => m.as_str(),
            None => continue,
        };
        let constructor_params = match class_caps.get(2) {
            Some(m) => m.as_str(),
            None => continue,
        };

        // Parse each parameter in the constructor
        for param_caps in param_re.captures_iter(constructor_params) {
            let type_name = match param_caps.get(2) {
                Some(m) => m.as_str(),
                None => continue,
            };

            // Only create an edge if the type is an @Injectable() class
            if injectable_classes.contains(&type_name.to_string()) {
                let consumer_fqn = format!("{}::{}", file, consumer_class);
                let service_fqn = format!("{}::{}", file, type_name);

                edges.push(Edge {
                    id: None,
                    source_fqn: consumer_fqn,
                    target_fqn: service_fqn,
                    kind: EdgeKind::Injects,
                    confidence: FRAMEWORK_CONFIDENCE,
                    edge_source: EdgeSource::FrameworkAdapter,
                    attributes: json!({
                        "decorator": "Injectable",
                        "framework": "nestjs",
                        "injection_type": "constructor"
                    }),
                });
            }
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to run the adapter on source code without needing a real tree-sitter tree.
    fn extract_edges_from_source(file: &str, source: &str) -> Vec<Edge> {
        let mut edges = Vec::new();
        edges.extend(extract_controller_routes(file, source));
        edges.extend(extract_injection_edges(file, source));
        edges
    }

    // ─── Controller Detection ────────────────────────────────────────────────

    #[test]
    fn detects_controller_with_path() {
        let source = r#"
import { Controller } from '@nestjs/common';

@Controller('users')
export class UsersController {
    // ...
}
"#;
        let edges = extract_edges_from_source("src/users/users.controller.ts", source);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].source_fqn, "src/users/users.controller.ts::module");
        assert_eq!(
            edges[0].target_fqn,
            "src/users/users.controller.ts::UsersController"
        );
        assert_eq!(edges[0].confidence, 0.8);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn detects_controller_without_path() {
        let source = r#"
@Controller()
class AppController {
}
"#;
        let edges = extract_edges_from_source("src/app.controller.ts", source);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(edges[0].target_fqn, "src/app.controller.ts::AppController");
    }

    #[test]
    fn detects_controller_with_double_quotes() {
        let source = r#"
@Controller("products")
export class ProductsController {}
"#;
        let edges = extract_edges_from_source("src/products.controller.ts", source);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Routes);
        assert_eq!(
            edges[0].target_fqn,
            "src/products.controller.ts::ProductsController"
        );
    }

    #[test]
    fn detects_multiple_controllers_in_same_file() {
        let source = r#"
@Controller('users')
export class UsersController {}

@Controller('admin')
export class AdminController {}
"#;
        let edges = extract_edges_from_source("src/controllers.ts", source);

        let route_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        assert_eq!(route_edges.len(), 2);
        assert_eq!(route_edges[0].target_fqn, "src/controllers.ts::UsersController");
        assert_eq!(route_edges[1].target_fqn, "src/controllers.ts::AdminController");
    }

    // ─── Injectable Detection ────────────────────────────────────────────────

    #[test]
    fn detects_injectable_with_constructor_injection() {
        let source = r#"
@Injectable()
export class UsersService {
    findAll() { return []; }
}

@Controller('users')
export class UsersController {
    constructor(private readonly usersService: UsersService) {}
}
"#;
        let edges = extract_edges_from_source("src/users.ts", source);

        // Should have: 1 Routes edge (Controller) + 1 Injects edge (constructor injection)
        let route_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();

        assert_eq!(route_edges.len(), 1);
        assert_eq!(inject_edges.len(), 1);

        assert_eq!(inject_edges[0].source_fqn, "src/users.ts::UsersController");
        assert_eq!(inject_edges[0].target_fqn, "src/users.ts::UsersService");
        assert_eq!(inject_edges[0].confidence, 0.8);
        assert_eq!(inject_edges[0].edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn detects_multiple_constructor_injections() {
        let source = r#"
@Injectable()
export class UsersService {}

@Injectable()
export class AuthService {}

@Controller('users')
export class UsersController {
    constructor(
        private readonly usersService: UsersService,
        private readonly authService: AuthService
    ) {}
}
"#;
        let edges = extract_edges_from_source("src/users.ts", source);

        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges.len(), 2);

        let targets: Vec<&str> = inject_edges.iter().map(|e| e.target_fqn.as_str()).collect();
        assert!(targets.contains(&"src/users.ts::UsersService"));
        assert!(targets.contains(&"src/users.ts::AuthService"));
    }

    #[test]
    fn no_injection_edge_for_non_injectable_type() {
        let source = r#"
export class NotInjectable {}

@Controller('test')
export class TestController {
    constructor(private readonly svc: NotInjectable) {}
}
"#;
        let edges = extract_edges_from_source("src/test.ts", source);

        // Only the Routes edge for the controller, no Injects edge
        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges.len(), 0);
    }

    #[test]
    fn no_edges_for_plain_typescript() {
        let source = r#"
export class PlainClass {
    constructor(private name: string) {}
}

function helper() {
    return 42;
}
"#;
        let edges = extract_edges_from_source("src/plain.ts", source);
        assert!(edges.is_empty());
    }

    #[test]
    fn injectable_without_export() {
        let source = r#"
@Injectable()
class InternalService {}

class Consumer {
    constructor(private svc: InternalService) {}
}
"#;
        let edges = extract_edges_from_source("src/internal.ts", source);

        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges.len(), 1);
        assert_eq!(inject_edges[0].source_fqn, "src/internal.ts::Consumer");
        assert_eq!(inject_edges[0].target_fqn, "src/internal.ts::InternalService");
    }

    #[test]
    fn constructor_with_public_modifier() {
        let source = r#"
@Injectable()
export class LoggerService {}

export class AppService {
    constructor(public logger: LoggerService) {}
}
"#;
        let edges = extract_edges_from_source("src/app.ts", source);

        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges.len(), 1);
        assert_eq!(inject_edges[0].target_fqn, "src/app.ts::LoggerService");
    }

    #[test]
    fn constructor_with_protected_modifier() {
        let source = r#"
@Injectable()
export class BaseService {}

export class DerivedController {
    constructor(protected baseService: BaseService) {}
}
"#;
        let edges = extract_edges_from_source("src/derived.ts", source);

        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges.len(), 1);
        assert_eq!(inject_edges[0].target_fqn, "src/derived.ts::BaseService");
    }

    // ─── Edge Metadata ───────────────────────────────────────────────────────

    #[test]
    fn controller_edge_has_correct_attributes() {
        let source = r#"
@Controller('items')
export class ItemsController {}
"#;
        let edges = extract_edges_from_source("src/items.ts", source);

        assert_eq!(edges[0].attributes["decorator"], "Controller");
        assert_eq!(edges[0].attributes["framework"], "nestjs");
    }

    #[test]
    fn injection_edge_has_correct_attributes() {
        let source = r#"
@Injectable()
export class SomeService {}

export class SomeConsumer {
    constructor(private svc: SomeService) {}
}
"#;
        let edges = extract_edges_from_source("src/some.ts", source);

        let inject_edges: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
        assert_eq!(inject_edges[0].attributes["decorator"], "Injectable");
        assert_eq!(inject_edges[0].attributes["framework"], "nestjs");
        assert_eq!(inject_edges[0].attributes["injection_type"], "constructor");
    }

    // ─── FrameworkAdapter Trait ──────────────────────────────────────────────

    #[test]
    fn adapter_reports_nestjs_framework() {
        let adapter = NestJsAdapter;
        assert_eq!(adapter.framework(), FrameworkKind::NestJS);
    }
}
