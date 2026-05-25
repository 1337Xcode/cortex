//! Build system detection and workspace member extraction.
//!
//! Detects Cargo workspaces, npm/yarn workspaces, Go workspaces,
//! and Gradle/Maven multi-module projects. Extracts module boundaries
//! and inter-module dependencies.

use std::path::Path;

use serde::Serialize;

/// Detected build system type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum BuildSystem {
    Cargo,
    Npm,
    Go,
    Gradle,
    Maven,
    Unknown,
}

/// A workspace member/module detected from the build system.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceMember {
    /// Name of the module/package.
    pub name: String,
    /// Relative path to the module root.
    pub path: String,
    /// Dependencies on other workspace members (by name).
    pub internal_deps: Vec<String>,
}

/// Result of build system detection.
#[derive(Debug, Clone, Serialize)]
pub struct BuildSystemInfo {
    /// The detected build system type.
    pub build_system: BuildSystem,
    /// Workspace members/modules found.
    pub members: Vec<WorkspaceMember>,
}

/// Detect the build system and extract workspace members from the given repo root.
pub fn detect(repo_root: &Path) -> BuildSystemInfo {
    // Try each build system in order of specificity
    if let Some(info) = detect_cargo(repo_root) {
        return info;
    }
    if let Some(info) = detect_npm(repo_root) {
        return info;
    }
    if let Some(info) = detect_go(repo_root) {
        return info;
    }
    if let Some(info) = detect_gradle(repo_root) {
        return info;
    }
    if let Some(info) = detect_maven(repo_root) {
        return info;
    }

    BuildSystemInfo {
        build_system: BuildSystem::Unknown,
        members: Vec::new(),
    }
}

/// Detect Cargo workspace by looking for Cargo.toml with [workspace] section.
fn detect_cargo(repo_root: &Path) -> Option<BuildSystemInfo> {
    let cargo_toml_path = repo_root.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&cargo_toml_path).ok()?;

    // Check for [workspace] section
    if !content.contains("[workspace]") {
        // Single crate, not a workspace. Still report it as a single member.
        let name = extract_cargo_package_name(&content).unwrap_or_else(|| "root".to_string());
        return Some(BuildSystemInfo {
            build_system: BuildSystem::Cargo,
            members: vec![WorkspaceMember {
                name,
                path: ".".to_string(),
                internal_deps: Vec::new(),
            }],
        });
    }

    // Parse workspace members from the TOML
    let mut members = Vec::new();

    // Look for members = ["crate1", "crate2"] pattern
    if let Some(members_start) = content.find("members")
        && let Some(bracket_start) = content[members_start..].find('[')
    {
        let after_bracket = members_start + bracket_start + 1;
        if let Some(bracket_end) = content[after_bracket..].find(']') {
            let members_str = &content[after_bracket..after_bracket + bracket_end];
            for member in members_str.split(',') {
                let member = member.trim().trim_matches('"').trim_matches('\'').trim();
                if member.is_empty() || member.contains('*') {
                    // Handle glob patterns by scanning directories
                    if member.contains('*') {
                        let pattern_base = member.trim_end_matches("/*").trim_end_matches("\\*");
                        let base_dir = repo_root.join(pattern_base);
                        if base_dir.exists()
                            && let Ok(entries) = std::fs::read_dir(&base_dir)
                        {
                            for entry in entries.flatten() {
                                if entry.path().join("Cargo.toml").exists() {
                                    let name = entry.file_name().to_string_lossy().to_string();
                                    let path = format!("{}/{}", pattern_base, name);
                                    members.push(WorkspaceMember {
                                        name,
                                        path,
                                        internal_deps: Vec::new(),
                                    });
                                }
                            }
                        }
                    }
                    continue;
                }
                let member_path = repo_root.join(member);
                let name = if member_path.join("Cargo.toml").exists() {
                    let member_toml =
                        std::fs::read_to_string(member_path.join("Cargo.toml")).unwrap_or_default();
                    extract_cargo_package_name(&member_toml)
                        .unwrap_or_else(|| member.rsplit('/').next().unwrap_or(member).to_string())
                } else {
                    member.rsplit('/').next().unwrap_or(member).to_string()
                };
                members.push(WorkspaceMember {
                    name,
                    path: member.to_string(),
                    internal_deps: Vec::new(),
                });
            }
        }
    }

    Some(BuildSystemInfo {
        build_system: BuildSystem::Cargo,
        members,
    })
}

