//! Framework-specific adapters for extracting edges from parsed source files.
//!
//! Each adapter implements the [`FrameworkAdapter`] trait and pattern-matches
//! framework-specific code (DI wiring, middleware chains, routing, component
//! composition) to create edges with `edge_source=FrameworkAdapter` and
//! `confidence=0.8`.
//!
//! The [`AdapterRegistry`] holds only the adapters for frameworks detected in
//! the repository's dependency manifests, ensuring no false-positive edges from
//! irrelevant framework patterns.

pub mod django;
pub mod express;
pub mod fastapi;
pub mod nestjs;
#[cfg(test)]
mod property_tests;
pub mod react;
pub mod spring;

use crate::indexer::framework_detect::{DetectedFramework, FrameworkKind};
use crate::store::types::{Edge, Node};

/// Trait implemented by all framework adapters.
///
/// Each adapter is responsible for detecting framework-specific patterns
/// in parsed source files and creating edges with appropriate edge kinds.
/// All edges produced by framework adapters carry:
/// - `edge_source = EdgeSource::FrameworkAdapter`
/// - `confidence = 0.8` (ConfidenceTier::Medium)
pub trait FrameworkAdapter: Send + Sync {
    /// The framework this adapter handles.
    fn framework(&self) -> FrameworkKind;

    /// Extract framework-specific edges from a parsed file.
    ///
    /// # Arguments
    /// - `file` — Relative file path within the repository.
    /// - `source` — The full source code text of the file.
    /// - `tree` — The tree-sitter parse tree for the file.
    /// - `existing_nodes` — Nodes already extracted from this file (for FQN resolution).
    ///
    /// # Returns
    /// Edges with `edge_source=FrameworkAdapter` and `confidence=0.8`.
    fn extract_edges(
        &self,
        file: &str,
        source: &str,
        tree: &tree_sitter::Tree,
        existing_nodes: &[Node],
    ) -> Vec<Edge>;
}

/// Registry of active framework adapters.
///
/// Only holds adapters for frameworks that were detected in the repository's
/// dependency manifests (or manually overridden via config). This ensures
/// adapters don't produce false-positive edges for frameworks not in use.
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn FrameworkAdapter>>,
}

impl AdapterRegistry {
    /// Create a new registry, instantiating only adapters for detected frameworks.
    ///
    /// # Arguments
    /// - `detected` — Frameworks detected from dependency manifests or config overrides.
    pub fn new(detected: &[DetectedFramework]) -> Self {
        let adapters = detected
            .iter()
            .filter_map(|fw| Self::create_adapter(&fw.name))
            .collect();

        Self { adapters }
    }

    /// Run all registered adapters on a parsed file and collect edges.
    ///
    /// # Arguments
    /// - `file` — Relative file path within the repository.
    /// - `source` — The full source code text of the file.
    /// - `tree` — The tree-sitter parse tree for the file.
    /// - `existing_nodes` — Nodes already extracted from this file.
    ///
    /// # Returns
    /// All edges produced by all active adapters for this file.
    pub fn run_adapters(
        &self,
        file: &str,
        source: &str,
        tree: &tree_sitter::Tree,
        existing_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut all_edges = Vec::new();
        for adapter in &self.adapters {
            let edges = adapter.extract_edges(file, source, tree, existing_nodes);
            all_edges.extend(edges);
        }
        all_edges
    }

    /// Returns the number of active adapters in the registry.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Returns the framework kinds of all active adapters.
    pub fn active_frameworks(&self) -> Vec<FrameworkKind> {
        self.adapters.iter().map(|a| a.framework()).collect()
    }

