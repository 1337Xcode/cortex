//! Agent configuration module: writes MCP server config for detected agents.
//!
//! Each agent has a specific configuration format and root key. Configuration
//! writing is idempotent: running twice produces identical config without duplication.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::AgentError;

use super::detect::DetectedAgent;

/// Configure a detected agent to use Cortex as an MCP server.
///
/// Writes the appropriate configuration file for the agent. The operation
/// is idempotent: calling it multiple times produces the same result.
pub fn configure_agent(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    match agent.name.as_str() {
        "claude_code" => configure_claude_code(agent, cortex_binary),
        "cursor" => configure_cursor(agent, cortex_binary),
        "windsurf" => configure_windsurf(agent, cortex_binary),
        "vscode" => configure_vscode(agent, cortex_binary),
        "jetbrains" => configure_jetbrains(agent, cortex_binary),
        "zed" => configure_zed(agent, cortex_binary),
        "aider" => configure_aider(agent, cortex_binary),
        "continue_dev" => configure_continue_dev(agent, cortex_binary),
        "copilot" => configure_copilot(agent, cortex_binary),
        "cline_roo" => configure_cline_roo(agent, cortex_binary),
        "codex_cli" => configure_codex_cli(agent, cortex_binary),
        "antigravity" => configure_antigravity(agent, cortex_binary),
        "supermaven" => configure_supermaven(agent, cortex_binary),
        "codeium" => configure_codeium(agent, cortex_binary),
        "tabnine" => configure_tabnine(agent, cortex_binary),
        _ => Err(AgentError::ConfigurationFailed {
            reason: format!("unknown agent: {}", agent.name),
        }),
    }
}

/// Build the standard MCP server entry for Cortex.
fn build_cortex_entry(cortex_binary: &Path) -> Value {
    serde_json::json!({
        "command": cortex_binary.to_string_lossy(),
        "args": ["serve"]
    })
}

/// Configure Claude Code: writes to ~/.claude/settings.json under "mcpServers".
fn configure_claude_code(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("settings.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Cursor: writes to .cursor/mcp.json under "mcpServers".
fn configure_cursor(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Windsurf: writes to .windsurf/mcp.json under "mcpServers".
fn configure_windsurf(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure VS Code: writes to .vscode/mcp.json under "servers".
fn configure_vscode(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure JetBrains: writes to .idea/mcp.json under "mcpServers".
fn configure_jetbrains(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Zed: writes to .zed/settings.json under "context_servers".
fn configure_zed(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("settings.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Aider: writes to .aider.conf.yml (as JSON sidecar .aider.mcp.json).
fn configure_aider(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    // Aider uses a JSON sidecar for MCP config
    let config_file = agent.config_path.with_file_name(".aider.mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Continue.dev: writes to .continue/config.json under "mcpServers".
fn configure_continue_dev(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("config.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure GitHub Copilot: writes to .github/copilot-mcp.json under "mcpServers".
fn configure_copilot(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("copilot-mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Cline/Roo: writes to .vscode/mcp.json under "mcpServers".
fn configure_cline_roo(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Codex CLI: writes to .codex/mcp.json under "mcpServers".
fn configure_codex_cli(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Antigravity: writes to .antigravity/mcp.json under "mcpServers".
fn configure_antigravity(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Supermaven: writes to .supermaven/mcp.json under "mcpServers".
fn configure_supermaven(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Codeium: writes to .codeium/mcp.json under "mcpServers".
fn configure_codeium(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
}

/// Configure Tabnine: writes to .tabnine/mcp.json under "mcpServers".
fn configure_tabnine(agent: &DetectedAgent, cortex_binary: &Path) -> Result<(), AgentError> {
    let config_file = agent.config_path.join("mcp.json");
    let mut config = load_json(&config_file)?;
    ensure_object(&mut config);
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key(&agent.root_key) {
        obj.insert(agent.root_key.clone(), serde_json::json!({}));
    }
    let servers = obj.get_mut(&agent.root_key).unwrap().as_object_mut().unwrap();
    servers.insert("cortex".to_string(), build_cortex_entry(cortex_binary));
    write_json(&config_file, &config)
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
        Err(e) => Err(AgentError::ConfigurationFailed {
            reason: format!("failed to read {}: {}", path.display(), e),
        }),
    }
}

/// Write a JSON value to a file with pretty formatting.
fn write_json(path: &Path, value: &Value) -> Result<(), AgentError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AgentError::ConfigurationFailed {
            reason: format!("failed to create directory {}: {}", parent.display(), e),
        })?;
    }
    let content = serde_json::to_string_pretty(value).map_err(|e| AgentError::ConfigurationFailed {
        reason: format!("failed to serialize config: {}", e),
    })?;
    fs::write(path, content).map_err(|e| AgentError::ConfigurationFailed {
        reason: format!("failed to write {}: {}", path.display(), e),
    })
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

    #[test]
    fn test_agent_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".cursor");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let mcp_file = config_path.join("mcp.json");
        assert!(mcp_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
        assert_eq!(content["mcpServers"]["cortex"]["args"][0], "serve");
    }

    #[test]
    fn test_configuration_idempotent() {
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
    fn test_zed_uses_context_servers_key() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".zed");
        fs::create_dir_all(&config_path).unwrap();

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

        // Zed uses "context_servers" as root key
        assert!(content["context_servers"]["cortex"].is_object());
    }

    #[test]
    fn test_copilot_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".github");
        fs::create_dir_all(&config_path).unwrap();

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

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
        assert_eq!(content["mcpServers"]["cortex"]["args"][0], "serve");
    }

    #[test]
    fn test_cline_roo_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".vscode");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "cline_roo".to_string(),
            display_name: "Cline/Roo".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }

    #[test]
    fn test_codex_cli_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".codex");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "codex_cli".to_string(),
            display_name: "Codex CLI".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }

    #[test]
    fn test_antigravity_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".antigravity");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "antigravity".to_string(),
            display_name: "Antigravity".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }

    #[test]
    fn test_supermaven_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".supermaven");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "supermaven".to_string(),
            display_name: "Supermaven".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }

    #[test]
    fn test_codeium_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".codeium");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "codeium".to_string(),
            display_name: "Codeium".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }

    #[test]
    fn test_tabnine_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".tabnine");
        fs::create_dir_all(&config_path).unwrap();

        let agent = DetectedAgent {
            name: "tabnine".to_string(),
            display_name: "Tabnine".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config_path.clone(),
        };

        let binary = PathBuf::from("/usr/local/bin/cortex");
        configure_agent(&agent, &binary).unwrap();

        let config_file = config_path.join("mcp.json");
        assert!(config_file.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "/usr/local/bin/cortex");
    }
}
