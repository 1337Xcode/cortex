//! Agent configuration module: writes MCP server config for detected agents.
//!
//! Each agent has a specific configuration format and root key. Configuration
//! writing is idempotent: running twice produces identical config without duplication.
//!
//! Hardened flow:
//! 1. Create config directory with `fs::create_dir_all` if it doesn't exist
//! 2. Generate MCP configuration JSON for the agent
//! 3. Validate generated config by parsing back with `serde_json::from_str`
//! 4. On permission error: report specific file path and required permissions
//! 5. Report success only after validation passes

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::AgentError;

use super::detect::DetectedAgent;

/// Configure a detected agent to use Cortex as an MCP server.
///
/// Hardened flow:
/// 1. Creates config directory if it doesn't exist
/// 2. Generates MCP configuration JSON
/// 3. Validates the generated config by parsing it back
/// 4. Writes to disk only after validation passes
/// 5. Reports permission errors with specific file paths
///
/// The operation is idempotent: calling it multiple times produces the same result.
pub fn configure_agent(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    // Determine config file path based on agent
    let config_file = resolve_config_file(agent);

    // Step 1: Create config directory with fs::create_dir_all
    if let Some(parent) = config_file.parent() {
        create_config_dir(parent)?;
    }

    // Step 2: Generate MCP configuration JSON
    let config_content = generate_config(agent, cortex_binary, &config_file)?;

    // Step 3: Validate generated config by parsing back
    validate_config(agent, &config_content)?;

    // Step 4: Write to disk (with permission error handling)
    write_config_file(&config_file, &config_content)?;

    Ok(())
}

/// Resolve the config file path for a given agent.
fn resolve_config_file(agent: &DetectedAgent) -> std::path::PathBuf {
    match agent.name.as_str() {
        "claude_code" => agent.config_path.join("settings.json"),
        "zed" => agent.config_path.join("settings.json"),
        "gemini" => agent.config_path.join("settings.json"),
        "kiro" => agent.config_path.join("settings").join("mcp.json"),
        "aider" => agent.config_path.with_file_name(".aider.mcp.json"),
        "continue_dev" => agent.config_path.join("config.json"),
        "copilot" => agent.config_path.join("copilot-mcp.json"),
        _ => agent.config_path.join("mcp.json"),
    }
}

/// Create the config directory, reporting permission errors with specific paths.
fn create_config_dir(dir: &Path) -> Result<(), AgentError> {
    fs::create_dir_all(dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            AgentError::PermissionDenied {
                path: dir.to_string_lossy().to_string(),
            }
        } else {
            AgentError::ConfigurationFailed {
                reason: format!("failed to create directory {}: {}", dir.display(), e),
            }
        }
    })
}

/// Generate the MCP configuration JSON content for an agent.
/// Loads existing config if present, merges the cortex entry, and returns
/// the serialized JSON string.
fn generate_config(
    agent: &DetectedAgent,
    cortex_binary: &Path,
    config_file: &Path,
) -> Result<String, AgentError> {
    let mut config = load_json(config_file)?;
    ensure_object(&mut config);

    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }

    let servers = obj
        .get_mut(&agent.root_key)
        .unwrap()
        .as_object_mut()
        .unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));

    serde_json::to_string_pretty(&config).map_err(|e| AgentError::ConfigurationFailed {
        reason: format!("failed to serialize config: {}", e),
    })
}

/// Validate generated config by parsing it back with serde_json::from_str.
/// Returns Ok(()) if the config is valid JSON, or an error describing the
/// validation failure.
fn validate_config(agent: &DetectedAgent, content: &str) -> Result<(), AgentError> {
    serde_json::from_str::<Value>(content).map_err(|e| AgentError::ValidationFailed {
        agent: agent.display_name.clone(),
        reason: e.to_string(),
    })?;
    Ok(())
}

/// Write the config content to disk, handling permission errors specifically.
fn write_config_file(path: &Path, content: &str) -> Result<(), AgentError> {
    fs::write(path, content).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            AgentError::PermissionDenied {
                path: path.to_string_lossy().to_string(),
            }
        } else {
            AgentError::ConfigurationFailed {
                reason: format!("failed to write {}: {}", path.display(), e),
            }
        }
    })
}

/// Build the standard MCP server entry for Cortex.
fn build_cortex_entry(cortex_binary: &Path) -> Value {
    serde_json::json!({
        "command": cortex_binary.to_string_lossy(),
        "args": ["serve"]
    })
}

