//! Framework detection for dependency manifests.
//!
//! Scans repository dependency manifests (package.json, requirements.txt,
//! pyproject.toml, pom.xml, build.gradle, go.mod, Gemfile, composer.json)
//! to determine which framework adapters should be activated during indexing.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Known framework kinds that Cortex has adapters for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrameworkKind {
    FastAPI,
    Express,
    NestJS,
    Spring,
    Django,
    React,
}

impl FrameworkKind {
    /// Return a human-readable name for display in status output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FastAPI => "fastapi",
            Self::Express => "express",
            Self::NestJS => "nestjs",
            Self::Spring => "spring",
            Self::Django => "django",
            Self::React => "react",
        }
    }

    /// Parse from a lowercase string (e.g. from config.toml overrides).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fastapi" | "fast_api" | "fast-api" => Some(Self::FastAPI),
            "express" => Some(Self::Express),
            "nestjs" | "nest" => Some(Self::NestJS),
            "spring" | "spring-boot" | "spring_boot" => Some(Self::Spring),
            "django" => Some(Self::Django),
            "react" => Some(Self::React),
            _ => None,
        }
    }
}

impl std::fmt::Display for FrameworkKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A detected framework with its source manifest and optional version.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedFramework {
    /// Which framework was detected.
    pub name: FrameworkKind,
    /// Version string if extractable from the manifest.
    pub version: Option<String>,
    /// The manifest file that triggered detection (relative path).
    pub manifest_file: String,
}

/// Scan dependency manifests in the repository and return detected frameworks.
///
/// Checks the following manifests:
/// - `package.json` — Express, NestJS, React
/// - `requirements.txt` — FastAPI, Django
/// - `pyproject.toml` — FastAPI, Django
/// - `pom.xml` — Spring
/// - `build.gradle` / `build.gradle.kts` — Spring
/// - `go.mod`, `Gemfile`, `composer.json` — reserved for future use
pub fn detect_frameworks(repo_root: &Path) -> Vec<DetectedFramework> {
    let mut detected = Vec::new();

    detect_from_package_json(repo_root, &mut detected);
    detect_from_requirements_txt(repo_root, &mut detected);
    detect_from_pyproject_toml(repo_root, &mut detected);
    detect_from_pom_xml(repo_root, &mut detected);
    detect_from_build_gradle(repo_root, &mut detected);
    detect_from_go_mod(repo_root, &mut detected);
    detect_from_gemfile(repo_root, &mut detected);
    detect_from_composer_json(repo_root, &mut detected);

    // Deduplicate by framework kind (keep first detection)
    let mut seen = std::collections::HashSet::new();
    detected.retain(|f| seen.insert(f.name.clone()));

    debug!(
        "Detected {} framework(s): {:?}",
        detected.len(),
        detected.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()
    );

    detected
}

/// Load manual framework overrides from `.cortex/config.toml`.
///
/// Looks for a `frameworks` array in the config file:
/// ```toml
/// frameworks = ["fastapi", "express"]
/// ```
///
/// Returns `None` if the config file doesn't exist or has no `frameworks` key.
pub fn load_framework_overrides(repo_root: &Path) -> Option<Vec<FrameworkKind>> {
    let config_path = repo_root.join(".cortex").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;

    let frameworks = parsed.get("frameworks")?.as_array()?;
    let kinds: Vec<FrameworkKind> = frameworks
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(FrameworkKind::from_str)
        .collect();

    if kinds.is_empty() {
        None
    } else {
        Some(kinds)
    }
}

// ─── Private detection helpers ───────────────────────────────────────────────

/// Detect Express, NestJS, and React from package.json dependencies.
fn detect_from_package_json(repo_root: &Path, detected: &mut Vec<DetectedFramework>) {
    let path = repo_root.join("package.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Merge dependencies and devDependencies for scanning
    let deps = collect_npm_deps(&parsed);

    for (name, version) in &deps {
        let lower = name.to_lowercase();
        if lower == "express" {
            detected.push(DetectedFramework {
                name: FrameworkKind::Express,
                version: Some(version.clone()),
                manifest_file: "package.json".to_string(),
            });
        }
        if lower == "@nestjs/core" || lower == "@nestjs/common" {
            detected.push(DetectedFramework {
                name: FrameworkKind::NestJS,
                version: Some(version.clone()),
                manifest_file: "package.json".to_string(),
            });
        }
        if lower == "react" {
            detected.push(DetectedFramework {
                name: FrameworkKind::React,
                version: Some(version.clone()),
                manifest_file: "package.json".to_string(),
            });
        }
    }
}

/// Collect all dependency names and versions from package.json.
fn collect_npm_deps(parsed: &serde_json::Value) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    for section in &["dependencies", "devDependencies"] {
        if let Some(obj) = parsed.get(section).and_then(|v| v.as_object()) {
            for (name, version) in obj {
                let ver = version.as_str().unwrap_or("*").to_string();
                deps.push((name.clone(), ver));
            }
        }
    }
    deps
}

