//! User-configurable pattern rules engine.
//!
//! Loads custom pattern rules from `.cortex/patterns.toml` and applies them
//! to source code during indexing. Each rule specifies a regex pattern with
//! named capture groups for source and target nodes, plus the edge kind and
//! confidence tier to assign to matched edges.

use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::store::confidence::{ConfidenceTier, EdgeSource};
use crate::store::types::{Edge, EdgeKind, Node};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when loading or validating pattern rules.
#[derive(Debug, Error)]
pub enum PatternError {
    /// The `.cortex/patterns.toml` file was not found.
    #[error("pattern rules file not found: {path}")]
    FileNotFound { path: String },

    /// The TOML file could not be parsed.
    #[error("failed to parse patterns.toml: {reason}")]
    TomlParseError { reason: String },

    /// A regex pattern in a rule is invalid.
    #[error("invalid regex in rule '{rule_name}': {reason}")]
    InvalidRegex { rule_name: String, reason: String },

    /// A named capture group referenced by a rule does not exist in the regex.
    #[error("missing capture group '{group}' in rule '{rule_name}' pattern")]
    MissingCaptureGroup { rule_name: String, group: String },
}

// ---------------------------------------------------------------------------
// Pattern rule types
// ---------------------------------------------------------------------------

/// A user-defined pattern rule from `.cortex/patterns.toml`.
///
/// Each rule specifies a regex pattern to match against source code, with
/// named capture groups identifying the source and target nodes for edge
/// creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternRule {
    /// Human-readable name for this rule.
    pub name: String,
    /// Regex pattern to match against source code lines.
    pub pattern: String,
    /// Named capture group in the regex that identifies the source node.
    pub source_capture: String,
    /// Named capture group in the regex that identifies the target node.
    pub target_capture: String,
    /// The kind of edge to create when this rule matches.
    pub edge_kind: EdgeKind,
    /// The confidence tier to assign to edges created by this rule.
    pub confidence_tier: ConfidenceTier,
}

/// Top-level TOML structure for `.cortex/patterns.toml`.
#[derive(Debug, Deserialize)]
struct PatternsFile {
    /// List of pattern rules defined in the file.
    #[serde(default)]
    rules: Vec<PatternRule>,
}

// ---------------------------------------------------------------------------
// Loading and validation
// ---------------------------------------------------------------------------

/// Load and validate pattern rules from `.cortex/patterns.toml`.
///
/// Returns an empty `Vec` if the file does not exist (non-error case for
/// repositories without custom patterns). Returns `PatternError` if the file
/// exists but contains invalid TOML or invalid regex patterns.
pub fn load_pattern_rules(repo_root: &Path) -> Result<Vec<PatternRule>, PatternError> {
    let patterns_path = repo_root.join(".cortex").join("patterns.toml");

    if !patterns_path.exists() {
        // No patterns file is a valid state — just no custom rules.
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&patterns_path).map_err(|_| PatternError::FileNotFound {
        path: patterns_path.display().to_string(),
    })?;

    let patterns_file: PatternsFile =
        toml::from_str(&content).map_err(|e| PatternError::TomlParseError {
            reason: e.to_string(),
        })?;

    // Validate each rule's regex and capture groups.
    for rule in &patterns_file.rules {
        validate_rule(rule)?;
    }

    Ok(patterns_file.rules)
}

