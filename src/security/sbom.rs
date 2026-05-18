//! SBOM (Software Bill of Materials) generation.
//!
//! Walks Imports edges to identify external packages, cross-references against
//! manifest files (requirements.txt, package.json, go.mod, Cargo.toml),
//! and generates valid SPDX 2.3 JSON output.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::SecurityError;
use crate::store::db::StoreManager;
use crate::store::types::SbomEntry;

// ---------------------------------------------------------------------------
// SPDX 2.3 output types
// ---------------------------------------------------------------------------

/// SPDX 2.3 JSON document structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxDocument {
    pub spdx_version: String,
    pub data_license: String,
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub name: String,
    pub document_namespace: String,
    pub creation_info: SpdxCreationInfo,
    pub packages: Vec<SpdxPackage>,
    pub relationships: Vec<SpdxRelationship>,
}

/// SPDX creation info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxCreationInfo {
    pub created: String,
    pub creators: Vec<String>,
}

/// SPDX package entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxPackage {
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub name: String,
    pub version_info: String,
    pub download_location: String,
    pub files_analyzed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_concluded: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_declared: Option<String>,
    pub copyright_text: String,
}

/// SPDX relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxRelationship {
    pub spdx_element_id: String,
    pub related_spdx_element: String,
    pub relationship_type: String,
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

/// Parsed dependency from a manifest file.
#[derive(Debug, Clone)]
struct ManifestDependency {
    name: String,
    version: Option<String>,
    source_file: String,
}

/// Parse requirements.txt format.
fn parse_requirements_txt(content: &str, source_file: &str) -> Vec<ManifestDependency> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }

        // Handle various version specifiers: ==, >=, <=, ~=, !=
        let (name, version) = if let Some(pos) = line.find("==") {
            (&line[..pos], Some(line[pos + 2..].trim().to_string()))
        } else if let Some(pos) = line.find(">=") {
            (&line[..pos], Some(line[pos + 2..].trim().to_string()))
        } else if let Some(pos) = line.find("<=") {
            (&line[..pos], Some(line[pos + 2..].trim().to_string()))
        } else if let Some(pos) = line.find("~=") {
            (&line[..pos], Some(line[pos + 2..].trim().to_string()))
        } else {
            (line, None)
        };

        // Strip extras like [security]
        let name = if let Some(pos) = name.find('[') {
            &name[..pos]
        } else {
            name
        };

        deps.push(ManifestDependency {
            name: name.trim().to_lowercase(),
            version,
            source_file: source_file.to_string(),
        });
    }

    deps
}

/// Parse package.json dependencies.
fn parse_package_json(content: &str, source_file: &str) -> Vec<ManifestDependency> {
    let mut deps = Vec::new();

    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return deps,
    };

    for section in &["dependencies", "devDependencies"] {
        if let Some(obj) = parsed.get(section).and_then(|v| v.as_object()) {
            for (name, version) in obj {
                let version_str = version.as_str().map(|v| {
                    // Strip version prefixes like ^, ~, >=
                    v.trim_start_matches('^')
                        .trim_start_matches('~')
                        .trim_start_matches(">=")
                        .trim_start_matches("<=")
                        .to_string()
                });

                deps.push(ManifestDependency {
                    name: name.to_lowercase(),
                    version: version_str,
                    source_file: source_file.to_string(),
                });
            }
        }
    }

    deps
}

/// Parse go.mod dependencies.
fn parse_go_mod(content: &str, source_file: &str) -> Vec<ManifestDependency> {
    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let line = line.trim();

        if line == "require (" {
            in_require = true;
            continue;
        }
        if line == ")" {
            in_require = false;
            continue;
        }

        if in_require {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let version = parts[1].trim_start_matches('v').to_string();
                deps.push(ManifestDependency {
                    name: name.to_lowercase(),
                    version: Some(version),
                    source_file: source_file.to_string(),
                });
            }
        } else if line.starts_with("require ") {
            // Single-line require
            let rest = line.trim_start_matches("require ");
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let version = parts[1].trim_start_matches('v').to_string();
                deps.push(ManifestDependency {
                    name: name.to_lowercase(),
                    version: Some(version),
                    source_file: source_file.to_string(),
                });
            }
        }
    }

    deps
}

