//! Install command: configures AI coding agents to use Cortex.
//!
//! Basic version detects Claude Code and Cursor, writing MCP server
//! configuration to their respective config files. Idempotent: running
//! twice produces identical config without duplication.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::Config;

/// Run the install command: detect and configure AI agents.
pub fn run(config: &Config) -> Result<(), anyhow::Error> {
    let mut configured = Vec::new();

    println!("Detecting AI coding agents...");
    println!();

    // Detect and configure Claude Code
    if let Some(claude_dir) = detect_claude_code() {
        configure_claude_code(&claude_dir, &config.repo_root)?;
        configured.push("Claude Code");
    }

    // Detect and configure Cursor
    let cursor_dir = config.repo_root.join(".cursor");
    if detect_cursor(&config.repo_root) {
        configure_cursor(&cursor_dir, &config.repo_root)?;
        configured.push("Cursor");
    } else {
        // Create .cursor directory and configure anyway if user runs install
        fs::create_dir_all(&cursor_dir)?;
        configure_cursor(&cursor_dir, &config.repo_root)?;
        configured.push("Cursor (created .cursor/)");
    }

    if configured.is_empty() {
        println!("No supported AI agents detected.");
    } else {
        println!("Configured Cortex MCP server for:");
        for agent in &configured {
            println!("  ✓ {}", agent);
        }
    }

    println!();
    println!("Supported platforms:");
    println!("  • Claude Code       ~/.claude/settings.json");
    println!("  • Cursor            .cursor/mcp.json                  (or: cortex cursor install)");
    println!("  • Windsurf          .windsurf/mcp.json");
    println!("  • VS Code           .vscode/mcp.json");
    println!("  • Kiro              .kiro/settings/mcp.json           (or: cortex kiro install)");
    println!("  • Zed               .zed/settings.json");
    println!("  • JetBrains         .idea/mcp.json");
    println!("  • Cline/Roo         .vscode/mcp.json");
    println!("  • Aider             .aider.mcp.json");
    println!("  • Continue.dev      .continue/config.json");
    println!("  • GitHub Copilot    .github/copilot-mcp.json");
    println!("  • Codex CLI         .codex/mcp.json");
    println!("  • OpenCode          .opencode/mcp.json");
    println!("  • OpenClaw          .openclaw/mcp.json");
    println!("  • Factory Droid     .droid/mcp.json");
    println!("  • Trae              .trae/mcp.json");
    println!("  • Trae CN           .trae-cn/mcp.json");
    println!("  • Gemini CLI        .gemini/settings.json");
    println!("  • Hermes            .hermes/mcp.json");
    println!("  • Kimi Code         .kimi/mcp.json");
    println!("  • Pi                .pi/mcp.json");
    println!("  • Google Antigravity ~/.antigravity/mcp.json          (or: cortex antigravity install)");
    println!("  • Supermaven        .supermaven/mcp.json");
    println!("  • Codeium           .codeium/mcp.json");
    println!("  • Tabnine           .tabnine/mcp.json");
    println!();
    println!("Use --platform <name> to configure a specific agent.");

    // Write workspace-level .cortex/mcp.json for VS Code, Cursor, and Kiro auto-discovery
    write_workspace_mcp_config(&config.repo_root)?;

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
    write_json_file(&mcp_path, &mcp_config)?;
    println!("  ✓ Wrote .cortex/mcp.json (workspace-level MCP config)");
    Ok(())
}

/// Build the MCP server configuration JSON for Cortex.
fn build_cortex_mcp_config(repo_root: &Path) -> Value {
    serde_json::json!({
        "command": "cortex",
        "args": ["serve"],
        "env": {
            "CORTEX_REPO_ROOT": repo_root.to_string_lossy()
        }
    })
}

/// Detect Claude Code by checking for ~/.claude/ directory.
fn detect_claude_code() -> Option<PathBuf> {
    let home = dirs_path()?;
    let claude_dir = home.join(".claude");
    if claude_dir.exists() {
        Some(claude_dir)
    } else {
        None
    }
}

/// Detect Cursor by checking for .cursor/ in repo root.
fn detect_cursor(repo_root: &Path) -> bool {
    repo_root.join(".cursor").exists()
}

/// Configure Claude Code by writing to ~/.claude/settings.json.
fn configure_claude_code(claude_dir: &Path, repo_root: &Path) -> Result<(), anyhow::Error> {
    let settings_path = claude_dir.join("settings.json");
    let mut settings = load_json_file(&settings_path)?;

    // Ensure mcpServers key exists
    if !settings.is_object() {
        settings = serde_json::json!({});
    }
    let obj = settings.as_object_mut().unwrap();
    if !obj.contains_key("mcpServers") {
        obj.insert("mcpServers".to_string(), serde_json::json!({}));
    }

    // Set the cortex entry (idempotent: overwrites if exists)
    let mcp_servers = obj.get_mut("mcpServers").unwrap().as_object_mut().unwrap();
    mcp_servers.insert("cortex".to_string(), build_cortex_mcp_config(repo_root));

    // Write back
    write_json_file(&settings_path, &settings)?;
    Ok(())
}