/// Validate a single pattern rule: compile the regex and check that the
/// named capture groups exist.
fn validate_rule(rule: &PatternRule) -> Result<(), PatternError> {
    let regex = Regex::new(&rule.pattern).map_err(|e| PatternError::InvalidRegex {
        rule_name: rule.name.clone(),
        reason: e.to_string(),
    })?;

    // Check that source_capture is a valid named group in the regex.
    let capture_names: Vec<&str> = regex.capture_names().flatten().collect();

    if !capture_names.contains(&rule.source_capture.as_str()) {
        return Err(PatternError::MissingCaptureGroup {
            rule_name: rule.name.clone(),
            group: rule.source_capture.clone(),
        });
    }

    if !capture_names.contains(&rule.target_capture.as_str()) {
        return Err(PatternError::MissingCaptureGroup {
            rule_name: rule.name.clone(),
            group: rule.target_capture.clone(),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Pattern rule application
// ---------------------------------------------------------------------------

/// Apply pattern rules to source code, returning matched edges.
///
/// For each rule, the regex is applied to the source code. When a match is
/// found, the source and target FQNs are extracted from the named capture
/// groups, and an edge is created with the configured kind and confidence.
pub fn apply_pattern_rules(
    rules: &[PatternRule],
    file: &str,
    source: &str,
    _existing_nodes: &[Node],
) -> Vec<Edge> {
    let mut edges = Vec::new();

    for rule in rules {
        // Compile regex (validated at load time, so unwrap is safe).
        let regex = match Regex::new(&rule.pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for caps in regex.captures_iter(source) {
            let source_fqn = match caps.name(&rule.source_capture) {
                Some(m) => format!("{}::{}", file, m.as_str()),
                None => continue,
            };

            let target_fqn = match caps.name(&rule.target_capture) {
                Some(m) => m.as_str().to_string(),
                None => continue,
            };

            edges.push(Edge {
                id: None,
                source_fqn,
                target_fqn,
                kind: rule.edge_kind.clone(),
                confidence: rule.confidence_tier.numeric(),
                edge_source: EdgeSource::FrameworkAdapter,
                attributes: serde_json::json!({"pattern_rule": rule.name}),
            });
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
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a temp repo with a patterns.toml file.
    fn create_patterns_file(dir: &TempDir, content: &str) {
        let cortex_dir = dir.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("patterns.toml"), content).unwrap();
    }

    #[test]
    fn load_returns_empty_when_no_file() {
        let dir = TempDir::new().unwrap();
        let rules = load_pattern_rules(dir.path()).unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn load_parses_valid_patterns_toml() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(
            &dir,
            r#"
[[rules]]
name = "celery_task"
pattern = '(?P<source>\w+)\.delay\((?P<target>\w+)\)'
source_capture = "source"
target_capture = "target"
edge_kind = "Calls"
confidence_tier = "Medium"
"#,
        );

        let rules = load_pattern_rules(dir.path()).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "celery_task");
        assert_eq!(rules[0].edge_kind, EdgeKind::Calls);
        assert_eq!(rules[0].confidence_tier, ConfidenceTier::Medium);
    }

    #[test]
    fn load_returns_error_on_invalid_toml() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(&dir, "this is not valid toml [[[");

        let result = load_pattern_rules(dir.path());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PatternError::TomlParseError { .. }));
    }

    #[test]
    fn load_returns_error_on_invalid_regex() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(
            &dir,
            r#"
[[rules]]
name = "bad_regex"
pattern = '(?P<source>\w+)\.delay\((?P<target>[unclosed'
source_capture = "source"
target_capture = "target"
edge_kind = "Calls"
confidence_tier = "Medium"
"#,
        );

        let result = load_pattern_rules(dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            PatternError::InvalidRegex { rule_name, .. } => {
                assert_eq!(rule_name, "bad_regex");
            }
            other => panic!("expected InvalidRegex, got {:?}", other),
        }
    }

    #[test]
    fn load_returns_error_on_missing_source_capture() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(
            &dir,
            r#"
[[rules]]
name = "missing_group"
pattern = '(?P<target>\w+)\.call\(\)'
source_capture = "source"
target_capture = "target"
edge_kind = "Calls"
confidence_tier = "Low"
"#,
        );

        let result = load_pattern_rules(dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            PatternError::MissingCaptureGroup { rule_name, group } => {
                assert_eq!(rule_name, "missing_group");
                assert_eq!(group, "source");
            }
            other => panic!("expected MissingCaptureGroup, got {:?}", other),
        }
    }

    #[test]
    fn load_returns_error_on_missing_target_capture() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(
            &dir,
            r#"
[[rules]]
name = "missing_target"
pattern = '(?P<source>\w+)\.call\(\)'
source_capture = "source"
target_capture = "target"
edge_kind = "Calls"
confidence_tier = "Low"
"#,
        );

        let result = load_pattern_rules(dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            PatternError::MissingCaptureGroup { rule_name, group } => {
                assert_eq!(rule_name, "missing_target");
                assert_eq!(group, "target");
            }
            other => panic!("expected MissingCaptureGroup, got {:?}", other),
        }
    }

    #[test]
    fn apply_pattern_rules_creates_edges() {
        let rules = vec![PatternRule {
            name: "celery_task".to_string(),
            pattern: r"(?P<source>\w+)\.delay\((?P<target>\w+)\)".to_string(),
            source_capture: "source".to_string(),
            target_capture: "target".to_string(),
            edge_kind: EdgeKind::Calls,
            confidence_tier: ConfidenceTier::Medium,
        }];

        let source_code = "worker.delay(process_data)";
        let edges = apply_pattern_rules(&rules, "src/tasks.py", source_code, &[]);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_fqn, "src/tasks.py::worker");
        assert_eq!(edges[0].target_fqn, "process_data");
        assert_eq!(edges[0].kind, EdgeKind::Calls);
        assert_eq!(edges[0].confidence, 0.8);
        assert_eq!(edges[0].edge_source, EdgeSource::FrameworkAdapter);
    }

    #[test]
    fn apply_pattern_rules_handles_multiple_matches() {
        let rules = vec![PatternRule {
            name: "inject".to_string(),
            pattern: r"@inject\((?P<source>\w+),\s*(?P<target>\w+)\)".to_string(),
            source_capture: "source".to_string(),
            target_capture: "target".to_string(),
            edge_kind: EdgeKind::Injects,
            confidence_tier: ConfidenceTier::Medium,
        }];

        let source_code = "@inject(ServiceA, RepoA)\n@inject(ServiceB, RepoB)";
        let edges = apply_pattern_rules(&rules, "src/di.py", source_code, &[]);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].target_fqn, "RepoA");
        assert_eq!(edges[1].target_fqn, "RepoB");
    }

    #[test]
    fn apply_pattern_rules_returns_empty_on_no_match() {
        let rules = vec![PatternRule {
            name: "celery_task".to_string(),
            pattern: r"(?P<source>\w+)\.delay\((?P<target>\w+)\)".to_string(),
            source_capture: "source".to_string(),
            target_capture: "target".to_string(),
            edge_kind: EdgeKind::Calls,
            confidence_tier: ConfidenceTier::Medium,
        }];

        let source_code = "def hello(): pass";
        let edges = apply_pattern_rules(&rules, "src/main.py", source_code, &[]);

        assert!(edges.is_empty());
    }

    // ─── Property-Based Tests ────────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy to generate valid identifiers (start with a letter, followed by
    /// alphanumeric/underscore characters). These are guaranteed to match `\w+`.
    fn arb_identifier() -> impl Strategy<Value = String> {
        // Start with a letter, then 1-15 word chars
        "[a-zA-Z][a-zA-Z0-9_]{1,15}"
    }

    /// Strategy to generate an arbitrary EdgeKind.
    fn arb_edge_kind() -> impl Strategy<Value = EdgeKind> {
        prop_oneof![
            Just(EdgeKind::Calls),
            Just(EdgeKind::Imports),
            Just(EdgeKind::Inherits),
            Just(EdgeKind::Implements),
            Just(EdgeKind::HttpLink),
            Just(EdgeKind::DataFlow),
            Just(EdgeKind::Injects),
            Just(EdgeKind::Middleware),
            Just(EdgeKind::Routes),
            Just(EdgeKind::Renders),
        ]
    }

    /// Strategy to generate an arbitrary ConfidenceTier.
    fn arb_confidence_tier() -> impl Strategy<Value = ConfidenceTier> {
        prop_oneof![
            Just(ConfidenceTier::High),
            Just(ConfidenceTier::Medium),
            Just(ConfidenceTier::Low),
            Just(ConfidenceTier::VeryLow),
        ]
    }

    // **Feature: cortex-intelligence-overhaul**
    // **Property: Custom pattern rule application**
    // **Validates: Requirements 9.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // For any valid pattern rule with a regex containing named capture groups,
        // and any source string that matches that regex, applying the rule SHALL
        // create an edge with the configured edge_kind, the configured
        // confidence_tier, and edge_source=framework_adapter, where source_fqn
        // and target_fqn are extracted from the named captures.
        //
        // **Feature: cortex-intelligence-overhaul**
        // **Property: Custom pattern rule application**
        // **Validates: Requirements 9.3**
        #[test]
        fn property_pattern_application(
            source_name in arb_identifier(),
            target_name in arb_identifier(),
            edge_kind in arb_edge_kind(),
            confidence_tier in arb_confidence_tier(),
            file_path in "[a-z]{1,8}/[a-z]{1,8}\\.[a-z]{1,4}",
        ) {
            // Build a pattern rule with a regex that uses named capture groups
            let rule = PatternRule {
                name: "test_rule".to_string(),
                pattern: r"fn (?P<source>\w+) calls (?P<target>\w+)".to_string(),
                source_capture: "source".to_string(),
                target_capture: "target".to_string(),
                edge_kind: edge_kind.clone(),
                confidence_tier,
            };

            // Build source code that matches the pattern
            let source_code = format!("fn {} calls {}", source_name, target_name);

            // Apply the rule
            let edges = apply_pattern_rules(&[rule], &file_path, &source_code, &[]);

            // Must produce exactly one edge
            prop_assert_eq!(edges.len(), 1, "Expected 1 edge, got {}", edges.len());

            let edge = &edges[0];

            // edge_kind must match the configured kind
            prop_assert_eq!(
                &edge.kind, &edge_kind,
                "Edge kind mismatch: expected {:?}, got {:?}",
                edge_kind, edge.kind
            );

            // confidence must match the tier's numeric value
            let expected_confidence = confidence_tier.numeric();
            prop_assert!(
                (edge.confidence - expected_confidence).abs() < f64::EPSILON,
                "Confidence mismatch: expected {}, got {}",
                expected_confidence, edge.confidence
            );

            // edge_source must be FrameworkAdapter
            prop_assert_eq!(
                &edge.edge_source, &EdgeSource::FrameworkAdapter,
                "Edge source must be FrameworkAdapter, got {:?}",
                &edge.edge_source
            );

            // source_fqn must contain the source capture (file::source_name)
            let expected_source_fqn = format!("{}::{}", file_path, source_name);
            prop_assert_eq!(
                &edge.source_fqn, &expected_source_fqn,
                "source_fqn mismatch: expected '{}', got '{}'",
                expected_source_fqn, edge.source_fqn
            );

            // target_fqn must be the target capture
            prop_assert_eq!(
                &edge.target_fqn, &target_name,
                "target_fqn mismatch: expected '{}', got '{}'",
                target_name, edge.target_fqn
            );
        }
    }

    #[test]
    fn load_multiple_rules() {
        let dir = TempDir::new().unwrap();
        create_patterns_file(
            &dir,
            r#"
[[rules]]
name = "celery_task"
pattern = '(?P<source>\w+)\.delay\((?P<target>\w+)\)'
source_capture = "source"
target_capture = "target"
edge_kind = "Calls"
confidence_tier = "Medium"

[[rules]]
name = "event_handler"
pattern = '(?P<source>\w+)\.on\("(?P<target>\w+)"\)'
source_capture = "source"
target_capture = "target"
edge_kind = "Routes"
confidence_tier = "Low"
"#,
        );

        let rules = load_pattern_rules(dir.path()).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "celery_task");
        assert_eq!(rules[1].name, "event_handler");
        assert_eq!(rules[1].edge_kind, EdgeKind::Routes);
        assert_eq!(rules[1].confidence_tier, ConfidenceTier::Low);
    }
}

