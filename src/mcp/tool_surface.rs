//! Tool Surface Manager
//!
//! Controls which MCP tools are exposed to AI coding agents based on configuration.
//! Tools are classified as Default (always-on), Experimental (opt-in via config),
//! or SmartOnly (minimal set for --smart-tools mode).

use std::path::Path;

/// Tool visibility classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVisibility {
    /// Always exposed to agents.
    Default,
    /// Opt-in via `.cortex/config.toml` setting `experimental_tools = true`.
    Experimental,
    /// Only exposed in `--smart-tools` mode (minimal surface).
    SmartOnly,
}

/// Configuration controlling which tools are active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSurfaceConfig {
    /// Whether experimental tools are enabled (from `.cortex/config.toml`).
    pub experimental_tools: bool,
    /// Whether smart-tools mode is active (from `--smart-tools` CLI flag).
    pub smart_tools: bool,
}

/// Default tools (always-on): core intelligence tools that every agent needs.
/// Note: `semantic_search` is conditionally included based on embeddings availability.
pub const DEFAULT_TOOLS: &[&str] = &[
    "get_repo_brief",
    "get_task_context",
    "ask",
    "trace_callers",
    "blast_radius",
    "get_complexity_hotspots",
    "get_git_hotspots",
    "search_symbols",
    "write_observation",
    "read_observations",
    "semantic_search",
];

/// Experimental tools (opt-in): advanced analysis tools enabled via config.
pub const EXPERIMENTAL_TOOLS: &[&str] = &[
    "find_taint_paths",
    "check_dependencies",
    "decompose_boundaries",
    "generate_steering",
    "find_dead_code",
    "generate_sbom",
    "find_similar_functions",
];

/// Smart-tools mode (minimal): only the 5 most essential tools for token-constrained agents.
pub const SMART_TOOLS: &[&str] = &[
    "get_repo_brief",
    "ask",
    "get_task_context",
    "write_observation",
    "read_observations",
];

/// Returns the list of active tool names based on the provided configuration.
///
/// - If `smart_tools` is true, returns only the minimal SMART_TOOLS set.
/// - If `experimental_tools` is true, returns DEFAULT_TOOLS + EXPERIMENTAL_TOOLS.
/// - Otherwise, returns DEFAULT_TOOLS only.
pub fn get_active_tools(config: &ToolSurfaceConfig) -> Vec<&'static str> {
    if config.smart_tools {
        SMART_TOOLS.to_vec()
    } else if config.experimental_tools {
        let mut tools = DEFAULT_TOOLS.to_vec();
        tools.extend_from_slice(EXPERIMENTAL_TOOLS);
        tools
    } else {
        DEFAULT_TOOLS.to_vec()
    }
}