/// Configure Cursor by writing to .cursor/mcp.json.
fn configure_cursor(cursor_dir: &Path, repo_root: &Path) -> Result<(), anyhow::Error> {
    let mcp_path = cursor_dir.join("mcp.json");
    let mut config = load_json_file(&mcp_path)?;

    // Ensure mcpServers key exists
    if !config.is_object() {
        config = serde_json::json!({});
    }
    let obj = config.as_object_mut().unwrap();
    if !obj.contains_key("mcpServers") {
        obj.insert("mcpServers".to_string(), serde_json::json!({}));
    }

    // Set the cortex entry (idempotent: overwrites if exists)
    let mcp_servers = obj.get_mut("mcpServers").unwrap().as_object_mut().unwrap();
    mcp_servers.insert("cortex".to_string(), build_cortex_mcp_config(repo_root));

    // Write back
    write_json_file(&mcp_path, &config)?;
    Ok(())
}

/// Load a JSON file, returning an empty object if the file doesn't exist.
fn load_json_file(path: &Path) -> Result<Value, anyhow::Error> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.trim().is_empty() {
                Ok(serde_json::json!({}))
            } else {
                Ok(serde_json::from_str(&content)?)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(e.into()),
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

/// Get the user's home directory.
fn dirs_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_build_cortex_mcp_config() {
        let repo_root = Path::new("/tmp/my-repo");
        let config = build_cortex_mcp_config(repo_root);

        assert_eq!(config["command"], "cortex");
        assert_eq!(config["args"][0], "serve");
        assert_eq!(config["env"]["CORTEX_REPO_ROOT"], "/tmp/my-repo");
    }

    #[test]
    fn test_configure_cursor_creates_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let cursor_dir = tmp.path().join(".cursor");
        fs::create_dir_all(&cursor_dir).unwrap();

        let repo_root = tmp.path();
        configure_cursor(&cursor_dir, repo_root).unwrap();

        let mcp_path = cursor_dir.join("mcp.json");
        assert!(mcp_path.exists());

        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "cortex");
        assert_eq!(content["mcpServers"]["cortex"]["args"][0], "serve");
    }

    #[test]
    fn test_configure_cursor_idempotent() {
        let tmp = TempDir::new().unwrap();
        let cursor_dir = tmp.path().join(".cursor");
        fs::create_dir_all(&cursor_dir).unwrap();

        let repo_root = tmp.path();

        // Run twice
        configure_cursor(&cursor_dir, repo_root).unwrap();
        configure_cursor(&cursor_dir, repo_root).unwrap();

        let mcp_path = cursor_dir.join("mcp.json");
        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();

        // Should have exactly one cortex entry
        let mcp_servers = content["mcpServers"].as_object().unwrap();
        assert_eq!(mcp_servers.len(), 1);
        assert!(mcp_servers.contains_key("cortex"));
    }

    #[test]
    fn test_configure_cursor_preserves_existing_entries() {
        let tmp = TempDir::new().unwrap();
        let cursor_dir = tmp.path().join(".cursor");
        fs::create_dir_all(&cursor_dir).unwrap();

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
            cursor_dir.join("mcp.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        let repo_root = tmp.path();
        configure_cursor(&cursor_dir, repo_root).unwrap();

        let mcp_path = cursor_dir.join("mcp.json");
        let content: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();

        // Should have both entries
        let mcp_servers = content["mcpServers"].as_object().unwrap();
        assert_eq!(mcp_servers.len(), 2);
        assert!(mcp_servers.contains_key("cortex"));
        assert!(mcp_servers.contains_key("other-server"));
    }

    #[test]
    fn test_configure_claude_code() {
        let tmp = TempDir::new().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();

        let repo_root = tmp.path();
        configure_claude_code(&claude_dir, repo_root).unwrap();

        let settings_path = claude_dir.join("settings.json");
        assert!(settings_path.exists());

        let content: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(content["mcpServers"]["cortex"].is_object());
        assert_eq!(content["mcpServers"]["cortex"]["command"], "cortex");
    }

    #[test]
    fn test_configure_claude_code_idempotent() {
        let tmp = TempDir::new().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();

        let repo_root = tmp.path();

        // Run twice
        configure_claude_code(&claude_dir, repo_root).unwrap();
        configure_claude_code(&claude_dir, repo_root).unwrap();

        let settings_path = claude_dir.join("settings.json");
        let content: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();

        let mcp_servers = content["mcpServers"].as_object().unwrap();
        assert_eq!(mcp_servers.len(), 1);
        assert!(mcp_servers.contains_key("cortex"));
    }

    #[test]
    fn test_load_json_file_nonexistent() {
        let result = load_json_file(Path::new("/nonexistent/file.json")).unwrap();
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn test_load_json_file_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.json");
        fs::write(&path, "").unwrap();

        let result = load_json_file(&path).unwrap();
        assert_eq!(result, serde_json::json!({}));
    }
}