/// Load a JSON file, returning an empty object if it doesn't exist.
fn load_json(path: &Path) -> Result<Value, AgentError> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.trim().is_empty() {
                Ok(serde_json::json!({}))
            } else {
                serde_json::from_str(&content).map_err(|e| AgentError::ConfigurationFailed {
                    reason: format!("failed to parse {}: {}", path.display(), e),
                })
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(AgentError::PermissionDenied {
                path: path.to_string_lossy().to_string(),
            })
        }
        Err(e) => Err(AgentError::ConfigurationFailed {
            reason: format!("failed to read {}: {}", path.display(), e),
        }),
    }
}

/// Ensure a Value is an object (replace with empty object if not).
fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use proptest::prelude::*;

    #[test]
    fn test_configure_agent_creates_dir_and_validates() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".cursor");
        // Do NOT create the directory; configure_agent should create it

        let agent = DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        // Directory should have been created
        assert!(config_path.exists());

        // Config file should exist and be valid JSON
        let mcp_file = config_path.join("mcp.json");
        assert!(mcp_file.exists());

        let content = fs::read_to_string(&mcp_file).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["mcpServers"]["cortex"].is_object());
        assert_eq!(
            parsed["mcpServers"]["cortex"]["command"],
            "/usr/local/bin/cortex"
        );
        assert_eq!(parsed["mcpServers"]["cortex"]["args"][0], "serve");
    }

    #[test]
    fn test_configure_agent_idempotent() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".vscode");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "vscode".to_string(),
            display_name: "VS Code".to_string(),
            root_key: "servers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");

        // Configure twice
        configure_agent(&agent, &binary).unwrap();
        configure_agent(&agent, &binary).unwrap();

        let mcp_file = config_path.join("mcp.json");
        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_file).unwrap()).unwrap();

        // Should have exactly one cortex entry under "servers"
        let servers = content["servers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("cortex"));
    }

    #[test]
    fn test_configure_zed_uses_context_servers() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".zed");

        let agent = DetectedAgent {
            name: "zed".to_string(),
            display_name: "Zed".to_string(),
            root_key: "context_servers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let settings_file = config_path.join("settings.json");
        let content: Value =
            serde_json::from_str(&fs::read_to_string(&settings_file).unwrap()).unwrap();
        assert!(content["context_servers"]["cortex"].is_object());
    }

    #[test]
    fn test_configure_copilot() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".github");

        let agent = DetectedAgent {
            name: "copilot".to_string(),
            display_name: "GitHub Copilot".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("copilot-mcp.json");
        assert!(config_file.exists());

        let content: Value =
            serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
    }

    #[test]
    fn test_configure_kiro_nested_path() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".kiro");

        let agent = DetectedAgent {
            name: "kiro".to_string(),
            display_name: "Kiro".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        // Kiro writes to .kiro/settings/mcp.json
        let config_file = config_path.join("settings").join("mcp.json");
        assert!(config_file.exists());

        let content: Value =
            serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
    }

    #[test]
    fn test_configure_preserves_existing_entries() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".cursor");
        fs::create_dir_all(&config_path).unwrap();

        // Write an existing mcp.json with another server
        let existing = serde_json::json!({
            "mcpServers": {
                "other-server": {
                    "command": "other",
                    "args": []
                }
            }
        });
        fs::write(
            config_path.join("mcp.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        let agent = DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let mcp_file = config_path.join("mcp.json");
        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_file).unwrap()).unwrap();

        // Should have both entries
        let mcp_servers = content["mcpServers"].as_object().unwrap();
        assert_eq!(mcp_servers.len(), 2);
        assert!(mcp_servers.contains_key("cortex"));
        assert!(mcp_servers.contains_key("other-server"));
    }

    #[test]
    fn test_validate_config_rejects_invalid_json() {
        let agent = DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: PathBuf::from("/tmp"),
        };

        let result = validate_config(&agent, "not valid json {{{");
        assert!(result.is_err());
        match result.unwrap_err() {
            AgentError::ValidationFailed { agent, reason } => {
                assert_eq!(agent, "Cursor");
                assert!(!reason.is_empty());
            }
            other => panic!("expected ValidationFailed, got: {:?}", other),
        }
    }

    #[test]
    fn test_validate_config_accepts_valid_json() {
        let agent = DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: PathBuf::from("/tmp"),
        };

        let valid = r#"{"mcpServers":{"cortex":{"command":"cortex","args":["serve"]}}}"#;
        assert!(validate_config(&agent, valid).is_ok());
    }

    #[test]
    fn test_resolve_config_file_paths() {
        let agent = |name: &str, path: &str| DetectedAgent {
            name: name.to_string(),
            display_name: name.to_string(),
            root_key: "mcpServers".to_string(),
            config_path: PathBuf::from(path),
        };

        // Claude Code -> settings.json
        let f = resolve_config_file(&agent("claude_code", "/home/.claude"));
        assert_eq!(f, PathBuf::from("/home/.claude/settings.json"));

        // Zed -> settings.json
        let f = resolve_config_file(&agent("zed", "/repo/.zed"));
        assert_eq!(f, PathBuf::from("/repo/.zed/settings.json"));

        // Kiro -> settings/mcp.json
        let f = resolve_config_file(&agent("kiro", "/repo/.kiro"));
        assert_eq!(f, PathBuf::from("/repo/.kiro/settings/mcp.json"));

        // Copilot -> copilot-mcp.json
        let f = resolve_config_file(&agent("copilot", "/repo/.github"));
        assert_eq!(f, PathBuf::from("/repo/.github/copilot-mcp.json"));

        // Default (cursor, windsurf, etc.) -> mcp.json
        let f = resolve_config_file(&agent("cursor", "/repo/.cursor"));
        assert_eq!(f, PathBuf::from("/repo/.cursor/mcp.json"));
    }

    #[test]
    fn test_all_25_platforms_produce_valid_config() {
        let tmp = TempDir::new().unwrap();
        let binary = PathBuf::from("/usr/local/bin/cortex");

        let platforms = [
            ("claude_code", "Claude Code", "mcpServers"),
            ("cursor", "Cursor", "mcpServers"),
            ("windsurf", "Windsurf", "mcpServers"),
            ("vscode", "VS Code", "servers"),
            ("jetbrains", "JetBrains", "mcpServers"),
            ("zed", "Zed", "context_servers"),
            ("aider", "Aider", "mcpServers"),
            ("continue_dev", "Continue.dev", "mcpServers"),
            ("copilot", "GitHub Copilot", "mcpServers"),
            ("cline_roo", "Cline/Roo", "mcpServers"),
            ("codex_cli", "Codex CLI", "mcpServers"),
            ("antigravity", "Google Antigravity", "mcpServers"),
            ("supermaven", "Supermaven", "mcpServers"),
            ("codeium", "Codeium", "mcpServers"),
            ("tabnine", "Tabnine", "mcpServers"),
            ("opencode", "OpenCode", "mcpServers"),
            ("openclaw", "OpenClaw", "mcpServers"),
            ("droid", "Factory Droid", "mcpServers"),
            ("trae", "Trae", "mcpServers"),
            ("trae-cn", "Trae CN", "mcpServers"),
            ("gemini", "Gemini CLI", "mcpServers"),
            ("hermes", "Hermes", "mcpServers"),
            ("kimi", "Kimi Code", "mcpServers"),
            ("kiro", "Kiro", "mcpServers"),
            ("pi", "Pi", "mcpServers"),
        ];

        for (name, display, root_key) in &platforms {
            let config_path = tmp.path().join(format!(".{}", name));
            let agent = DetectedAgent {
                name: name.to_string(),
                display_name: display.to_string(),
                root_key: root_key.to_string(),
                config_path,
            };

            let result = configure_agent(&agent, &binary);
            assert!(
                result.is_ok(),
                "Failed to configure {}: {:?}",
                name,
                result.err()
            );

            // Verify the written file is valid JSON
            let config_file = resolve_config_file(&agent);
            let content = fs::read_to_string(&config_file).unwrap_or_else(|e| {
                panic!("Failed to read config for {}: {}", name, e);
            });
            let parsed: Result<Value, _> = serde_json::from_str(&content);
            assert!(
                parsed.is_ok(),
                "Config for {} is not valid JSON: {:?}",
                name,
                parsed.err()
            );
        }
    }

    /// The 25 supported platforms as (name, display_name, root_key) tuples.
    const PLATFORMS: [(&str, &str, &str); 25] = [
        ("claude_code", "Claude Code", "mcpServers"),
        ("cursor", "Cursor", "mcpServers"),
        ("windsurf", "Windsurf", "mcpServers"),
        ("vscode", "VS Code", "servers"),
        ("jetbrains", "JetBrains", "mcpServers"),
        ("zed", "Zed", "context_servers"),
        ("aider", "Aider", "mcpServers"),
        ("continue_dev", "Continue.dev", "mcpServers"),
        ("copilot", "GitHub Copilot", "mcpServers"),
        ("cline_roo", "Cline/Roo", "mcpServers"),
        ("codex_cli", "Codex CLI", "mcpServers"),
        ("antigravity", "Google Antigravity", "mcpServers"),
        ("supermaven", "Supermaven", "mcpServers"),
        ("codeium", "Codeium", "mcpServers"),
        ("tabnine", "Tabnine", "mcpServers"),
        ("opencode", "OpenCode", "mcpServers"),
        ("openclaw", "OpenClaw", "mcpServers"),
        ("droid", "Factory Droid", "mcpServers"),
        ("trae", "Trae", "mcpServers"),
        ("trae-cn", "Trae CN", "mcpServers"),
        ("gemini", "Gemini CLI", "mcpServers"),
        ("hermes", "Hermes", "mcpServers"),
        ("kimi", "Kimi Code", "mcpServers"),
        ("kiro", "Kiro", "mcpServers"),
        ("pi", "Pi", "mcpServers"),
    ];

    /// Strategy to select a random platform index from the 25 supported platforms.
    fn platform_index_strategy() -> impl Strategy<Value = usize> {
        0..25usize
    }

    /// Strategy to generate optional pre-existing JSON config content.
    /// This tests that configure_agent correctly merges into existing configs.
    fn existing_config_strategy() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            // No existing config (fresh install)
            Just(None),
            // Empty object
            Just(Some("{}".to_string())),
            // Config with another MCP server already present
            Just(Some(
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "other-tool": {
                            "command": "other-tool",
                            "args": ["run"]
                        }
                    }
                }))
                .unwrap()
            )),
            // Config with unrelated keys
            Just(Some(
                serde_json::to_string_pretty(&serde_json::json!({
                    "someOtherKey": true,
                    "nested": { "value": 42 }
                }))
                .unwrap()
            )),
        ]
    }

    // **Validates: Requirements 14.1, 14.2, 14.4**

    proptest! {
        /// **Property 13: Install produces valid configuration**
        ///
        /// For any of the 25 supported platform names, `configure_agent()` produces
        /// a configuration file that passes JSON syntax validation when parsed back.
        /// Additionally tests with random pre-existing config content to verify
        /// merging behavior produces valid JSON.
        ///
        /// **Validates: Requirements 14.1, 14.2, 14.4**
        #[test]
        fn prop_install_produces_valid_config(
            platform_idx in platform_index_strategy(),
            existing_config in existing_config_strategy(),
        ) {
            let (name, display, root_key) = PLATFORMS[platform_idx];
            let tmp = TempDir::new().unwrap();
            let config_path = tmp.path().join(format!(".{}", name));
            let binary = PathBuf::from("/usr/local/bin/cortex");

            let agent = DetectedAgent {
                name: name.to_string(),
                display_name: display.to_string(),
                root_key: root_key.to_string(),
                config_path: config_path.clone(),
            };

            // If there's pre-existing config, write it to the expected file location
            if let Some(ref content) = existing_config {
                let config_file = resolve_config_file(&agent);
                if let Some(parent) = config_file.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&config_file, content).unwrap();
            }

            // Call configure_agent - should succeed for all platforms
            let result = configure_agent(&agent, &binary);
            prop_assert!(
                result.is_ok(),
                "configure_agent failed for platform '{}': {:?}",
                name,
                result.err()
            );

            // Read the generated config file
            let config_file = resolve_config_file(&agent);
            let content = fs::read_to_string(&config_file);
            prop_assert!(
                content.is_ok(),
                "Failed to read config file for '{}': {:?}",
                name,
                content.err()
            );
            let content = content.unwrap();

            // Verify it passes JSON syntax validation
            let parsed: Result<Value, _> = serde_json::from_str(&content);
            prop_assert!(
                parsed.is_ok(),
                "Config for '{}' is not valid JSON: {:?}\nContent: {}",
                name,
                parsed.err(),
                content
            );

            // Verify the cortex entry exists under the correct root key
            let parsed = parsed.unwrap();
            prop_assert!(
                parsed[root_key]["cortex"].is_object(),
                "Config for '{}' missing cortex entry under '{}'. Got: {}",
                name,
                root_key,
                parsed
            );
        }
    }
}
