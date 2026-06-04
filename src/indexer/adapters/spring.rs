//! Spring framework adapter.
//!
//! Detects `@Autowired`, `@Inject` → Injects edges.
//! Detects `@Component`, `@Service`, `@Repository`, `@Controller` → marks injectable.
//! Detects `@Bean` methods → Injects edges from dependents to factory.

use regex::Regex;
use std::sync::LazyLock;

use crate::indexer::framework_detect::FrameworkKind;
use crate::store::confidence::EdgeSource;
use crate::store::types::{Edge, EdgeKind, Node, NodeKind};

use super::FrameworkAdapter;

/// Confidence value for all framework adapter edges.
const FRAMEWORK_CONFIDENCE: f64 = 0.8;

// ─── Regex patterns ──────────────────────────────────────────────────────────

/// Matches `@Autowired` or `@Inject` followed by a field declaration with a type.
/// Captures: (1) the type name being injected.
/// Example: `@Autowired private UserService userService;`
static AUTOWIRED_FIELD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@(?:Autowired|Inject)\s+(?:private|protected|public)?\s*(?:final\s+)?([A-Z]\w*(?:<[^>]*>)?)\s+\w+\s*;"
    )
    .unwrap()
});

/// Matches constructor injection: a constructor parameter with a type.
/// Used when we detect `@Autowired` on a constructor.
/// Captures: (1) the type name of each parameter.
/// Example: `@Autowired public MyClass(UserService userService, OrderRepo orderRepo)`
static AUTOWIRED_CONSTRUCTOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@(?:Autowired|Inject)\s+(?:public|protected|private)?\s*\w+\s*\(([^)]+)\)"
    )
    .unwrap()
});

/// Matches a single constructor parameter type.
/// Captures: (1) the type name.
static CONSTRUCTOR_PARAM_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:@\w+\s+)*(?:final\s+)?([A-Z]\w*(?:<[^>]*>)?)\s+\w+").unwrap()
});

/// Matches class declarations annotated with `@Component`, `@Service`, `@Repository`, or `@Controller`.
/// Captures: (1) the annotation name, (2) the class name.
static INJECTABLE_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@(Component|Service|Repository|Controller)\b[^\n]*\n(?:\s*@\w+[^\n]*\n)*\s*(?:public\s+)?(?:abstract\s+)?class\s+(\w+)"
    )
    .unwrap()
});

/// Matches `@Bean` method declarations.
/// Captures: (1) the return type, (2) the method name.
/// Example: `@Bean public DataSource dataSource() {`
static BEAN_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@Bean\b[^\n]*\n\s*(?:public|protected|private)?\s*([A-Z]\w*(?:<[^>]*>)?)\s+(\w+)\s*\("
    )
    .unwrap()
});

/// Matches a class declaration to determine the containing class for a given position.
/// Captures: (1) the class name.
static CLASS_DECL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:public\s+|private\s+|protected\s+)?(?:abstract\s+)?class\s+(\w+)").unwrap()
});

/// Adapter for Spring dependency injection and bean detection.
pub struct SpringAdapter;

impl SpringAdapter {
    /// Find the class name that contains the given byte offset in the source.
    /// Walks backwards through class declarations to find the enclosing class.
    fn find_containing_class(source: &str, offset: usize) -> Option<String> {
        let mut last_class: Option<String> = None;
        for cap in CLASS_DECL_RE.captures_iter(source) {
            let m = cap.get(0).unwrap();
            if m.start() > offset {
                break;
            }
            last_class = Some(cap[1].to_string());
        }
        last_class
    }

    /// Strip generic type parameters from a type name.
    /// e.g., `List<String>` → `List`
    fn strip_generics(type_name: &str) -> &str {
        type_name.split('<').next().unwrap_or(type_name)
    }

    /// Build an FQN for a class in the given file.
    fn class_fqn(file: &str, class_name: &str) -> String {
        format!("{}::{}", file, class_name)
    }