/// Parse Cargo.toml dependencies.
fn parse_cargo_toml(content: &str, source_file: &str) -> Vec<ManifestDependency> {
    let mut deps = Vec::new();

    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return deps,
    };

    for section in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = parsed.get(section).and_then(|v| v.as_table()) {
            for (name, value) in table {
                let version = match value {
                    toml::Value::String(v) => Some(v.clone()),
                    toml::Value::Table(t) => {
                        t.get("version").and_then(|v| v.as_str()).map(|s| s.to_string())
                    }
                    _ => None,
                };

                deps.push(ManifestDependency {
                    name: name.to_lowercase(),
                    version,
                    source_file: source_file.to_string(),
                });
            }
        }
    }

    deps
}

// ---------------------------------------------------------------------------
// SBOM generation
// ---------------------------------------------------------------------------

/// Generate SBOM entries by cross-referencing import edges with manifest files.
pub fn generate_sbom(
    store: &StoreManager,
    repo_root: &Path,
) -> Result<Vec<SbomEntry>, SecurityError> {
    let conn = store.read_conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Load manifest dependencies
    let manifest_deps = load_manifest_dependencies(repo_root)?;

    // Build a lookup map: lowercase package name -> (version, source_file)
    let dep_map: HashMap<String, (&ManifestDependency,)> = manifest_deps
        .iter()
        .map(|d| (d.name.clone(), (d,)))
        .collect();

    // Query all import edges
    let mut stmt = conn
        .prepare("SELECT source_fqn, target_fqn FROM edges WHERE kind = 'Imports'")
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to prepare imports query: {}", e),
        })?;

    let imports = stmt
        .query_map([], |row| {
            let source: String = row.get(0)?;
            let target: String = row.get(1)?;
            Ok((source, target))
        })
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to query imports: {}", e),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SecurityError::AnalysisFailed {
            reason: format!("failed to collect imports: {}", e),
        })?;

    let mut sbom_entries: HashMap<String, SbomEntry> = HashMap::new();

    for (source_fqn, target_fqn) in &imports {
        // Extract package name from import target
        let package_name = extract_package_name(target_fqn);
        let package_lower = package_name.to_lowercase();

        // Skip standard library imports
        if is_stdlib_import(&package_lower) {
            continue;
        }

        // Look up in manifest
        let (version, source_file) = if let Some((dep,)) = dep_map.get(&package_lower) {
            (dep.version.clone(), dep.source_file.clone())
        } else {
            (None, "unknown".to_string())
        };

        // Deduplicate by package name
        sbom_entries.entry(package_lower.clone()).or_insert_with(|| SbomEntry {
            id: None,
            name: package_name.to_string(),
            version,
            license: None,
            source_file,
            import_fqn: source_fqn.clone(),
            indexed_at: now,
        });
    }

    Ok(sbom_entries.into_values().collect())
}

/// Format SBOM entries as SPDX 2.3 JSON string.
///
/// Convenience wrapper around `generate_spdx` that serializes the result to JSON.
pub fn format_spdx_json(entries: &[SbomEntry], repo_root: &Path) -> Result<String, SecurityError> {
    let project_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let doc = generate_spdx(entries, project_name);
    serde_json::to_string_pretty(&doc).map_err(|e| SecurityError::AnalysisFailed {
        reason: format!("failed to serialize SPDX JSON: {}", e),
    })
}