    /// Instantiate the appropriate adapter for a given framework kind.
    fn create_adapter(kind: &FrameworkKind) -> Option<Box<dyn FrameworkAdapter>> {
        match kind {
            FrameworkKind::FastAPI => Some(Box::new(fastapi::FastApiAdapter)),
            FrameworkKind::Express => Some(Box::new(express::ExpressAdapter)),
            FrameworkKind::NestJS => Some(Box::new(nestjs::NestJsAdapter)),
            FrameworkKind::Spring => Some(Box::new(spring::SpringAdapter)),
            FrameworkKind::Django => Some(Box::new(django::DjangoAdapter)),
            FrameworkKind::React => Some(Box::new(react::ReactAdapter)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn make_detected(kinds: &[FrameworkKind]) -> Vec<DetectedFramework> {
        kinds
            .iter()
            .map(|k| DetectedFramework {
                name: k.clone(),
                version: None,
                manifest_file: "test".to_string(),
            })
            .collect()
    }

    #[test]
    fn empty_detection_creates_empty_registry() {
        let registry = AdapterRegistry::new(&[]);
        assert_eq!(registry.adapter_count(), 0);
        assert!(registry.active_frameworks().is_empty());
    }

    #[test]
    fn single_framework_creates_one_adapter() {
        let detected = make_detected(&[FrameworkKind::FastAPI]);
        let registry = AdapterRegistry::new(&detected);
        assert_eq!(registry.adapter_count(), 1);
        assert_eq!(registry.active_frameworks(), vec![FrameworkKind::FastAPI]);
    }

    #[test]
    fn multiple_frameworks_creates_multiple_adapters() {
        let detected = make_detected(&[
            FrameworkKind::FastAPI,
            FrameworkKind::React,
            FrameworkKind::Express,
        ]);
        let registry = AdapterRegistry::new(&detected);
        assert_eq!(registry.adapter_count(), 3);

        let active = registry.active_frameworks();
        assert!(active.contains(&FrameworkKind::FastAPI));
        assert!(active.contains(&FrameworkKind::React));
        assert!(active.contains(&FrameworkKind::Express));
    }

    #[test]
    fn all_frameworks_creates_all_adapters() {
        let detected = make_detected(&[
            FrameworkKind::FastAPI,
            FrameworkKind::Express,
            FrameworkKind::NestJS,
            FrameworkKind::Spring,
            FrameworkKind::Django,
            FrameworkKind::React,
        ]);
        let registry = AdapterRegistry::new(&detected);
        assert_eq!(registry.adapter_count(), 6);
    }

    #[test]
    fn adapter_framework_matches_detection() {
        let detected = make_detected(&[FrameworkKind::Spring, FrameworkKind::Django]);
        let registry = AdapterRegistry::new(&detected);

        let active = registry.active_frameworks();
        assert_eq!(active.len(), 2);
        assert!(active.contains(&FrameworkKind::Spring));
        assert!(active.contains(&FrameworkKind::Django));
    }

    // ─── Property-Based Tests ────────────────────────────────────────────────

    /// All possible framework kinds for generating arbitrary subsets.
    const ALL_FRAMEWORKS: &[FrameworkKind] = &[
        FrameworkKind::FastAPI,
        FrameworkKind::Express,
        FrameworkKind::NestJS,
        FrameworkKind::Spring,
        FrameworkKind::Django,
        FrameworkKind::React,
    ];

    /// Strategy to generate an arbitrary subset of frameworks as a boolean mask.
    /// Each framework is independently included or excluded.
    fn arb_framework_subset() -> impl Strategy<Value = Vec<FrameworkKind>> {
        // Generate a 6-element boolean vector, one per framework
        proptest::collection::vec(proptest::bool::ANY, 6..=6).prop_map(|mask| {
            ALL_FRAMEWORKS
                .iter()
                .zip(mask.iter())
                .filter_map(|(fw, &include)| if include { Some(fw.clone()) } else { None })
                .collect()
        })
    }

    /// Write the appropriate manifest files for a given set of frameworks into a temp dir.
    fn write_manifests_for_frameworks(dir: &std::path::Path, frameworks: &[FrameworkKind]) {
        // Collect which manifests we need
        let mut npm_deps: Vec<(&str, &str)> = Vec::new();
        let mut pip_lines: Vec<String> = Vec::new();
        let mut need_pom = false;

        for fw in frameworks {
            match fw {
                FrameworkKind::Express => npm_deps.push(("express", "^4.18.0")),
                FrameworkKind::NestJS => npm_deps.push(("@nestjs/core", "^10.0.0")),
                FrameworkKind::React => npm_deps.push(("react", "^18.2.0")),
                FrameworkKind::FastAPI => pip_lines.push("fastapi==0.104.0".to_string()),
                FrameworkKind::Django => pip_lines.push("django>=4.2".to_string()),
                FrameworkKind::Spring => need_pom = true,
            }
        }

        // Write package.json if any npm deps
        if !npm_deps.is_empty() {
            let deps: Vec<String> = npm_deps
                .iter()
                .map(|(name, ver)| format!("    \"{}\": \"{}\"", name, ver))
                .collect();
            let content = format!(
                "{{\n  \"dependencies\": {{\n{}\n  }}\n}}",
                deps.join(",\n")
            );
            std::fs::write(dir.join("package.json"), content).unwrap();
        }

        // Write requirements.txt if any pip deps
        if !pip_lines.is_empty() {
            std::fs::write(dir.join("requirements.txt"), pip_lines.join("\n")).unwrap();
        }

        // Write pom.xml if Spring
        if need_pom {
            std::fs::write(
                dir.join("pom.xml"),
                "<project><parent><groupId>org.springframework.boot</groupId></parent></project>",
            )
            .unwrap();
        }
    }

    // **Feature: cortex-intelligence-overhaul**
    // **Property 19: Adapter activation matches framework detection**
    // **Validates: Requirements 24.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// For any repository with a set of dependency manifests, the framework
        /// adapters that produce edges SHALL be exactly the set of adapters whose
        /// corresponding framework was detected in the manifests (no false-positive
        /// edges from undetected frameworks).
        ///
        /// **Feature: cortex-intelligence-overhaul**
        /// **Property: Adapter activation matches framework detection**
        /// **Validates: Requirements 24.2**
        #[test]
        fn property_activation_matches_detection(
            chosen_frameworks in arb_framework_subset()
        ) {
            use crate::indexer::framework_detect::detect_frameworks;

            // Create a temp directory with manifests for the chosen frameworks
            let tmp = TempDir::new().unwrap();
            write_manifests_for_frameworks(tmp.path(), &chosen_frameworks);

            // Run framework detection
            let detected = detect_frameworks(tmp.path());
            let detected_kinds: HashSet<FrameworkKind> =
                detected.iter().map(|d| d.name.clone()).collect();

            // The detected set should match exactly what we put in manifests
            let expected_kinds: HashSet<FrameworkKind> =
                chosen_frameworks.iter().cloned().collect();
            prop_assert_eq!(
                &detected_kinds,
                &expected_kinds,
                "Detection mismatch: expected {:?}, got {:?}",
                expected_kinds,
                detected_kinds
            );

            // Create adapter registry from detected frameworks
            let registry = AdapterRegistry::new(&detected);
            let active_kinds: HashSet<FrameworkKind> =
                registry.active_frameworks().into_iter().collect();

            // The active adapters should be exactly the detected frameworks
            prop_assert_eq!(
                &active_kinds,
                &detected_kinds,
                "Adapter activation mismatch: detected {:?}, but active adapters are {:?}",
                detected_kinds,
                active_kinds
            );

            // No false-positive adapters: active set must be subset of chosen
            for active in &active_kinds {
                prop_assert!(
                    expected_kinds.contains(active),
                    "False-positive adapter: {:?} is active but was not in manifests",
                    active
                );
            }

            // No missing adapters: every chosen framework should have an active adapter
            for chosen in &expected_kinds {
                prop_assert!(
                    active_kinds.contains(chosen),
                    "Missing adapter: {:?} was in manifests but has no active adapter",
                    chosen
                );
            }
        }
    }
}