    /// Build an FQN for a method in the given file and class.
    fn method_fqn(file: &str, class_name: &str, method_name: &str) -> String {
        format!("{}::{}::{}", file, class_name, method_name)
    }

    /// Try to resolve a type name to an FQN using existing nodes.
    /// Falls back to just the type name if no matching node is found.
    fn resolve_type_fqn(type_name: &str, existing_nodes: &[Node]) -> String {
        // Look for a node whose FQN ends with ::TypeName and is a Class
        let suffix = format!("::{}", type_name);
        existing_nodes
            .iter()
            .find(|n| {
                n.fqn.ends_with(&suffix)
                    && matches!(
                        n.kind,
                        NodeKind::Class | NodeKind::Interface | NodeKind::Type
                    )
            })
            .map(|n| n.fqn.clone())
            .unwrap_or_else(|| type_name.to_string())
    }

    /// Create an Injects edge with framework adapter source and 0.8 confidence.
    fn make_injects_edge(source_fqn: String, target_fqn: String) -> Edge {
        Edge {
            id: None,
            source_fqn,
            target_fqn,
            kind: EdgeKind::Injects,
            confidence: FRAMEWORK_CONFIDENCE,
            edge_source: EdgeSource::FrameworkAdapter,
            attributes: serde_json::json!({}),
        }
    }

    /// Detect `@Autowired` / `@Inject` on fields and create Injects edges.
    fn detect_autowired_fields(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in AUTOWIRED_FIELD_RE.captures_iter(source) {
            let type_name = Self::strip_generics(&cap[1]);
            let offset = cap.get(0).unwrap().start();

            if let Some(class_name) = Self::find_containing_class(source, offset) {
                let source_fqn = Self::class_fqn(file, &class_name);
                let target_fqn = Self::resolve_type_fqn(type_name, existing_nodes);
                edges.push(Self::make_injects_edge(source_fqn, target_fqn));
            }
        }

        edges
    }

    /// Detect `@Autowired` / `@Inject` on constructors and create Injects edges.
    fn detect_autowired_constructors(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in AUTOWIRED_CONSTRUCTOR_RE.captures_iter(source) {
            let params_str = &cap[1];
            let offset = cap.get(0).unwrap().start();

            if let Some(class_name) = Self::find_containing_class(source, offset) {
                let source_fqn = Self::class_fqn(file, &class_name);

                // Extract each parameter type from the constructor
                for param_cap in CONSTRUCTOR_PARAM_TYPE_RE.captures_iter(params_str) {
                    let type_name = Self::strip_generics(&param_cap[1]);
                    let target_fqn = Self::resolve_type_fqn(type_name, existing_nodes);
                    edges.push(Self::make_injects_edge(source_fqn.clone(), target_fqn));
                }
            }
        }

        edges
    }

