//! Install command: configures AI coding agents to use Cortex.
//!
//! Detects all 25 supported AI coding agents and writes MCP server
//! configuration for each. The hardened flow:
//! 1. Detect installed agents via detect.rs (or use --platform flag)
//! 2. For each agent: create config dir, generate config, validate, write
//! 3. On permission error: report specific path and permissions
//! 4. Report success only after validation passes
//!
//! Idempotent: running twice produces identical config without duplication.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agents::configure::configure_agent;
use crate::agents::detect::detect_installed_agents;
use crate::agents::steering::{STEERING_TEMPLATE, write_steering_file};
use crate::config::Config;
use crate::error::AgentError;

/// Run the install command: detect and configure AI agents.
///
/// Uses the hardened configure_agent flow that:
/// - Creates config directories with fs::create_dir_all
/// - Validates generated config by parsing back
/// - Reports permission errors with specific file paths
/// - Reports success only after validation passes
pub fn run(config: &Config) -> Result<(), anyhow::Error> {
    let mut configured = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    println!("Detecting AI coding agents...");
    println!();

    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("cortex"));

    // Detect all installed agents
    let agents = detect_installed_agents(&config.repo_root);

    if agents.is_empty() {
        // If no agents detected, at minimum configure Cursor (common default)
        let cursor_agent = crate::agents::detect::DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: config.repo_root.join(".cursor"),
        };
        match configure_agent(&cursor_agent, &binary) {
            Ok(()) => configured.push("Cursor (created .cursor/)".to_string()),
            Err(e) => errors.push(format!("Cursor: {}", e)),
        }
    } else {
        // Configure each detected agent
        for agent in &agents {
            match configure_agent(agent, &binary) {
                Ok(()) => configured.push(agent.display_name.clone()),
                Err(AgentError::PermissionDenied { ref path }) => {
                    errors.push(format!(
                        "{}: Permission denied writing {}. Required: write access to {}",
                        agent.display_name, path, path
                    ));
                }
                Err(AgentError::ValidationFailed {
                    agent: ref agent_name,
                    ref reason,
                }) => {
                    errors.push(format!(
                        "{}: Generated config failed validation: {}",
                        agent_name, reason
                    ));
                }
                Err(e) => {
                    errors.push(format!("{}: {}", agent.display_name, e));
                }
            }
        }
    }

    if configured.is_empty() && errors.is_empty() {
        println!("No supported AI agents detected.");
    } else if !configured.is_empty() {
        println!("Configured Cortex MCP server for:");
        for agent in &configured {
            println!("  [ok] {}", agent);
        }
    }

    // Report any errors
    if !errors.is_empty() {
        println!();
        println!("Errors:");
        for err in &errors {
            println!("  [err] {}", err);
        }
    }

    println!();
    println!("Supported platforms:");
    println!("  - Claude Code       ~/.claude/settings.json");
    println!("  - Cursor            .cursor/mcp.json                  (or: cortex cursor install)");
    println!("  - Windsurf          .windsurf/mcp.json");
    println!("  - VS Code           .vscode/mcp.json");
    println!("  - Kiro              .kiro/settings/mcp.json           (or: cortex kiro install)");
    println!("  - Zed               .zed/settings.json");
    println!("  - JetBrains         .idea/mcp.json");
    println!("  - Cline/Roo         .vscode/mcp.json");
    println!("  - Aider             .aider.mcp.json");
    println!("  - Continue.dev      .continue/config.json");
    println!("  - GitHub Copilot    .github/copilot-mcp.json");
    println!("  - Codex CLI         .codex/mcp.json");
    println!("  - OpenCode          .opencode/mcp.json");
    println!("  - OpenClaw          .openclaw/mcp.json");
    println!("  - Factory Droid     .droid/mcp.json");
    println!("  - Trae              .trae/mcp.json");
    println!("  - Trae CN           .trae-cn/mcp.json");
    println!("  - Gemini CLI        .gemini/settings.json");
    println!("  - Hermes            .hermes/mcp.json");
    println!("  - Kimi Code         .kimi/mcp.json");
    println!("  - Pi                .pi/mcp.json");
    println!(
        "  - Google Antigravity ~/.antigravity/mcp.json          (or: cortex antigravity install)"
    );
    println!("  - Supermaven        .supermaven/mcp.json");
    println!("  - Codeium           .codeium/mcp.json");
    println!("  - Tabnine           .tabnine/mcp.json");
    println!();
    println!("Use --platform <name> to configure a specific agent.");

    // Write workspace-level .cortex/mcp.json for VS Code, Cursor, and Kiro auto-discovery
    write_workspace_mcp_config(&config.repo_root)?;

    // Generate agent steering file after successful install
    generate_agent_steering(&config.repo_root);

    Ok(())
}

