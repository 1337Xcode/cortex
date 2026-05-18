//! Agent detection: identifies installed AI coding agents.
//!
//! Detects 15 agents by checking for their configuration directories/files:
//! - Claude Code: ~/.claude/
//! - Cursor: .cursor/ in repo root
//! - Windsurf: ~/.windsurf/ or .windsurf/ in repo root
//! - VS Code: .vscode/ in repo root
//! - JetBrains: .idea/ in repo root
//! - Zed: ~/.config/zed/ or .zed/ in repo root
//! - Aider: .aider.conf.yml in repo root or ~/.aider.conf.yml
//! - Continue.dev: .continue/ in repo root or ~/.continue/
//! - GitHub Copilot: .github/ in repo root
//! - Cline/Roo: .vscode/ in repo root (shares VS Code config dir)
//! - Codex CLI: .codex/ in repo root
//! - Antigravity: .antigravity/ in repo root
//! - Supermaven: .supermaven/ in repo root
//! - Codeium: .codeium/ in repo root
//! - Tabnine: .tabnine/ in repo root

use std::path::{Path, PathBuf};

/// Represents a detected AI coding agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedAgent {
    /// Agent name (e.g., "claude_code", "cursor").
    pub name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// The root key used in the agent's MCP config JSON.
    pub root_key: String,
    /// Path to the agent's configuration directory or file.
    pub config_path: PathBuf,
}

