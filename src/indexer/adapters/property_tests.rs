//! Property-based tests for framework adapter edge correctness.
//!
//! **Feature: cortex-intelligence-overhaul**
//! **Property: Framework adapter edge correctness**
//!
//! For any source code file containing a recognized framework pattern
//! (FastAPI Depends, Express app.use, NestJS @Injectable, Spring @Autowired,
//! Django urlpatterns path(), React JSX render), the corresponding framework
//! adapter SHALL create an edge with the correct kind (Injects, Middleware,
//! Routes, or Renders), edge_source=framework_adapter, and confidence=0.8.
//!
//! **Validates: Requirements 4.1, 4.2, 5.1, 5.4, 6.1, 7.1, 8.1**

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::indexer::adapters::django::DjangoAdapter;
    use crate::indexer::adapters::express::ExpressAdapter;
    use crate::indexer::adapters::fastapi::FastApiAdapter;
    use crate::indexer::adapters::nestjs::NestJsAdapter;
    use crate::indexer::adapters::react::ReactAdapter;
    use crate::indexer::adapters::spring::SpringAdapter;
    use crate::indexer::adapters::FrameworkAdapter;
    use crate::store::confidence::EdgeSource;
    use crate::store::types::EdgeKind;

    /// Strategy to generate valid Python/JS/Java identifiers.
    /// Starts with a letter or underscore, followed by alphanumeric or underscore.
    fn arb_identifier() -> impl Strategy<Value = String> {
        // Start with a letter (a-z), then 2-15 lowercase alphanumeric chars
        ("[a-z][a-z0-9_]{2,15}").prop_filter("must not be a keyword", |s| {
            !matches!(
                s.as_str(),
                "def" | "class"
                    | "import"
                    | "from"
                    | "return"
                    | "pass"
                    | "async"
                    | "await"
                    | "function"
                    | "const"
                    | "let"
                    | "var"
                    | "new"
                    | "this"
                    | "super"
                    | "null"
                    | "true"
                    | "false"
                    | "if"
                    | "else"
                    | "for"
                    | "while"
                    | "do"
                    | "switch"
                    | "case"
                    | "break"
                    | "continue"
                    | "throw"
                    | "try"
                    | "catch"
                    | "finally"
                    | "typeof"
                    | "instanceof"
                    | "void"
                    | "delete"
                    | "in"
                    | "of"
                    | "use"
                    | "get"
                    | "post"
                    | "put"
                    | "patch"
                    | "options"
                    | "head"
                    | "all"
            )
        })
    }

    /// Strategy to generate PascalCase identifiers (for React components, Java classes).
    fn arb_pascal_case() -> impl Strategy<Value = String> {
        "[A-Z][a-z]{2,10}[A-Z][a-z]{2,10}".prop_filter("must not be a keyword", |s| {
            !matches!(
                s.as_str(),
                "Controller" | "Injectable" | "Component" | "Service" | "Repository"
            )
        })
    }

    /// Helper to create a minimal tree-sitter tree for Python source.
    fn make_python_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// Helper to create a minimal tree-sitter tree for JavaScript source.
    fn make_js_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// Helper to create a minimal tree-sitter tree for TypeScript source.
    fn make_ts_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// Helper to create a minimal tree-sitter tree for Java source.
    fn make_java_tree(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// Asserts that an edge has the correct framework adapter metadata:
    /// - edge_source == FrameworkAdapter
    /// - confidence == 0.8
    /// - kind matches expected
    fn assert_framework_edge(
        edge: &crate::store::types::Edge,
        expected_kind: &EdgeKind,
    ) -> Result<(), proptest::test_runner::TestCaseError> {
        prop_assert!(
            edge.edge_source == EdgeSource::FrameworkAdapter,
            "edge_source should be FrameworkAdapter, got {:?}",
            &edge.edge_source
        );
        prop_assert!(
            (edge.confidence - 0.8).abs() < f64::EPSILON,
            "confidence should be 0.8, got {}",
            edge.confidence
        );
        prop_assert!(
            edge.kind == *expected_kind,
            "edge kind should be {:?}, got {:?}",
            expected_kind,
            &edge.kind
        );
        Ok(())
    }

    // ─── FastAPI: Depends → Injects ──────────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 4.1, 4.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_fastapi_depends_creates_injects_edge(
            func_name in arb_identifier(),
            dep_name in arb_identifier(),
        ) {
            // Skip if names collide (would create self-edge)
            prop_assume!(func_name != dep_name);

            let source = format!(
                "from fastapi import Depends\n\ndef {func}(x = Depends({dep})):\n    pass\n",
                func = func_name,
                dep = dep_name,
            );

            let tree = make_python_tree(&source);
            let adapter = FastApiAdapter;
            let edges = adapter.extract_edges("test.py", &source, &tree, &[]);

            // Must produce at least one Injects edge
            let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
            prop_assert!(
                !injects.is_empty(),
                "FastAPI Depends({}) in function {} should produce Injects edge, got {:?}",
                dep_name, func_name, edges
            );

            // Verify all Injects edges have correct metadata
            for edge in &injects {
                assert_framework_edge(edge, &EdgeKind::Injects)?;
            }
        }
    }

    // ─── FastAPI: Route decorators → Routes ──────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 4.1, 4.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_fastapi_route_creates_routes_edge(
            func_name in arb_identifier(),
            method in prop_oneof![
                Just("get"),
                Just("post"),
                Just("put"),
                Just("delete"),
                Just("patch"),
            ],
        ) {
            let source = format!(
                "from fastapi import FastAPI\n\napp = FastAPI()\n\n@app.{method}(\"/test\")\ndef {func}():\n    pass\n",
                method = method,
                func = func_name,
            );

            let tree = make_python_tree(&source);
            let adapter = FastApiAdapter;
            let edges = adapter.extract_edges("test.py", &source, &tree, &[]);

            // Must produce at least one Routes edge
            let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
            prop_assert!(
                !routes.is_empty(),
                "FastAPI @app.{}(\"/test\") def {} should produce Routes edge, got {:?}",
                method, func_name, edges
            );

            // Verify all Routes edges have correct metadata
            for edge in &routes {
                assert_framework_edge(edge, &EdgeKind::Routes)?;
            }
        }
    }

    // ─── Express: app.use → Middleware ───────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 5.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_express_use_creates_middleware_edge(
            middleware_name in arb_identifier(),
        ) {
            let source = format!(
                "const express = require('express');\nconst app = express();\napp.use({mw});\n",
                mw = middleware_name,
            );

            let tree = make_js_tree(&source);
            let adapter = ExpressAdapter;
            let edges = adapter.extract_edges("src/app.js", &source, &tree, &[]);

            // Must produce at least one Middleware edge
            let middleware: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Middleware).collect();
            prop_assert!(
                !middleware.is_empty(),
                "Express app.use({}) should produce Middleware edge, got {:?}",
                middleware_name, edges
            );

            // Verify all Middleware edges have correct metadata
            for edge in &middleware {
                assert_framework_edge(edge, &EdgeKind::Middleware)?;
            }
        }
    }

    // ─── Express: router.get → Routes ────────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 5.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_express_route_creates_routes_edge(
            handler_name in arb_identifier(),
            method in prop_oneof![
                Just("get"),
                Just("post"),
                Just("put"),
                Just("delete"),
                Just("patch"),
            ],
        ) {
            let source = format!(
                "const router = express.Router();\nrouter.{method}('/test', {handler});\n",
                method = method,
                handler = handler_name,
            );

            let tree = make_js_tree(&source);
            let adapter = ExpressAdapter;
            let edges = adapter.extract_edges("src/routes.js", &source, &tree, &[]);

            // Must produce at least one Routes edge
            let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
            prop_assert!(
                !routes.is_empty(),
                "Express router.{}('/test', {}) should produce Routes edge, got {:?}",
                method, handler_name, edges
            );

            // Verify all Routes edges have correct metadata
            for edge in &routes {
                assert_framework_edge(edge, &EdgeKind::Routes)?;
            }
        }
    }

    // ─── NestJS: @Injectable → Injects ───────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 5.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_nestjs_injectable_creates_injects_edge(
            service_name in arb_pascal_case(),
            consumer_name in arb_pascal_case(),
        ) {
            prop_assume!(service_name != consumer_name);

            let source = format!(
                "@Injectable()\nexport class {service} {{\n  findAll() {{ return []; }}\n}}\n\nexport class {consumer} {{\n  constructor(private readonly svc: {service}) {{}}\n}}\n",
                service = service_name,
                consumer = consumer_name,
            );

            let tree = make_ts_tree(&source);
            let adapter = NestJsAdapter;
            let edges = adapter.extract_edges("src/test.ts", &source, &tree, &[]);

            // Must produce at least one Injects edge
            let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
            prop_assert!(
                !injects.is_empty(),
                "NestJS @Injectable() {} with consumer {} should produce Injects edge, got {:?}",
                service_name, consumer_name, edges
            );

            // Verify all Injects edges have correct metadata
            for edge in &injects {
                assert_framework_edge(edge, &EdgeKind::Injects)?;
            }
        }
    }

    // ─── NestJS: @Controller → Routes ────────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 5.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_nestjs_controller_creates_routes_edge(
            controller_name in arb_pascal_case(),
        ) {
            let source = format!(
                "@Controller('test')\nexport class {ctrl} {{\n}}\n",
                ctrl = controller_name,
            );

            let tree = make_ts_tree(&source);
            let adapter = NestJsAdapter;
            let edges = adapter.extract_edges("src/test.controller.ts", &source, &tree, &[]);

            // Must produce at least one Routes edge
            let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
            prop_assert!(
                !routes.is_empty(),
                "NestJS @Controller('test') class {} should produce Routes edge, got {:?}",
                controller_name, edges
            );

            // Verify all Routes edges have correct metadata
            for edge in &routes {
                assert_framework_edge(edge, &EdgeKind::Routes)?;
            }
        }
    }

    // ─── Spring: @Autowired → Injects ────────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 6.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_spring_autowired_creates_injects_edge(
            service_type in arb_pascal_case(),
            field_name in arb_identifier(),
        ) {
            let source = format!(
                "public class MyController {{\n    @Autowired private {svc_type} {field};\n}}\n",
                svc_type = service_type,
                field = field_name,
            );

            let tree = make_java_tree(&source);
            let adapter = SpringAdapter;
            let edges = adapter.extract_edges("src/MyController.java", &source, &tree, &[]);

            // Must produce at least one Injects edge
            let injects: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Injects).collect();
            prop_assert!(
                !injects.is_empty(),
                "Spring @Autowired {} {} should produce Injects edge, got {:?}",
                service_type, field_name, edges
            );

            // Verify all Injects edges have correct metadata
            for edge in &injects {
                assert_framework_edge(edge, &EdgeKind::Injects)?;
            }
        }
    }

    // ─── Django: urlpatterns path() → Routes ─────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 7.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_django_path_creates_routes_edge(
            view_name in arb_identifier(),
        ) {
            let source = format!(
                "from django.urls import path\n\nurlpatterns = [\n    path('test/', {view}),\n]\n",
                view = view_name,
            );

            let tree = make_python_tree(&source);
            let adapter = DjangoAdapter;
            let edges = adapter.extract_edges("urls.py", &source, &tree, &[]);

            // Must produce at least one Routes edge
            let routes: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Routes).collect();
            prop_assert!(
                !routes.is_empty(),
                "Django path('test/', {}) should produce Routes edge, got {:?}",
                view_name, edges
            );

            // Verify all Routes edges have correct metadata
            for edge in &routes {
                assert_framework_edge(edge, &EdgeKind::Routes)?;
            }
        }
    }

    // ─── React: JSX render → Renders ─────────────────────────────────────────

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Framework adapter edge correctness**
    // **Validates: Requirements 8.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn property_adapter_react_jsx_creates_renders_edge(
            component_name in arb_pascal_case(),
        ) {
            let source = format!(
                "function App() {{\n  return <{comp} />;\n}}\n",
                comp = component_name,
            );

            let tree = make_js_tree(&source);
            let adapter = ReactAdapter;
            let edges = adapter.extract_edges("src/App.jsx", &source, &tree, &[]);

            // Must produce at least one Renders edge
            let renders: Vec<_> = edges.iter().filter(|e| e.kind == EdgeKind::Renders).collect();
            prop_assert!(
                !renders.is_empty(),
                "React JSX <{} /> should produce Renders edge, got {:?}",
                component_name, edges
            );

            // Verify all Renders edges have correct metadata
            for edge in &renders {
                assert_framework_edge(edge, &EdgeKind::Renders)?;
            }
        }
    }
}