/// Extract the package name from a Cargo.toml content string.
fn extract_cargo_package_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") && trimmed.contains('=') {
            let value = trimmed
                .split('=')
                .nth(1)?
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Detect npm/yarn workspace by looking for package.json with workspaces field.
fn detect_npm(repo_root: &Path) -> Option<BuildSystemInfo> {
    let package_json_path = repo_root.join("package.json");
    if !package_json_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&package_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check for "workspaces" field
    let workspaces = parsed.get("workspaces")?;

    let workspace_patterns: Vec<&str> = if let Some(arr) = workspaces.as_array() {
        arr.iter().filter_map(|v| v.as_str()).collect()
    } else if let Some(obj) = workspaces.as_object() {
        // yarn workspaces format: { "packages": ["packages/*"] }
        obj.get("packages")
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default()
    } else {
        return None;
    };

    let mut members = Vec::new();

    for pattern in workspace_patterns {
        if pattern.contains('*') {
            // Glob pattern: scan directories
            let base = pattern.trim_end_matches("/*").trim_end_matches("\\*");
            let base_dir = repo_root.join(base);
            if base_dir.exists()
                && let Ok(entries) = std::fs::read_dir(&base_dir)
            {
                for entry in entries.flatten() {
                    if entry.path().join("package.json").exists() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let path = format!("{}/{}", base, name);
                        members.push(WorkspaceMember {
                            name,
                            path,
                            internal_deps: Vec::new(),
                        });
                    }
                }
            }
        } else {
            // Direct path
            let member_path = repo_root.join(pattern);
            if member_path.join("package.json").exists() {
                let name = pattern.rsplit('/').next().unwrap_or(pattern).to_string();
                members.push(WorkspaceMember {
                    name,
                    path: pattern.to_string(),
                    internal_deps: Vec::new(),
                });
            }
        }
    }

    Some(BuildSystemInfo {
        build_system: BuildSystem::Npm,
        members,
    })
}

/// Detect Go workspace by looking for go.work file.
fn detect_go(repo_root: &Path) -> Option<BuildSystemInfo> {
    let go_work_path = repo_root.join("go.work");
    if !go_work_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&go_work_path).ok()?;
    let mut members = Vec::new();
    let mut in_use_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "use (" {
            in_use_block = true;
            continue;
        }
        if trimmed == ")" {
            in_use_block = false;
            continue;
        }

        if in_use_block && !trimmed.is_empty() && !trimmed.starts_with("//") {
            let module_path = trimmed.trim_start_matches("./");
            let name = module_path
                .rsplit('/')
                .next()
                .unwrap_or(module_path)
                .to_string();
            members.push(WorkspaceMember {
                name,
                path: module_path.to_string(),
                internal_deps: Vec::new(),
            });
        } else if trimmed.starts_with("use ") && !trimmed.contains('(') {
            // Single-line use directive
            let module_path = trimmed
                .trim_start_matches("use ")
                .trim()
                .trim_start_matches("./");
            let name = module_path
                .rsplit('/')
                .next()
                .unwrap_or(module_path)
                .to_string();
            members.push(WorkspaceMember {
                name,
                path: module_path.to_string(),
                internal_deps: Vec::new(),
            });
        }
    }

    Some(BuildSystemInfo {
        build_system: BuildSystem::Go,
        members,
    })
}

/// Detect Gradle multi-module project by looking for settings.gradle or settings.gradle.kts.
fn detect_gradle(repo_root: &Path) -> Option<BuildSystemInfo> {
    let settings_path = repo_root.join("settings.gradle");
    let settings_kts_path = repo_root.join("settings.gradle.kts");

    let content = if settings_path.exists() {
        std::fs::read_to_string(&settings_path).ok()?
    } else if settings_kts_path.exists() {
        std::fs::read_to_string(&settings_kts_path).ok()?
    } else {
        return None;
    };

    let mut members = Vec::new();

    // Look for include/include() patterns
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("include") {
            // Extract module names from include(':module1', ':module2') or include ":module1"
            let after_include = trimmed
                .trim_start_matches("include")
                .trim_start_matches('(')
                .trim_end_matches(')');
            for part in after_include.split(',') {
                let module = part
                    .trim()
                    .trim_matches('\'')
                    .trim_matches('"')
                    .trim_start_matches(':')
                    .trim();
                if !module.is_empty() {
                    let path = module.replace(':', "/");
                    members.push(WorkspaceMember {
                        name: module.to_string(),
                        path,
                        internal_deps: Vec::new(),
                    });
                }
            }
        }
    }

    if members.is_empty() {
        return None;
    }

    Some(BuildSystemInfo {
        build_system: BuildSystem::Gradle,
        members,
    })
}