// ---------------------------------------------------------------------------
// Property-based tests
// ---------------------------------------------------------------------------

/// **Feature: cortex-intelligence-overhaul**
/// **Property: Pattern rule serialization round-trip**
/// **Validates: Requirements 9.2**
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Strategy to generate a valid EdgeKind variant.
    fn arb_edge_kind() -> impl Strategy<Value = EdgeKind> {
        prop_oneof![
            Just(EdgeKind::Calls),
            Just(EdgeKind::Imports),
            Just(EdgeKind::Inherits),
            Just(EdgeKind::Implements),
            Just(EdgeKind::HttpLink),
            Just(EdgeKind::DataFlow),
            Just(EdgeKind::Injects),
            Just(EdgeKind::Middleware),
            Just(EdgeKind::Routes),
            Just(EdgeKind::Renders),
        ]
    }

    /// Strategy to generate a valid ConfidenceTier variant.
    fn arb_confidence_tier() -> impl Strategy<Value = ConfidenceTier> {
        prop_oneof![
            Just(ConfidenceTier::High),
            Just(ConfidenceTier::Medium),
            Just(ConfidenceTier::Low),
            Just(ConfidenceTier::VeryLow),
        ]
    }

    /// Strategy to generate a valid regex pattern containing named capture
    /// groups `source` and `target`. We use a set of known-valid patterns
    /// to avoid generating invalid regex strings.
    fn arb_regex_pattern() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(r"(?P<source>\w+)\.delay\((?P<target>\w+)\)".to_string()),
            Just(r"(?P<source>\w+)\.call\((?P<target>\w+)\)".to_string()),
            Just(r"@inject\((?P<source>\w+),\s*(?P<target>\w+)\)".to_string()),
            Just(r"(?P<source>[a-zA-Z_]\w*)\.on\((?P<target>[a-zA-Z_]\w*)\)".to_string()),
            Just(r"fn (?P<source>\w+).*(?P<target>\w+)".to_string()),
            Just(r"(?P<source>\w+)\s*->\s*(?P<target>\w+)".to_string()),
            Just(r"import\s+(?P<target>\w+)\s+from\s+(?P<source>\w+)".to_string()),
            Just(r"(?P<source>\w+)::(?P<target>\w+)".to_string()),
        ]
    }

    /// Strategy to generate a valid identifier-like name (alphanumeric + underscore).
    fn arb_identifier() -> impl Strategy<Value = String> {
        "[a-zA-Z_][a-zA-Z0-9_]{0,30}".prop_map(|s| s)
    }

    /// Strategy to generate a valid PatternRule with a valid regex containing
    /// the required named capture groups.
    fn arb_pattern_rule() -> impl Strategy<Value = PatternRule> {
        (
            arb_identifier(),
            arb_regex_pattern(),
            arb_edge_kind(),
            arb_confidence_tier(),
        )
            .prop_map(|(name, pattern, edge_kind, confidence_tier)| PatternRule {
                name,
                pattern,
                source_capture: "source".to_string(),
                target_capture: "target".to_string(),
                edge_kind,
                confidence_tier,
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// **Feature: cortex-intelligence-overhaul**
        /// **Property: Pattern rule serialization round-trip**
        ///
        /// For any valid PatternRule struct (with valid regex, edge_kind, and
        /// confidence_tier), serializing to TOML and deserializing back SHALL
        /// produce an equivalent PatternRule.
        ///
        /// **Validates: Requirements 9.2**
        #[test]
        fn property_pattern_roundtrip(rule in arb_pattern_rule()) {
            // Wrap in a struct that mirrors the TOML file format
            #[derive(Debug, Serialize, Deserialize)]
            struct Wrapper {
                rules: Vec<PatternRule>,
            }

            let wrapper = Wrapper { rules: vec![rule.clone()] };

            // Serialize to TOML
            let toml_str = toml::to_string(&wrapper)
                .expect("serialization to TOML should succeed for valid PatternRule");

            // Deserialize back from TOML
            let deserialized: Wrapper = toml::from_str(&toml_str)
                .expect("deserialization from TOML should succeed for valid TOML");

            // Assert round-trip equivalence
            prop_assert_eq!(deserialized.rules.len(), 1);
            let recovered = &deserialized.rules[0];
            prop_assert_eq!(&recovered.name, &rule.name);
            prop_assert_eq!(&recovered.pattern, &rule.pattern);
            prop_assert_eq!(&recovered.source_capture, &rule.source_capture);
            prop_assert_eq!(&recovered.target_capture, &rule.target_capture);
            prop_assert_eq!(&recovered.edge_kind, &rule.edge_kind);
            prop_assert_eq!(&recovered.confidence_tier, &rule.confidence_tier);

            // Also verify full struct equality
            prop_assert_eq!(recovered, &rule);
        }
    }
}