/// Detect FastAPI and Django from requirements.txt.
fn detect_from_requirements_txt(repo_root: &Path, detected: &mut Vec<DetectedFramework>) {
    let path = repo_root.join("requirements.txt");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Parse "package==version" or "package>=version" or just "package"
        let (name, version) = parse_pip_requirement(trimmed);
        let lower = name.to_lowercase();

        if lower == "fastapi" {
            detected.push(DetectedFramework {
                name: FrameworkKind::FastAPI,
                version: version.clone(),
                manifest_file: "requirements.txt".to_string(),
            });
        }
        if lower == "django" {
            detected.push(DetectedFramework {
                name: FrameworkKind::Django,
                version: version.clone(),
                manifest_file: "requirements.txt".to_string(),
            });
        }
    }
}

/// Parse a pip requirement line into (name, optional version).
fn parse_pip_requirement(line: &str) -> (String, Option<String>) {
    // Strip extras like [all] and environment markers
    let line = line.split(';').next().unwrap_or(line).trim();
    // Split on version specifiers
    for sep in &["==", ">=", "<=", "!=", "~=", ">", "<"] {
        if let Some(idx) = line.find(sep) {
            let name = line[..idx].trim().trim_end_matches('[');
            // Also strip extras bracket from name
            let name = name.split('[').next().unwrap_or(name).trim();
            let version = line[idx + sep.len()..].trim().to_string();
            return (name.to_string(), Some(version));
        }
    }
    let name = line.split('[').next().unwrap_or(line).trim();
    (name.to_string(), None)
}

/// Detect FastAPI and Django from pyproject.toml.
fn detect_from_pyproject_toml(repo_root: &Path, detected: &mut Vec<DetectedFramework>) {
    let path = repo_root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Check [project.dependencies] array (PEP 621)
    let deps = parsed
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array());

    if let Some(deps) = deps {
        for dep in deps {
            if let Some(dep_str) = dep.as_str() {
                let (name, version) = parse_pip_requirement(dep_str);
                check_python_framework(&name, version, "pyproject.toml", detected);
            }
        }
    }

    // Also check [tool.poetry.dependencies] for Poetry projects
    let poetry_deps = parsed
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table());

    if let Some(deps) = poetry_deps {
        for (name, value) in deps {
            let version = match value {
                toml::Value::String(v) => Some(v.clone()),
                toml::Value::Table(t) => {
                    t.get("version").and_then(|v| v.as_str()).map(String::from)
                }
                _ => None,
            };
            check_python_framework(name, version, "pyproject.toml", detected);
        }
    }
}

/// Helper to check if a Python dependency name matches a known framework.
fn check_python_framework(
    name: &str,
    version: Option<String>,
    manifest: &str,
    detected: &mut Vec<DetectedFramework>,
) {
    let lower = name.to_lowercase();
    if lower == "fastapi" {
        detected.push(DetectedFramework {
            name: FrameworkKind::FastAPI,
            version: version.clone(),
            manifest_file: manifest.to_string(),
        });
    }
    if lower == "django" {
        detected.push(DetectedFramework {
            name: FrameworkKind::Django,
            version: version.clone(),
            manifest_file: manifest.to_string(),
        });
    }
}

/// Detect Spring from pom.xml (look for spring-boot or spring-framework).
fn detect_from_pom_xml(repo_root: &Path, detected: &mut Vec<DetectedFramework>) {
    let path = repo_root.join("pom.xml");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let lower = content.to_lowercase();
    if lower.contains("spring-boot") || lower.contains("spring-framework")
        || lower.contains("org.springframework")
    {
        // Try to extract version from <version> tag near spring
        let version = extract_spring_version(&content);
        detected.push(DetectedFramework {
            name: FrameworkKind::Spring,
            version,
            manifest_file: "pom.xml".to_string(),
        });
    }
}