/// Detect Maven multi-module project by looking for pom.xml with <modules> section.
fn detect_maven(repo_root: &Path) -> Option<BuildSystemInfo> {
    let pom_path = repo_root.join("pom.xml");
    if !pom_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&pom_path).ok()?;

    // Check for <modules> section
    if !content.contains("<modules>") {
        return None;
    }

    let mut members = Vec::new();

    // Simple XML parsing: extract <module>name</module> entries
    let modules_start = content.find("<modules>")?;
    let modules_end = content[modules_start..].find("</modules>")?;
    let modules_section = &content[modules_start..modules_start + modules_end];

    for line in modules_section.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<module>") && trimmed.ends_with("</module>") {
            let module = trimmed
                .trim_start_matches("<module>")
                .trim_end_matches("</module>")
                .trim();
            if !module.is_empty() {
                members.push(WorkspaceMember {
                    name: module.to_string(),
                    path: module.to_string(),
                    internal_deps: Vec::new(),
                });
            }
        }
    }

    if members.is_empty() {
        return None;
    }

    Some(BuildSystemInfo {
        build_system: BuildSystem::Maven,
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_no_build_system() {
        let tmp = TempDir::new().unwrap();
        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Unknown);
        assert!(info.members.is_empty());
    }

    #[test]
    fn test_detect_cargo_single_crate() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Cargo);
        assert_eq!(info.members.len(), 1);
        assert_eq!(info.members[0].name, "my-crate");
    }

    #[test]
    fn test_detect_cargo_workspace() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crate-a\", \"crate-b\"]\n",
        )
        .unwrap();

        // Create member directories with Cargo.toml
        std::fs::create_dir_all(tmp.path().join("crate-a")).unwrap();
        std::fs::write(
            tmp.path().join("crate-a/Cargo.toml"),
            "[package]\nname = \"crate-a\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("crate-b")).unwrap();
        std::fs::write(
            tmp.path().join("crate-b/Cargo.toml"),
            "[package]\nname = \"crate-b\"\n",
        )
        .unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Cargo);
        assert_eq!(info.members.len(), 2);
    }

    #[test]
    fn test_detect_npm_workspace() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"name": "monorepo", "workspaces": ["packages/core", "packages/cli"]}"#,
        )
        .unwrap();

        // Create member directories
        std::fs::create_dir_all(tmp.path().join("packages/core")).unwrap();
        std::fs::write(tmp.path().join("packages/core/package.json"), "{}").unwrap();
        std::fs::create_dir_all(tmp.path().join("packages/cli")).unwrap();
        std::fs::write(tmp.path().join("packages/cli/package.json"), "{}").unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Npm);
        assert_eq!(info.members.len(), 2);
    }

    #[test]
    fn test_detect_go_workspace() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("go.work"),
            "go 1.21\n\nuse (\n\t./cmd/server\n\t./pkg/auth\n)\n",
        )
        .unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Go);
        assert_eq!(info.members.len(), 2);
        assert_eq!(info.members[0].name, "server");
        assert_eq!(info.members[0].path, "cmd/server");
    }

    #[test]
    fn test_detect_gradle() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("settings.gradle"),
            "include ':app', ':core', ':data'\n",
        )
        .unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Gradle);
        assert_eq!(info.members.len(), 3);
    }

    #[test]
    fn test_detect_maven() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pom.xml"),
            "<project>\n  <modules>\n    <module>core</module>\n    <module>web</module>\n  </modules>\n</project>\n",
        ).unwrap();

        let info = detect(tmp.path());
        assert_eq!(info.build_system, BuildSystem::Maven);
        assert_eq!(info.members.len(), 2);
        assert_eq!(info.members[0].name, "core");
        assert_eq!(info.members[1].name, "web");
    }

    #[test]
    fn test_extract_cargo_package_name() {
        let content = "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            extract_cargo_package_name(content),
            Some("my-crate".to_string())
        );
    }

    #[test]
    fn test_extract_cargo_package_name_missing() {
        let content = "[workspace]\nmembers = [\"a\"]\n";
        assert_eq!(extract_cargo_package_name(content), None);
    }
}