    /// Detect `@Bean` methods and create Injects edges from dependents to the factory method.
    fn detect_bean_methods(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in BEAN_METHOD_RE.captures_iter(source) {
            let return_type = Self::strip_generics(&cap[1]);
            let method_name = &cap[2];
            let offset = cap.get(0).unwrap().start();

            if let Some(class_name) = Self::find_containing_class(source, offset) {
                let bean_fqn = Self::method_fqn(file, &class_name, method_name);

                // Find all classes that inject this return type (via @Autowired fields)
                // and create edges from those classes to this bean factory method.
                let type_suffix = format!("::{}", return_type);
                for node in existing_nodes {
                    if matches!(node.kind, NodeKind::Class)
                        && node.fqn.ends_with(&type_suffix)
                    {
                        // Any class that depends on this type should have an edge
                        // to the bean factory. We look for existing Injects edges
                        // targeting this type and redirect them to the bean.
                        // For now, create an edge from the bean return type to the factory.
                        edges.push(Self::make_injects_edge(
                            node.fqn.clone(),
                            bean_fqn.clone(),
                        ));
                    }
                }

                // Also: any class that has an @Autowired field of this return type
                // should get an edge to the bean factory. We scan for those.
                let autowired_pattern = format!(
                    r"@(?:Autowired|Inject)\s+(?:private|protected|public)?\s*(?:final\s+)?{}\s+\w+\s*;",
                    regex::escape(return_type)
                );
                if let Ok(re) = Regex::new(&autowired_pattern) {
                    for m in re.find_iter(source) {
                        if let Some(consumer_class) =
                            Self::find_containing_class(source, m.start())
                        {
                            let consumer_fqn = Self::class_fqn(file, &consumer_class);
                            // Avoid duplicate edges (the class→type edge is already created
                            // by detect_autowired_fields; here we add class→bean_factory)
                            if !edges.iter().any(|e| {
                                e.source_fqn == consumer_fqn && e.target_fqn == bean_fqn
                            }) {
                                edges.push(Self::make_injects_edge(
                                    consumer_fqn,
                                    bean_fqn.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        edges
    }

    /// Detect injectable classes (`@Component`, `@Service`, `@Repository`, `@Controller`)
    /// and create edges from classes that inject them.
    fn detect_injectable_classes(
        &self,
        file: &str,
        source: &str,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        for cap in INJECTABLE_CLASS_RE.captures_iter(source) {
            let class_name = &cap[2];
            let injectable_fqn = Self::class_fqn(file, class_name);

            // Find nodes that inject this class (look for @Autowired fields of this type
            // in other nodes). Since we only have the current file's source, we create
            // edges from existing nodes that reference this type.
            for node in existing_nodes {
                if matches!(node.kind, NodeKind::Class) && node.fqn != injectable_fqn {
                    // Check if this node's class has an @Autowired field of the injectable type
                    let node_class = node.fqn.rsplit("::").next().unwrap_or("");
                    if Self::class_injects_type(source, node_class, class_name) {
                        edges.push(Self::make_injects_edge(
                            node.fqn.clone(),
                            injectable_fqn.clone(),
                        ));
                    }
                }
            }
        }

        edges
    }

    /// Check if a class in the source code has an @Autowired/@Inject field of the given type.
    fn class_injects_type(source: &str, class_name: &str, type_name: &str) -> bool {
        // Find the class declaration
        let class_pattern = format!(r"class\s+{}\b", regex::escape(class_name));
        let class_re = match Regex::new(&class_pattern) {
            Ok(re) => re,
            Err(_) => return false,
        };

        if let Some(class_match) = class_re.find(source) {
            // Look for @Autowired/@Inject of the target type after the class declaration
            let after_class = &source[class_match.start()..];
            let inject_pattern = format!(
                r"@(?:Autowired|Inject)\s+(?:private|protected|public)?\s*(?:final\s+)?\b{}\b",
                regex::escape(type_name)
            );
            if let Ok(re) = Regex::new(&inject_pattern) {
                return re.is_match(after_class);
            }
        }
        false
    }
}

impl FrameworkAdapter for SpringAdapter {
    fn framework(&self) -> FrameworkKind {
        FrameworkKind::Spring
    }

    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        _tree: &tree_sitter::Tree,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edges = Vec::new();

        // 1. Detect @Autowired/@Inject on fields
        edges.extend(self.detect_autowired_fields(file, source, existing_nodes));

        // 2. Detect @Autowired/@Inject on constructors
        edges.extend(self.detect_autowired_constructors(file, source, existing_nodes));

        // 3. Detect @Bean methods
        edges.extend(self.detect_bean_methods(file, source, existing_nodes));

        // 4. Detect injectable classes and wire consumers
        edges.extend(self.detect_injectable_classes(file, source, existing_nodes));

        // Deduplicate edges (same source_fqn + target_fqn + kind)
        edges.sort_by(|a, b| {
            a.source_fqn
                .cmp(&b.source_fqn)
                .then(a.target_fqn.cmp(&b.target_fqn))
        });
        edges.dedup_by(|a, b| a.source_fqn == b.source_fqn && a.target_fqn == b.target_fqn);

        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper to create a minimal Node for testing.
    fn make_node(fqn: &str, kind: NodeKind, file: &str) -> Node {
        Node {
            fqn: fqn.to_string(),
            kind,
            file: file.to_string(),
            start_line: 1,
            end_line: 10,
            file_hash: "test".to_string(),
            indexed_at: 0,
            attributes: json!({}),
        }
    }

    /// Helper to run the adapter without a tree-sitter tree (we use regex, not AST).
    fn run_adapter(source: &str, file: &str, nodes: &[Node]) -> Vec<Edge> {
        let adapter = SpringAdapter;
        // We need a dummy tree-sitter tree. Since the adapter uses regex only,
        // we create a minimal Java parse.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        adapter.extract_edges(file, source, &tree, nodes)
    }

    #[test]
    fn detects_autowired_field_injection() {
        let source = r#"
package com.example;

public class UserController {
    @Autowired
    private UserService userService;

    @Autowired
    private OrderRepository orderRepo;
}
"#;
        let file = "src/main/java/com/example/UserController.java";
        let nodes = vec![
            make_node(
                "src/main/java/com/example/UserService.java::UserService",
                NodeKind::Class,
                "src/main/java/com/example/UserService.java",
            ),
            make_node(
                "src/main/java/com/example/OrderRepository.java::OrderRepository",
                NodeKind::Class,
                "src/main/java/com/example/OrderRepository.java",
            ),
        ];

        let edges = run_adapter(source, file, &nodes);

        // Should create Injects edges from UserController to UserService and OrderRepository
        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("UserController")
                && e.target_fqn.contains("UserService")
                && e.kind == EdgeKind::Injects
                && e.confidence == 0.8
                && e.edge_source == EdgeSource::FrameworkAdapter
        }));
        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("UserController")
                && e.target_fqn.contains("OrderRepository")
                && e.kind == EdgeKind::Injects
        }));
    }

    #[test]
    fn detects_inject_annotation() {
        let source = r#"
public class PaymentService {
    @Inject
    private PaymentGateway gateway;
}
"#;
        let file = "src/PaymentService.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);

        assert_eq!(edges.len(), 1);
        assert!(edges[0].source_fqn.contains("PaymentService"));
        assert_eq!(edges[0].target_fqn, "PaymentGateway");
        assert_eq!(edges[0].kind, EdgeKind::Injects);
        assert_eq!(edges[0].confidence, FRAMEWORK_CONFIDENCE);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn detects_constructor_injection() {
        let source = r#"
public class OrderService {
    private final UserService userService;
    private final PaymentService paymentService;

    @Autowired
    public OrderService(UserService userService, PaymentService paymentService) {
        this.userService = userService;
        this.paymentService = paymentService;
    }
}
"#;
        let file = "src/OrderService.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);

        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("OrderService") && e.target_fqn == "UserService"
        }));
        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("OrderService") && e.target_fqn == "PaymentService"
        }));
    }

    #[test]
    fn detects_bean_method() {
        let source = r#"
@Configuration
public class AppConfig {
    @Bean
    public DataSource dataSource() {
        return new HikariDataSource();
    }
}

public class UserRepository {
    @Autowired
    private DataSource dataSource;
}
"#;
        let file = "src/AppConfig.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);

        // Should have an edge from UserRepository to the bean factory method
        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("UserRepository")
                && e.target_fqn.contains("AppConfig::dataSource")
                && e.kind == EdgeKind::Injects
        }));
    }

    #[test]
    fn detects_component_annotation() {
        let source = r#"
@Component
public class EmailService {
    public void sendEmail(String to, String body) {}
}

public class NotificationController {
    @Autowired
    private EmailService emailService;
}
"#;
        let file = "src/EmailService.java";
        let nodes = vec![
            make_node(
                "src/EmailService.java::EmailService",
                NodeKind::Class,
                file,
            ),
            make_node(
                "src/EmailService.java::NotificationController",
                NodeKind::Class,
                file,
            ),
        ];

        let edges = run_adapter(source, file, &nodes);

        // Should detect that NotificationController injects EmailService
        assert!(edges.iter().any(|e| {
            e.source_fqn.contains("NotificationController")
                && e.target_fqn.contains("EmailService")
                && e.kind == EdgeKind::Injects
        }));
    }

    #[test]
    fn detects_service_annotation() {
        let source = r#"
@Service
public class AuthService {
    public boolean authenticate(String token) { return true; }
}
"#;
        let file = "src/AuthService.java";
        let nodes = vec![];

        let _edges = run_adapter(source, file, &nodes);
        // With no consumers in existing_nodes, no edges from injectable detection
        // but the class is recognized as injectable
        // (edges only created when consumers exist)
        assert!(INJECTABLE_CLASS_RE.is_match(source));
    }

    #[test]
    fn detects_repository_annotation() {
        let source = r#"
@Repository
public class UserRepository {
    public User findById(Long id) { return null; }
}
"#;
        let file = "src/UserRepository.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);
        assert!(INJECTABLE_CLASS_RE.is_match(source));
        // No consumers → no edges
        assert!(edges.is_empty());
    }

    #[test]
    fn all_edges_have_correct_metadata() {
        let source = r#"
public class MyController {
    @Autowired
    private MyService myService;

    @Inject
    private MyRepo myRepo;
}
"#;
        let file = "src/MyController.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);

        for edge in &edges {
            assert_eq!(edge.kind, EdgeKind::Injects);
            assert_eq!(edge.confidence, FRAMEWORK_CONFIDENCE);
            assert_eq!(edge.edge_source, EdgeSource::FrameworkAdapter);
            assert_eq!(edge.id, None);
        }
    }

    #[test]
    fn no_edges_for_non_spring_code() {
        let source = r#"
public class PlainClass {
    private String name;

    public PlainClass(String name) {
        this.name = name;
    }
}
"#;
        let file = "src/PlainClass.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);
        assert!(edges.is_empty());
    }

    #[test]
    fn deduplicates_edges() {
        // If the same injection is detected by multiple patterns, it should be deduplicated
        let source = r#"
@Component
public class ServiceA {
}

public class Consumer {
    @Autowired
    private ServiceA serviceA;
}
"#;
        let file = "src/test.java";
        let nodes = vec![
            make_node("src/test.java::ServiceA", NodeKind::Class, file),
            make_node("src/test.java::Consumer", NodeKind::Class, file),
        ];

        let edges = run_adapter(source, file, &nodes);

        // Count edges from Consumer to ServiceA — should be exactly 1 after dedup
        let consumer_to_service: Vec<_> = edges
            .iter()
            .filter(|e| e.source_fqn.contains("Consumer") && e.target_fqn.contains("ServiceA"))
            .collect();
        assert_eq!(consumer_to_service.len(), 1);
    }

    #[test]
    fn handles_generic_types() {
        let source = r#"
public class GenericConsumer {
    @Autowired
    private List<String> items;

    @Autowired
    private Repository<User> userRepo;
}
"#;
        let file = "src/GenericConsumer.java";
        let nodes = vec![];

        let edges = run_adapter(source, file, &nodes);

        // Should strip generics: List<String> → List, Repository<User> → Repository
        assert!(edges.iter().any(|e| e.target_fqn == "List"));
        assert!(edges.iter().any(|e| e.target_fqn == "Repository"));
    }

    #[test]
    fn controller_annotation_marks_injectable() {
        let source = r#"
@Controller
public class WebController {
    public String index() { return "index"; }
}
"#;
        let _file = "src/WebController.java";
        assert!(INJECTABLE_CLASS_RE.is_match(source));
    }
}