/// Generate SPDX 2.3 JSON document from SBOM entries.
pub fn generate_spdx(entries: &[SbomEntry], project_name: &str) -> SpdxDocument {
    let now = chrono_now_iso();

    let mut packages = Vec::new();
    let mut relationships = Vec::new();

    // Root package
    packages.push(SpdxPackage {
        spdx_id: "SPDXRef-RootPackage".to_string(),
        name: project_name.to_string(),
        version_info: "0.0.0".to_string(),
        download_location: "NOASSERTION".to_string(),
        files_analyzed: false,
        license_concluded: None,
        license_declared: None,
        copyright_text: "NOASSERTION".to_string(),
    });

    relationships.push(SpdxRelationship {
        spdx_element_id: "SPDXRef-DOCUMENT".to_string(),
        related_spdx_element: "SPDXRef-RootPackage".to_string(),
        relationship_type: "DESCRIBES".to_string(),
    });

    for (i, entry) in entries.iter().enumerate() {
        let spdx_id = format!("SPDXRef-Package-{}", i + 1);

        packages.push(SpdxPackage {
            spdx_id: spdx_id.clone(),
            name: entry.name.clone(),
            version_info: entry.version.clone().unwrap_or_else(|| "NOASSERTION".to_string()),
            download_location: "NOASSERTION".to_string(),
            files_analyzed: false,
            license_concluded: entry.license.clone(),
            license_declared: entry.license.clone(),
            copyright_text: "NOASSERTION".to_string(),
        });

        relationships.push(SpdxRelationship {
            spdx_element_id: "SPDXRef-RootPackage".to_string(),
            related_spdx_element: spdx_id,
            relationship_type: "DEPENDS_ON".to_string(),
        });
    }

    SpdxDocument {
        spdx_version: "SPDX-2.3".to_string(),
        data_license: "CC0-1.0".to_string(),
        spdx_id: "SPDXRef-DOCUMENT".to_string(),
        name: format!("{}-sbom", project_name),
        document_namespace: format!(
            "https://spdx.org/spdxdocs/{}-{}",
            project_name,
            uuid::Uuid::new_v4()
        ),
        creation_info: SpdxCreationInfo {
            created: now,
            creators: vec!["Tool: cortex".to_string()],
        },
        packages,
        relationships,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load all manifest dependencies from known manifest files in the repo root.
fn load_manifest_dependencies(repo_root: &Path) -> Result<Vec<ManifestDependency>, SecurityError> {
    let mut all_deps = Vec::new();

    // requirements.txt
    let req_path = repo_root.join("requirements.txt");
    if req_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&req_path) {
            all_deps.extend(parse_requirements_txt(&content, "requirements.txt"));
        }
    }

    // package.json
    let pkg_path = repo_root.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&pkg_path) {
            all_deps.extend(parse_package_json(&content, "package.json"));
        }
    }

    // go.mod
    let go_path = repo_root.join("go.mod");
    if go_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&go_path) {
            all_deps.extend(parse_go_mod(&content, "go.mod"));
        }
    }

    // Cargo.toml
    let cargo_path = repo_root.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_path) {
            all_deps.extend(parse_cargo_toml(&content, "Cargo.toml"));
        }
    }

    Ok(all_deps)
}

/// Extract the top-level package name from an import FQN.
/// e.g., "flask.request" -> "flask", "github.com/gin-gonic/gin" -> "github.com/gin-gonic/gin"
fn extract_package_name(import_fqn: &str) -> &str {
    // For Python-style imports: take first segment
    if let Some(pos) = import_fqn.find('.') {
        // But for Go-style imports with slashes, keep the full path
        if import_fqn.contains('/') {
            return import_fqn;
        }
        return &import_fqn[..pos];
    }
    import_fqn
}

/// Check if an import is from the standard library.
fn is_stdlib_import(name: &str) -> bool {
    // Python stdlib
    let python_stdlib = [
        "os", "sys", "re", "json", "math", "time", "datetime", "collections",
        "itertools", "functools", "pathlib", "typing", "abc", "io", "string",
        "hashlib", "hmac", "secrets", "random", "copy", "enum", "dataclasses",
    ];

    // Node.js built-ins
    let node_stdlib = [
        "fs", "path", "http", "https", "url", "util", "os", "crypto",
        "stream", "events", "buffer", "child_process", "net", "dns",
    ];

    // Go stdlib (common prefixes)
    let go_stdlib_prefixes = ["fmt", "log", "net", "os", "io", "sync", "context", "strings"];

    python_stdlib.contains(&name)
        || node_stdlib.contains(&name)
        || go_stdlib_prefixes.contains(&name)
}