/// Reads the `experimental_tools` setting from `.cortex/config.toml`.
///
/// Returns `true` if the config file exists and contains `experimental_tools = true`.
/// Returns `false` if the file doesn't exist, can't be parsed, or the key is absent.
pub fn read_experimental_tools_config(repo_root: &Path) -> bool {
    let config_path = repo_root.join(".cortex").join("config.toml");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    parsed
        .get("experimental_tools")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Builds a `ToolSurfaceConfig` from the repository config and CLI flags.
///
/// - `repo_root`: Path to the repository root (for reading `.cortex/config.toml`).
/// - `smart_tools_flag`: Whether `--smart-tools` was passed on the CLI.
pub fn build_tool_surface_config(repo_root: &Path, smart_tools_flag: bool) -> ToolSurfaceConfig {
    ToolSurfaceConfig {
        experimental_tools: read_experimental_tools_config(repo_root),
        smart_tools: smart_tools_flag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_tools_returns_11_tools() {
        let config = ToolSurfaceConfig {
            experimental_tools: false,
            smart_tools: false,
        };
        let tools = get_active_tools(&config);
        assert_eq!(tools.len(), 11);
        assert!(tools.contains(&"get_repo_brief"));
        assert!(tools.contains(&"get_task_context"));
        assert!(tools.contains(&"ask"));
        assert!(tools.contains(&"trace_callers"));
        assert!(tools.contains(&"blast_radius"));
        assert!(tools.contains(&"get_complexity_hotspots"));
        assert!(tools.contains(&"get_git_hotspots"));
        assert!(tools.contains(&"search_symbols"));
        assert!(tools.contains(&"write_observation"));
        assert!(tools.contains(&"read_observations"));
        assert!(tools.contains(&"semantic_search"));
    }

    #[test]
    fn test_experimental_tools_includes_default_plus_experimental() {
        let config = ToolSurfaceConfig {
            experimental_tools: true,
            smart_tools: false,
        };
        let tools = get_active_tools(&config);
        assert_eq!(tools.len(), 18); // 11 default (incl. semantic_search) + 7 experimental
        // Check default tools are present
        assert!(tools.contains(&"get_repo_brief"));
        assert!(tools.contains(&"ask"));
        assert!(tools.contains(&"semantic_search"));
        // Check experimental tools are present
        assert!(tools.contains(&"find_taint_paths"));
        assert!(tools.contains(&"check_dependencies"));
        assert!(tools.contains(&"decompose_boundaries"));
        assert!(tools.contains(&"generate_steering"));
        assert!(tools.contains(&"find_dead_code"));
        assert!(tools.contains(&"generate_sbom"));
        assert!(tools.contains(&"find_similar_functions"));
    }

    #[test]
    fn test_smart_tools_returns_minimal_set() {
        let config = ToolSurfaceConfig {
            experimental_tools: false,
            smart_tools: true,
        };
        let tools = get_active_tools(&config);
        assert_eq!(tools.len(), 5);
        assert!(tools.contains(&"get_repo_brief"));
        assert!(tools.contains(&"ask"));
        assert!(tools.contains(&"get_task_context"));
        assert!(tools.contains(&"write_observation"));
        assert!(tools.contains(&"read_observations"));
    }

    #[test]
    fn test_smart_tools_overrides_experimental() {
        // When both flags are set, smart_tools takes precedence
        let config = ToolSurfaceConfig {
            experimental_tools: true,
            smart_tools: true,
        };
        let tools = get_active_tools(&config);
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_default_tools_excludes_experimental() {
        let config = ToolSurfaceConfig {
            experimental_tools: false,
            smart_tools: false,
        };
        let tools = get_active_tools(&config);
        assert!(!tools.contains(&"find_taint_paths"));
        assert!(!tools.contains(&"check_dependencies"));
        assert!(!tools.contains(&"generate_sbom"));
    }

    #[test]
    fn test_read_experimental_tools_config_missing_file() {
        let tmp = TempDir::new().unwrap();
        assert!(!read_experimental_tools_config(tmp.path()));
    }

    #[test]
    fn test_read_experimental_tools_config_false() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("config.toml"), "experimental_tools = false").unwrap();
        assert!(!read_experimental_tools_config(tmp.path()));
    }

    #[test]
    fn test_read_experimental_tools_config_true() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("config.toml"), "experimental_tools = true").unwrap();
        assert!(read_experimental_tools_config(tmp.path()));
    }

    #[test]
    fn test_read_experimental_tools_config_missing_key() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("config.toml"), "some_other_key = true").unwrap();
        assert!(!read_experimental_tools_config(tmp.path()));
    }

    #[test]
    fn test_read_experimental_tools_config_invalid_toml() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("config.toml"), "not valid toml {{{{").unwrap();
        assert!(!read_experimental_tools_config(tmp.path()));
    }

    #[test]
    fn test_build_tool_surface_config_with_smart_flag() {
        let tmp = TempDir::new().unwrap();
        let config = build_tool_surface_config(tmp.path(), true);
        assert!(config.smart_tools);
        assert!(!config.experimental_tools);
    }

    #[test]
    fn test_build_tool_surface_config_reads_experimental_from_file() {
        let tmp = TempDir::new().unwrap();
        let cortex_dir = tmp.path().join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();
        fs::write(cortex_dir.join("config.toml"), "experimental_tools = true").unwrap();
        let config = build_tool_surface_config(tmp.path(), false);
        assert!(config.experimental_tools);
        assert!(!config.smart_tools);
    }

    #[test]
    fn test_smart_tools_is_subset_of_default() {
        // Every tool in SMART_TOOLS should also be in DEFAULT_TOOLS
        for tool in SMART_TOOLS {
            assert!(
                DEFAULT_TOOLS.contains(tool),
                "Smart tool '{}' is not in DEFAULT_TOOLS",
                tool
            );
        }
    }

    #[test]
    fn test_no_overlap_between_default_and_experimental() {
        // DEFAULT_TOOLS and EXPERIMENTAL_TOOLS should be disjoint
        for tool in EXPERIMENTAL_TOOLS {
            assert!(
                !DEFAULT_TOOLS.contains(tool),
                "Experimental tool '{}' should not be in DEFAULT_TOOLS",
                tool
            );
        }
    }

    // ─── Property-Based Tests for cortex-intelligence-overhaul ────────────────

    /// **Feature: cortex-intelligence-overhaul**
    ///
    /// **Property 18: Tool descriptions under 100 tokens**
    ///
    /// Every tool description in the manifest has word_count < 100.
    ///
    /// **Validates: Requirements 23.4**
    ///
    /// Note: This is a deterministic property test that validates all tool
    /// descriptions in the manifest. Since the tool list is fixed at compile
    /// time, we verify the invariant holds for every tool definition.
    #[test]
    fn prop_tool_descriptions_under_100_tokens() {
        use crate::mcp::server::{get_tool_definitions, get_smart_tool_definitions};

        let all_tools = get_tool_definitions();
        let smart_tools = get_smart_tool_definitions();

        for tool in all_tools.iter().chain(smart_tools.iter()) {
            let word_count = tool.description.split_whitespace().count();
            assert!(
                word_count < 100,
                "Tool '{}' description has {} words (>= 100): '{}'",
                tool.name, word_count, tool.description
            );
        }
    }
}
