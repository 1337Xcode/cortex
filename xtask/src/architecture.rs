//! Dependency direction between the workspace crates.
//!
//! Each product crate sits in a layer and may depend only on crates in lower layers, so the inner
//! crates never learn about the outer ones. The hook client is held tighter: it may depend on
//! `cortex-core` alone, which keeps its start-up small. `cortex-cli` builds the one binary, so it
//! may also depend on the other top-layer crates. Dev-dependencies are not checked: tests may use
//! whatever helps them.

use std::collections::BTreeSet;

use serde::Deserialize;

/// Layer of each product crate; a lower number is further in.
const LAYERS: &[(&str, u8)] = &[
    ("cortex-core", 0),
    ("cortex-store", 1),
    ("cortex-index", 2),
    ("cortex-git", 2),
    ("cortex-learn", 2),
    ("cortex-coord", 2),
    ("cortex-meter", 2),
    ("cortex-retrieve", 3),
    ("cortex-daemon", 4),
    ("cortex-hook", 5),
    ("cortex-mcp", 5),
    ("cortex-cli", 5),
];

/// Workspace packages that are tooling rather than product, exempt from layering.
const TOOLING_CRATES: &[&str] = &["xtask", "cortex-tests"];

/// The crate that links the top layer into the `cortex` binary.
const BINARY_CRATE: &str = "cortex-cli";

/// The hook client, which runs on every agent tool call.
const HOOK_CRATE: &str = "cortex-hook";

/// The only workspace crate the hook client may depend on.
const CORE_CRATE: &str = "cortex-core";

/// The part of `cargo metadata --no-deps` output this check reads.
#[derive(Debug, Deserialize)]
pub(crate) struct Metadata {
    /// Every package in the workspace.
    pub(crate) packages: Vec<Package>,
}

/// One workspace package and what it depends on.
#[derive(Debug, Deserialize)]
pub(crate) struct Package {
    /// The package name, such as `cortex-store`.
    pub(crate) name: String,
    /// Its declared dependencies, workspace and external alike.
    pub(crate) dependencies: Vec<Dependency>,
}

/// One declared dependency.
#[derive(Debug, Deserialize)]
pub(crate) struct Dependency {
    /// The depended-on package's name.
    pub(crate) name: String,
    /// `None` for a normal dependency, otherwise `"dev"` or `"build"`.
    pub(crate) kind: Option<String>,
}

/// Every dependency between workspace crates that points the wrong way, sorted.
pub(crate) fn check(packages: &[Package]) -> Vec<String> {
    let workspace: BTreeSet<&str> = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut violations = Vec::new();
    for package in packages {
        if TOOLING_CRATES.contains(&package.name.as_str()) {
            continue;
        }
        let Some(layer) = layer_of(&package.name) else {
            violations.push(format!(
                "{} has no layer: add it to LAYERS in xtask/src/architecture.rs",
                package.name
            ));
            continue;
        };
        for dependency in &package.dependencies {
            let is_dev = dependency.kind.as_deref() == Some("dev");
            if is_dev || !workspace.contains(dependency.name.as_str()) {
                continue;
            }
            if let Some(violation) = check_edge(&package.name, layer, &dependency.name) {
                violations.push(violation);
            }
        }
    }
    violations.sort();
    violations
}

/// The layer of a product crate, if it has one.
fn layer_of(name: &str) -> Option<u8> {
    LAYERS
        .iter()
        .find(|(crate_name, _)| *crate_name == name)
        .map(|(_, layer)| *layer)
}

/// Why `from` must not depend on `to`, or `None` when the dependency is allowed.
fn check_edge(from: &str, from_layer: u8, to: &str) -> Option<String> {
    if from == HOOK_CRATE && to != CORE_CRATE {
        return Some(format!("{from} may depend only on {CORE_CRATE}, not {to}"));
    }
    let Some(to_layer) = layer_of(to) else {
        return Some(format!("{from} depends on {to}, which has no layer"));
    };
    let is_inward = to_layer < from_layer;
    let is_binary_linking_top = from == BINARY_CRATE && to_layer == from_layer;
    if is_inward || is_binary_linking_top {
        None
    } else {
        Some(format!(
            "{from} (layer {from_layer}) must not depend on {to} (layer {to_layer})"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package named `name` with normal dependencies on `deps`.
    fn package(name: &str, deps: &[&str]) -> Package {
        Package {
            name: name.to_owned(),
            dependencies: deps
                .iter()
                .map(|dep| Dependency {
                    name: (*dep).to_owned(),
                    kind: None,
                })
                .collect(),
        }
    }

    /// Dependencies on lower layers, and the binary's on its own layer, pass.
    #[test]
    fn inward_dependencies_pass() {
        let packages = [
            package("cortex-core", &["serde"]),
            package("cortex-store", &["cortex-core"]),
            package("cortex-retrieve", &["cortex-core", "cortex-store"]),
            package("cortex-hook", &["cortex-core"]),
            package("cortex-mcp", &["cortex-core"]),
            package("cortex-cli", &["cortex-hook", "cortex-mcp", "cortex-store"]),
        ];
        assert_eq!(check(&packages), Vec::<String>::new());
    }

    /// A dependency on a higher layer is reported with both layers.
    #[test]
    fn outward_dependency_fails() {
        let packages = [
            package("cortex-core", &["cortex-store"]),
            package("cortex-store", &[]),
        ];
        assert_eq!(
            check(&packages),
            vec!["cortex-core (layer 0) must not depend on cortex-store (layer 1)".to_owned()]
        );
    }

    /// Crates in the same layer may not depend on each other, except the binary.
    #[test]
    fn same_layer_dependency_fails_outside_the_binary() {
        let packages = [
            package("cortex-index", &["cortex-learn"]),
            package("cortex-learn", &[]),
        ];
        assert_eq!(
            check(&packages),
            vec!["cortex-index (layer 2) must not depend on cortex-learn (layer 2)".to_owned()]
        );
    }

    /// The hook client may depend on the core crate and nothing else of ours.
    #[test]
    fn hook_may_depend_on_core_alone() {
        let packages = [
            package("cortex-hook", &["cortex-core", "cortex-store"]),
            package("cortex-core", &[]),
            package("cortex-store", &[]),
        ];
        assert_eq!(
            check(&packages),
            vec!["cortex-hook may depend only on cortex-core, not cortex-store".to_owned()]
        );
    }

    /// A workspace crate missing from the layer table is reported.
    #[test]
    fn crate_without_a_layer_fails() {
        let packages = [package("cortex-extra", &[])];
        assert_eq!(
            check(&packages),
            vec![
                "cortex-extra has no layer: add it to LAYERS in xtask/src/architecture.rs"
                    .to_owned()
            ]
        );
    }

    /// Dev-dependencies and the tooling crates are outside the layer rules.
    #[test]
    fn dev_dependencies_and_tooling_are_not_checked() {
        let mut core = package("cortex-core", &[]);
        core.dependencies.push(Dependency {
            name: "cortex-store".to_owned(),
            kind: Some("dev".to_owned()),
        });
        let packages = [
            core,
            package("cortex-store", &["cortex-core"]),
            package("cortex-tests", &["cortex-store", "cortex-core"]),
            package("xtask", &[]),
        ];
        assert_eq!(check(&packages), Vec::<String>::new());
    }
}