/// Get current time in ISO 8601 format (simplified).
fn chrono_now_iso() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple ISO format without chrono dependency
    format!("1970-01-01T00:00:00Z") // Placeholder - in production use chrono
        .replace(
            "1970-01-01T00:00:00Z",
            &format_timestamp(secs),
        )
}

/// Format a Unix timestamp as ISO 8601.
fn format_timestamp(secs: u64) -> String {
    // Simple calculation for ISO date
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Approximate year/month/day (good enough for SPDX)
    let mut year = 1970u64;
    let mut remaining_days = days;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let days_in_months: [u64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for &dim in &days_in_months {
        if remaining_days < dim {
            break;
        }
        remaining_days -= dim;
        month += 1;
    }
    let day = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;
    use std::fs;

    fn setup_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create store");
        let conn = store.write_conn();
        migrations::run_migrations(&conn, std::path::Path::new("migrations"))
            .expect("failed to run migrations");
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_parse_requirements_txt() {
        let content = "flask==2.3.0\nrequests>=2.28.0\n# comment\nnumpy\n";
        let deps = parse_requirements_txt(content, "requirements.txt");

        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "flask");
        assert_eq!(deps[0].version, Some("2.3.0".to_string()));
        assert_eq!(deps[1].name, "requests");
        assert_eq!(deps[1].version, Some("2.28.0".to_string()));
        assert_eq!(deps[2].name, "numpy");
        assert_eq!(deps[2].version, None);
    }

    #[test]
    fn test_parse_package_json() {
        let content = r#"{
            "dependencies": {
                "express": "^4.18.0",
                "lodash": "~4.17.21"
            },
            "devDependencies": {
                "jest": "^29.0.0"
            }
        }"#;
        let deps = parse_package_json(content, "package.json");

        assert_eq!(deps.len(), 3);
        let express = deps.iter().find(|d| d.name == "express").unwrap();
        assert_eq!(express.version, Some("4.18.0".to_string()));
    }

    #[test]
    fn test_parse_go_mod() {
        let content = "module example.com/myapp\n\ngo 1.21\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.1\n\tgithub.com/lib/pq v1.10.9\n)\n";
        let deps = parse_go_mod(content, "go.mod");

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "github.com/gin-gonic/gin");
        assert_eq!(deps[0].version, Some("1.9.1".to_string()));
    }

    #[test]
    fn test_parse_cargo_toml() {
        let content = r#"
[package]
name = "myapp"
version = "0.1.0"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = "1.0"
"#;
        let deps = parse_cargo_toml(content, "Cargo.toml");

        assert_eq!(deps.len(), 2);
        let serde_dep = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde_dep.version, Some("1.0".to_string()));
        let tokio_dep = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio_dep.version, Some("1.0".to_string()));
    }

    #[test]
    fn test_version_extracted_from_manifest() {
        let (store, _tmp) = setup_store();

        // Create a temp repo root with a requirements.txt
        let repo_root = tempfile::tempdir().unwrap();
        fs::write(
            repo_root.path().join("requirements.txt"),
            "flask==2.3.0\nrequests==2.28.0\n",
        )
        .unwrap();

        // Insert import edges
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('app.py::main', 'Function', 'app.py', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('app.py::main', 'flask.Flask', 'Imports', 1.0, '{}')",
                [],
            ).unwrap();
        }

        let entries = generate_sbom(&store, repo_root.path()).unwrap();
        assert!(!entries.is_empty());

        let flask_entry = entries.iter().find(|e| e.name == "flask").unwrap();
        assert_eq!(flask_entry.version, Some("2.3.0".to_string()));
        assert_eq!(flask_entry.source_file, "requirements.txt");
    }

    #[test]
    fn test_missing_version_handled() {
        let (store, _tmp) = setup_store();

        // Create a temp repo root without any manifest
        let repo_root = tempfile::tempdir().unwrap();

        // Insert import edge for an unknown package
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('app.py::main', 'Function', 'app.py', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('app.py::main', 'unknown_pkg.module', 'Imports', 1.0, '{}')",
                [],
            ).unwrap();
        }

        let entries = generate_sbom(&store, repo_root.path()).unwrap();
        assert!(!entries.is_empty());

        let entry = &entries[0];
        assert_eq!(entry.version, None);
        assert_eq!(entry.source_file, "unknown");
    }

    #[test]
    fn test_spdx_document_valid() {
        let entries = vec![
            SbomEntry {
                id: None,
                name: "flask".to_string(),
                version: Some("2.3.0".to_string()),
                license: Some("BSD-3-Clause".to_string()),
                source_file: "requirements.txt".to_string(),
                import_fqn: "app.py::flask".to_string(),
                indexed_at: 1000,
            },
            SbomEntry {
                id: None,
                name: "requests".to_string(),
                version: Some("2.28.0".to_string()),
                license: None,
                source_file: "requirements.txt".to_string(),
                import_fqn: "app.py::requests".to_string(),
                indexed_at: 1000,
            },
        ];

        let spdx = generate_spdx(&entries, "test-project");

        assert_eq!(spdx.spdx_version, "SPDX-2.3");
        assert_eq!(spdx.data_license, "CC0-1.0");
        assert_eq!(spdx.spdx_id, "SPDXRef-DOCUMENT");
        // Root package + 2 dependency packages
        assert_eq!(spdx.packages.len(), 3);
        // DESCRIBES + 2 DEPENDS_ON
        assert_eq!(spdx.relationships.len(), 3);

        // Verify root package
        assert_eq!(spdx.packages[0].name, "test-project");

        // Verify dependency packages
        assert_eq!(spdx.packages[1].name, "flask");
        assert_eq!(spdx.packages[1].version_info, "2.3.0");
        assert_eq!(
            spdx.packages[1].license_concluded,
            Some("BSD-3-Clause".to_string())
        );

        // Verify SPDX JSON serialization
        let json = serde_json::to_string_pretty(&spdx).unwrap();
        assert!(json.contains("SPDX-2.3"));
        assert!(json.contains("SPDXRef-DOCUMENT"));
    }

    #[test]
    fn test_format_spdx_json() {
        let entries = vec![SbomEntry {
            id: None,
            name: "flask".to_string(),
            version: Some("2.3.0".to_string()),
            license: None,
            source_file: "requirements.txt".to_string(),
            import_fqn: "app.py::flask".to_string(),
            indexed_at: 1000,
        }];

        let repo_root = std::path::Path::new("/tmp/my-project");
        let json = format_spdx_json(&entries, repo_root).unwrap();

        // Verify it's valid JSON with SPDX structure
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["spdxVersion"], "SPDX-2.3");
        assert_eq!(parsed["dataLicense"], "CC0-1.0");
        assert_eq!(parsed["SPDXID"], "SPDXRef-DOCUMENT");
        assert!(parsed["packages"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_stdlib_imports_excluded() {
        assert!(is_stdlib_import("os"));
        assert!(is_stdlib_import("sys"));
        assert!(is_stdlib_import("fs"));
        assert!(is_stdlib_import("path"));
        assert!(!is_stdlib_import("flask"));
        assert!(!is_stdlib_import("express"));
    }

    #[test]
    fn test_extract_package_name() {
        assert_eq!(extract_package_name("flask.request"), "flask");
        assert_eq!(extract_package_name("numpy"), "numpy");
        assert_eq!(
            extract_package_name("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
    }
}