/// Write a workspace-level .cortex/mcp.json file in the repo root.
/// This config gets picked up by VS Code, Cursor, and Kiro automatically.
fn write_workspace_mcp_config(repo_root: &Path) -> Result<(), anyhow::Error> {
    let cortex_dir = repo_root.join(".cortex");
    fs::create_dir_all(&cortex_dir)?;

    let mcp_config = serde_json::json!({
        "mcpServers": {
            "cortex": {
                "command": "cortex",
                "args": ["serve"],
                "env": {
                    "CORTEX_REPO_ROOT": "."
                }
            }
        }
    });

    let mcp_path = cortex_dir.join("mcp.json");

    // Generate and validate before writing
    let content = serde_json::to_string_pretty(&mcp_config)?;
    let _: Value = serde_json::from_str(&content)?;

    write_json_file(&mcp_path, &mcp_config)?;
    println!("  [ok] Wrote .cortex/mcp.json (workspace-level MCP config)");
    Ok(())
}

/// Detect the active AI agent environment by checking for agent-specific
/// directories and files in the repo root.
///
/// Returns the agent name if detected, or `None` for fallback.
fn detect_active_agent(repo_root: &Path) -> Option<&'static str> {
    // Check in priority order (most specific first)
    if repo_root.join(".cursor").is_dir() {
        return Some("cursor");
    }
    if repo_root.join(".claude").is_dir() {
        return Some("claude code");
    }
    if repo_root.join(".kiro").is_dir() {
        return Some("kiro");
    }
    if repo_root.join(".windsurfrules").exists() {
        return Some("windsurf");
    }
    if repo_root
        .join(".github")
        .join("copilot-instructions.md")
        .exists()
    {
        return Some("copilot");
    }
    None
}

/// Generate an agent steering file after successful install.
///
/// Detects the active agent environment and writes the Cortex MCP tool
/// preference guide to the appropriate location. If no known agent is
/// detected, writes to the fallback path and informs the user.
fn generate_agent_steering(repo_root: &Path) {
    println!();
    println!("Writing agent steering file...");

    let agent_name = detect_active_agent(repo_root);

    let effective_agent = agent_name.unwrap_or("fallback");

    match write_steering_file(effective_agent, repo_root, STEERING_TEMPLATE) {
        Ok(()) => {
            let path = crate::agents::steering::steering_file_path(effective_agent, repo_root);
            if agent_name.is_some() {
                println!(
                    "  [ok] Wrote steering file for {} -> {}",
                    effective_agent,
                    path.display()
                );
            } else {
                println!("  [ok] Wrote generic steering file -> {}", path.display());
                println!(
                    "    No known agent environment detected (Cursor, Claude Code, Kiro, Windsurf, Copilot)."
                );
                println!(
                    "    You can manually copy .cortex/steering.md to your agent's rules directory."
                );
            }
        }
        Err(e) => {
            eprintln!("  [warn] Failed to write steering file: {}", e);
        }
    }
}

/// Write a JSON value to a file with pretty formatting.
fn write_json_file(path: &Path, value: &Value) -> Result<(), anyhow::Error> {
    let content = serde_json::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_active_agent_cursor() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".cursor")).unwrap();

        assert_eq!(detect_active_agent(root), Some("cursor"));
    }

    #[test]
    fn test_detect_active_agent_claude_code() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".claude")).unwrap();

        assert_eq!(detect_active_agent(root), Some("claude code"));
    }

    #[test]
    fn test_detect_active_agent_kiro() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".kiro")).unwrap();

        assert_eq!(detect_active_agent(root), Some("kiro"));
    }

    #[test]
    fn test_detect_active_agent_windsurf() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".windsurfrules"), "rules").unwrap();

        assert_eq!(detect_active_agent(root), Some("windsurf"));
    }

    #[test]
    fn test_detect_active_agent_copilot() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let github_dir = root.join(".github");
        fs::create_dir_all(&github_dir).unwrap();
        fs::write(github_dir.join("copilot-instructions.md"), "instructions").unwrap();

        assert_eq!(detect_active_agent(root), Some("copilot"));
    }

    #[test]
    fn test_detect_active_agent_none() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        assert_eq!(detect_active_agent(root), None);
    }

    #[test]
    fn test_detect_active_agent_priority_cursor_over_claude() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Both present, cursor should win (checked first)
        fs::create_dir_all(root.join(".cursor")).unwrap();
        fs::create_dir_all(root.join(".claude")).unwrap();

        assert_eq!(detect_active_agent(root), Some("cursor"));
    }

    #[test]
    fn test_generate_agent_steering_writes_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".cursor")).unwrap();

        generate_agent_steering(root);

        let steering_path = root.join(".cursor").join("rules").join("cortex.mdc");
        assert!(steering_path.exists());
        let content = fs::read_to_string(&steering_path).unwrap();
        assert!(content.contains("Cortex MCP Tools"));
    }

    #[test]
    fn test_generate_agent_steering_fallback() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // No agent directories present

        generate_agent_steering(root);

        let steering_path = root.join(".cortex").join("steering.md");
        assert!(steering_path.exists());
        let content = fs::read_to_string(&steering_path).unwrap();
        assert!(content.contains("Cortex MCP Tools"));
    }
}