/// Detect all installed AI coding agents.
///
/// Checks for 15 known agents by looking for their configuration
/// directories/files in the user's home directory and the repo root.
pub fn detect_installed_agents(repo_root: &Path) -> Vec<DetectedAgent> {
    let mut agents = Vec::new();
    let home = home_dir();

    // 1. Claude Code: ~/.claude/
    if let Some(ref home) = home {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() {
            agents.push(DetectedAgent {
                name: "claude_code".to_string(),
                display_name: "Claude Code".to_string(),
                root_key: "mcpServers".to_string(),
                config_path: claude_dir,
            });
        }
    }

    // 2. Cursor: .cursor/ in repo root
    let cursor_dir = repo_root.join(".cursor");
    if cursor_dir.exists() {
        agents.push(DetectedAgent {
            name: "cursor".to_string(),
            display_name: "Cursor".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: cursor_dir,
        });
    }

    // 3. Windsurf: ~/.windsurf/ or .windsurf/ in repo root
    let windsurf_repo = repo_root.join(".windsurf");
    if windsurf_repo.exists() {
        agents.push(DetectedAgent {
            name: "windsurf".to_string(),
            display_name: "Windsurf".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: windsurf_repo,
        });
    } else if let Some(ref home) = home {
        let windsurf_home = home.join(".windsurf");
        if windsurf_home.exists() {
            agents.push(DetectedAgent {
                name: "windsurf".to_string(),
                display_name: "Windsurf".to_string(),
                root_key: "mcpServers".to_string(),
                config_path: windsurf_home,
            });
        }
    }

    // 4. VS Code: .vscode/ in repo root
    let vscode_dir = repo_root.join(".vscode");
    if vscode_dir.exists() {
        agents.push(DetectedAgent {
            name: "vscode".to_string(),
            display_name: "VS Code".to_string(),
            root_key: "servers".to_string(),
            config_path: vscode_dir,
        });
    }

    // 5. JetBrains: .idea/ in repo root
    let idea_dir = repo_root.join(".idea");
    if idea_dir.exists() {
        agents.push(DetectedAgent {
            name: "jetbrains".to_string(),
            display_name: "JetBrains".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: idea_dir,
        });
    }

    // 6. Zed: ~/.config/zed/ or .zed/ in repo root
    let zed_repo = repo_root.join(".zed");
    if zed_repo.exists() {
        agents.push(DetectedAgent {
            name: "zed".to_string(),
            display_name: "Zed".to_string(),
            root_key: "context_servers".to_string(),
            config_path: zed_repo,
        });
    } else if let Some(ref home) = home {
        let zed_config = home.join(".config").join("zed");
        if zed_config.exists() {
            agents.push(DetectedAgent {
                name: "zed".to_string(),
                display_name: "Zed".to_string(),
                root_key: "context_servers".to_string(),
                config_path: zed_config,
            });
        }
    }

    // 7. Aider: .aider.conf.yml in repo root or ~/.aider.conf.yml
    let aider_repo = repo_root.join(".aider.conf.yml");
    if aider_repo.exists() {
        agents.push(DetectedAgent {
            name: "aider".to_string(),
            display_name: "Aider".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: aider_repo,
        });
    } else if let Some(ref home) = home {
        let aider_home = home.join(".aider.conf.yml");
        if aider_home.exists() {
            agents.push(DetectedAgent {
                name: "aider".to_string(),
                display_name: "Aider".to_string(),
                root_key: "mcpServers".to_string(),
                config_path: aider_home,
            });
        }
    }

    // 8. Continue.dev: .continue/ in repo root or ~/.continue/
    let continue_repo = repo_root.join(".continue");
    if continue_repo.exists() {
        agents.push(DetectedAgent {
            name: "continue_dev".to_string(),
            display_name: "Continue.dev".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: continue_repo,
        });
    } else if let Some(ref home) = home {
        let continue_home = home.join(".continue");
        if continue_home.exists() {
            agents.push(DetectedAgent {
                name: "continue_dev".to_string(),
                display_name: "Continue.dev".to_string(),
                root_key: "mcpServers".to_string(),
                config_path: continue_home,
            });
        }
    }

    // 9. GitHub Copilot: .github/ in repo root
    let github_dir = repo_root.join(".github");
    if github_dir.exists() {
        agents.push(DetectedAgent {
            name: "copilot".to_string(),
            display_name: "GitHub Copilot".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: github_dir,
        });
    }

    // 10. Cline/Roo: .vscode/ in repo root (shares config dir with VS Code)
    // Cline/Roo uses the same .vscode/mcp.json but with "mcpServers" root key
    let cline_dir = repo_root.join(".vscode");
    if cline_dir.exists() {
        agents.push(DetectedAgent {
            name: "cline_roo".to_string(),
            display_name: "Cline/Roo".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: cline_dir,
        });
    }

    // 11. Codex CLI: .codex/ in repo root
    let codex_dir = repo_root.join(".codex");
    if codex_dir.exists() {
        agents.push(DetectedAgent {
            name: "codex_cli".to_string(),
            display_name: "Codex CLI".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: codex_dir,
        });
    }

    // 12. Antigravity: .antigravity/ in repo root
    let antigravity_dir = repo_root.join(".antigravity");
    if antigravity_dir.exists() {
        agents.push(DetectedAgent {
            name: "antigravity".to_string(),
            display_name: "Antigravity".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: antigravity_dir,
        });
    }

    // 13. Supermaven: .supermaven/ in repo root
    let supermaven_dir = repo_root.join(".supermaven");
    if supermaven_dir.exists() {
        agents.push(DetectedAgent {
            name: "supermaven".to_string(),
            display_name: "Supermaven".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: supermaven_dir,
        });
    }

    // 14. Codeium: .codeium/ in repo root
    let codeium_dir = repo_root.join(".codeium");
    if codeium_dir.exists() {
        agents.push(DetectedAgent {
            name: "codeium".to_string(),
            display_name: "Codeium".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: codeium_dir,
        });
    }

    // 15. Tabnine: .tabnine/ in repo root
    let tabnine_dir = repo_root.join(".tabnine");
    if tabnine_dir.exists() {
        agents.push(DetectedAgent {
            name: "tabnine".to_string(),
            display_name: "Tabnine".to_string(),
            root_key: "mcpServers".to_string(),
            config_path: tabnine_dir,
        });
    }

    agents
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
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
    fn test_detect_agents_configured_correctly() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Create .cursor and .vscode directories
        std::fs::create_dir_all(repo_root.join(".cursor")).unwrap();
        std::fs::create_dir_all(repo_root.join(".vscode")).unwrap();
        std::fs::create_dir_all(repo_root.join(".zed")).unwrap();

        let agents = detect_installed_agents(repo_root);

        // Should detect cursor, vscode, cline_roo, and zed
        assert!(agents.iter().any(|a| a.name == "cursor"));
        assert!(agents.iter().any(|a| a.name == "vscode"));
        assert!(agents.iter().any(|a| a.name == "zed"));
        // Cline/Roo shares .vscode directory
        assert!(agents.iter().any(|a| a.name == "cline_roo"));

        // Verify root keys
        let cursor = agents.iter().find(|a| a.name == "cursor").unwrap();
        assert_eq!(cursor.root_key, "mcpServers");

        let vscode = agents.iter().find(|a| a.name == "vscode").unwrap();
        assert_eq!(vscode.root_key, "servers");

        let zed = agents.iter().find(|a| a.name == "zed").unwrap();
        assert_eq!(zed.root_key, "context_servers");

        let cline = agents.iter().find(|a| a.name == "cline_roo").unwrap();
        assert_eq!(cline.root_key, "mcpServers");
    }

    #[test]
    fn test_detect_agents_idempotent() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".cursor")).unwrap();

        let agents1 = detect_installed_agents(repo_root);
        let agents2 = detect_installed_agents(repo_root);

        assert_eq!(agents1, agents2);
    }

    #[test]
    fn test_no_agents_detected_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Empty directory - no agent config dirs
        let agents = detect_installed_agents(repo_root);

        // May detect home-based agents, but repo-based ones should be empty
        // Filter to only repo-based agents for this test
        let repo_agents: Vec<_> = agents
            .iter()
            .filter(|a| a.config_path.starts_with(repo_root))
            .collect();
        assert!(repo_agents.is_empty());
    }

    #[test]
    fn test_detect_copilot() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".github")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let copilot = agents.iter().find(|a| a.name == "copilot").unwrap();
        assert_eq!(copilot.display_name, "GitHub Copilot");
        assert_eq!(copilot.root_key, "mcpServers");
        assert_eq!(copilot.config_path, repo_root.join(".github"));
    }

    #[test]
    fn test_detect_codex_cli() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".codex")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let codex = agents.iter().find(|a| a.name == "codex_cli").unwrap();
        assert_eq!(codex.display_name, "Codex CLI");
        assert_eq!(codex.root_key, "mcpServers");
        assert_eq!(codex.config_path, repo_root.join(".codex"));
    }

    #[test]
    fn test_detect_antigravity() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".antigravity")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let ag = agents.iter().find(|a| a.name == "antigravity").unwrap();
        assert_eq!(ag.display_name, "Antigravity");
        assert_eq!(ag.root_key, "mcpServers");
        assert_eq!(ag.config_path, repo_root.join(".antigravity"));
    }

    #[test]
    fn test_detect_supermaven() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".supermaven")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let sm = agents.iter().find(|a| a.name == "supermaven").unwrap();
        assert_eq!(sm.display_name, "Supermaven");
        assert_eq!(sm.root_key, "mcpServers");
        assert_eq!(sm.config_path, repo_root.join(".supermaven"));
    }

    #[test]
    fn test_detect_codeium() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".codeium")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let ci = agents.iter().find(|a| a.name == "codeium").unwrap();
        assert_eq!(ci.display_name, "Codeium");
        assert_eq!(ci.root_key, "mcpServers");
        assert_eq!(ci.config_path, repo_root.join(".codeium"));
    }

    #[test]
    fn test_detect_tabnine() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        std::fs::create_dir_all(repo_root.join(".tabnine")).unwrap();

        let agents = detect_installed_agents(repo_root);
        let tn = agents.iter().find(|a| a.name == "tabnine").unwrap();
        assert_eq!(tn.display_name, "Tabnine");
        assert_eq!(tn.root_key, "mcpServers");
        assert_eq!(tn.config_path, repo_root.join(".tabnine"));
    }

    #[test]
    fn test_detect_all_15_agents() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Create all repo-based agent directories
        std::fs::create_dir_all(repo_root.join(".cursor")).unwrap();
        std::fs::create_dir_all(repo_root.join(".windsurf")).unwrap();
        std::fs::create_dir_all(repo_root.join(".vscode")).unwrap();
        std::fs::create_dir_all(repo_root.join(".idea")).unwrap();
        std::fs::create_dir_all(repo_root.join(".zed")).unwrap();
        std::fs::write(repo_root.join(".aider.conf.yml"), "").unwrap();
        std::fs::create_dir_all(repo_root.join(".continue")).unwrap();
        std::fs::create_dir_all(repo_root.join(".github")).unwrap();
        std::fs::create_dir_all(repo_root.join(".codex")).unwrap();
        std::fs::create_dir_all(repo_root.join(".antigravity")).unwrap();
        std::fs::create_dir_all(repo_root.join(".supermaven")).unwrap();
        std::fs::create_dir_all(repo_root.join(".codeium")).unwrap();
        std::fs::create_dir_all(repo_root.join(".tabnine")).unwrap();

        let agents = detect_installed_agents(repo_root);

        // Filter to repo-based agents only (exclude home-based like claude_code)
        let repo_agents: Vec<_> = agents
            .iter()
            .filter(|a| a.config_path.starts_with(repo_root))
            .collect();

        // Should detect at least 14 repo-based agents
        // (claude_code is home-based, so 14 of 15 are repo-detectable)
        assert!(
            repo_agents.len() >= 14,
            "Expected at least 14 repo-based agents, got {}",
            repo_agents.len()
        );

        // Verify all expected agents are present
        let names: Vec<&str> = repo_agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"cursor"));
        assert!(names.contains(&"windsurf"));
        assert!(names.contains(&"vscode"));
        assert!(names.contains(&"jetbrains"));
        assert!(names.contains(&"zed"));
        assert!(names.contains(&"aider"));
        assert!(names.contains(&"continue_dev"));
        assert!(names.contains(&"copilot"));
        assert!(names.contains(&"cline_roo"));
        assert!(names.contains(&"codex_cli"));
        assert!(names.contains(&"antigravity"));
        assert!(names.contains(&"supermaven"));
        assert!(names.contains(&"codeium"));
        assert!(names.contains(&"tabnine"));
    }
}