/// Try to extract a Spring version from pom.xml content.
fn extract_spring_version(content: &str) -> Option<String> {
    // Look for <spring-boot.version>X.Y.Z</spring-boot.version> or
    // <version>X.Y.Z</version> near spring-boot-starter-parent
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("spring-boot.version")
            || trimmed.contains("spring.version")
        {
            if let Some(start) = trimmed.find('>') {
                if let Some(end) = trimmed[start + 1..].find('<') {
                    let ver = &trimmed[start + 1..start + 1 + end];
                    if !ver.is_empty() && !ver.starts_with('$') {
                        return Some(ver.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Detect Spring from build.gradle or build.gradle.kts.
fn detect_from_build_gradle(repo_root: &Path, detected: &mut Vec<DetectedFramework>) {
    let gradle_path = repo_root.join("build.gradle");
    let gradle_kts_path = repo_root.join("build.gradle.kts");

    let content = if gradle_path.exists() {
        std::fs::read_to_string(&gradle_path).ok()
    } else if gradle_kts_path.exists() {
        std::fs::read_to_string(&gradle_kts_path).ok()
    } else {
        None
    };

    let content = match content {
        Some(c) => c,
        None => return,
    };

    let lower = content.to_lowercase();
    if lower.contains("spring-boot") || lower.contains("org.springframework") {
        detected.push(DetectedFramework {
            name: FrameworkKind::Spring,
            version: None,
            manifest_file: if gradle_path.exists() {
                "build.gradle".to_string()
            } else {
                "build.gradle.kts".to_string()
            },
        });
    }
}

/// Placeholder for go.mod detection (future extensibility).
fn detect_from_go_mod(_repo_root: &Path, _detected: &mut Vec<DetectedFramework>) {
    // Reserved for future Go framework detection (e.g., Gin, Echo, Fiber)
}

/// Placeholder for Gemfile detection (future extensibility).
fn detect_from_gemfile(_repo_root: &Path, _detected: &mut Vec<DetectedFramework>) {
    // Reserved for future Ruby framework detection (e.g., Rails, Sinatra)
}

/// Placeholder for composer.json detection (future extensibility).
fn detect_from_composer_json(_repo_root: &Path, _detected: &mut Vec<DetectedFramework>) {
    // Reserved for future PHP framework detection (e.g., Laravel, Symfony)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_no_manifests() {
        let tmp = TempDir::new().unwrap();
        let result = detect_frameworks(tmp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_express_from_package_json() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"express": "^4.18.0"}}"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::Express);
        assert_eq!(result[0].version.as_deref(), Some("^4.18.0"));
        assert_eq!(result[0].manifest_file, "package.json");
    }

    #[test]
    fn test_detect_react_from_package_json() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"react": "^18.2.0", "react-dom": "^18.2.0"}}"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::React);
    }

    #[test]
    fn test_detect_nestjs_from_package_json() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"@nestjs/core": "^10.0.0", "@nestjs/common": "^10.0.0"}}"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::NestJS);
    }

    #[test]
    fn test_detect_fastapi_from_requirements_txt() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("requirements.txt"),
            "fastapi==0.104.0\nuvicorn>=0.24.0\n",
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::FastAPI);
        assert_eq!(result[0].version.as_deref(), Some("0.104.0"));
        assert_eq!(result[0].manifest_file, "requirements.txt");
    }

    #[test]
    fn test_detect_django_from_requirements_txt() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("requirements.txt"),
            "Django>=4.2\ncelery\n",
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::Django);
        assert_eq!(result[0].version.as_deref(), Some("4.2"));
    }

    #[test]
    fn test_detect_fastapi_from_pyproject_toml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[project]
name = "my-app"
dependencies = ["fastapi>=0.100.0", "pydantic>=2.0"]
"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::FastAPI);
        assert_eq!(result[0].manifest_file, "pyproject.toml");
    }

    #[test]
    fn test_detect_django_from_poetry_pyproject() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[tool.poetry.dependencies]
python = "^3.11"
django = "^4.2"
"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::Django);
    }

    #[test]
    fn test_detect_spring_from_pom_xml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pom.xml"),
            r#"<project>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.0</version>
  </parent>
</project>"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::Spring);
        assert_eq!(result[0].manifest_file, "pom.xml");
    }

    #[test]
    fn test_detect_spring_from_build_gradle() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("build.gradle"),
            r#"
plugins {
    id 'org.springframework.boot' version '3.2.0'
}
dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web'
}
"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::Spring);
        assert_eq!(result[0].manifest_file, "build.gradle");
    }

    #[test]
    fn test_detect_multiple_frameworks() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"express": "^4.18.0", "react": "^18.2.0"}}"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 2);
        let kinds: Vec<&FrameworkKind> = result.iter().map(|f| &f.name).collect();
        assert!(kinds.contains(&&FrameworkKind::Express));
        assert!(kinds.contains(&&FrameworkKind::React));
    }

    #[test]
    fn test_deduplication_across_manifests() {
        let tmp = TempDir::new().unwrap();
        // FastAPI in both requirements.txt and pyproject.toml
        std::fs::write(
            tmp.path().join("requirements.txt"),
            "fastapi==0.104.0\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[project]
dependencies = ["fastapi>=0.100.0"]
"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        // Should deduplicate to a single FastAPI entry
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::FastAPI);
    }

    #[test]
    fn test_detect_from_dev_dependencies() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"devDependencies": {"react": "^18.0.0"}}"#,
        )
        .unwrap();

        let result = detect_frameworks(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, FrameworkKind::React);
    }

    #[test]
    fn test_load_framework_overrides_none() {
        let tmp = TempDir::new().unwrap();
        let result = load_framework_overrides(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_load_framework_overrides_valid() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        std::fs::create_dir_all(&cortex_dir).unwrap();
        std::fs::write(
            cortex_dir.join("config.toml"),
            r#"frameworks = ["fastapi", "express"]"#,
        )
        .unwrap();

        let result = load_framework_overrides(tmp.path());
        assert!(result.is_some());
        let kinds = result.unwrap();
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0], FrameworkKind::FastAPI);
        assert_eq!(kinds[1], FrameworkKind::Express);
    }

    #[test]
    fn test_load_framework_overrides_unknown_ignored() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        std::fs::create_dir_all(&cortex_dir).unwrap();
        std::fs::write(
            cortex_dir.join("config.toml"),
            r#"frameworks = ["fastapi", "unknown_framework", "react"]"#,
        )
        .unwrap();

        let result = load_framework_overrides(tmp.path());
        assert!(result.is_some());
        let kinds = result.unwrap();
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0], FrameworkKind::FastAPI);
        assert_eq!(kinds[1], FrameworkKind::React);
    }

    #[test]
    fn test_load_framework_overrides_empty_array() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        std::fs::create_dir_all(&cortex_dir).unwrap();
        std::fs::write(
            cortex_dir.join("config.toml"),
            r#"frameworks = []"#,
        )
        .unwrap();

        let result = load_framework_overrides(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_framework_kind_from_str() {
        assert_eq!(FrameworkKind::from_str("fastapi"), Some(FrameworkKind::FastAPI));
        assert_eq!(FrameworkKind::from_str("FastAPI"), Some(FrameworkKind::FastAPI));
        assert_eq!(FrameworkKind::from_str("fast-api"), Some(FrameworkKind::FastAPI));
        assert_eq!(FrameworkKind::from_str("express"), Some(FrameworkKind::Express));
        assert_eq!(FrameworkKind::from_str("nestjs"), Some(FrameworkKind::NestJS));
        assert_eq!(FrameworkKind::from_str("nest"), Some(FrameworkKind::NestJS));
        assert_eq!(FrameworkKind::from_str("spring"), Some(FrameworkKind::Spring));
        assert_eq!(FrameworkKind::from_str("spring-boot"), Some(FrameworkKind::Spring));
        assert_eq!(FrameworkKind::from_str("django"), Some(FrameworkKind::Django));
        assert_eq!(FrameworkKind::from_str("react"), Some(FrameworkKind::React));
        assert_eq!(FrameworkKind::from_str("unknown"), None);
    }

    #[test]
    fn test_parse_pip_requirement() {
        assert_eq!(
            parse_pip_requirement("fastapi==0.104.0"),
            ("fastapi".to_string(), Some("0.104.0".to_string()))
        );
        assert_eq!(
            parse_pip_requirement("Django>=4.2"),
            ("Django".to_string(), Some("4.2".to_string()))
        );
        assert_eq!(
            parse_pip_requirement("uvicorn"),
            ("uvicorn".to_string(), None)
        );
        assert_eq!(
            parse_pip_requirement("fastapi[all]>=0.100.0"),
            ("fastapi".to_string(), Some("0.100.0".to_string()))
        );
    }
}
