//! Uninstall command: removes all Cortex traces from the system.
//!
//! Deletes:
//! - ~/.cortex/bin/ (the binary)
//! - .cortex-data/ in the current repo (graph database)
//! - .cortex/ in the current repo (config, mcp.json, steering)
//! - Shell PATH entries added by the installer
//! - MCP config entries written by `cortex install`

use std::fs;
use std::path::Path;

/// Run the uninstall command: remove all Cortex traces.
pub fn run_uninstall(repo_root: &Path, data_dir: &Path) {
    println!("Uninstalling Cortex...\n");

    let mut removed = Vec::new();
    let mut errors = Vec::new();

    // 1. Remove the graph database
    if data_dir.exists() {
        match fs::remove_dir_all(data_dir) {
            Ok(()) => removed.push(format!("Graph database: {}", data_dir.display())),
            Err(e) => errors.push(format!("Cannot remove {}: {}", data_dir.display(), e)),
        }
    }

    // 2. Remove .cortex/ config directory in repo
    let cortex_config = repo_root.join(".cortex");
    if cortex_config.exists() {
        match fs::remove_dir_all(&cortex_config) {
            Ok(()) => removed.push(format!("Config directory: {}", cortex_config.display())),
            Err(e) => errors.push(format!("Cannot remove {}: {}", cortex_config.display(), e)),
        }
    }

    // 3. Remove ~/.cortex/bin/ (the installed binary)
    let home = home_dir();
    if let Some(ref home) = home {
        let bin_dir = home.join(".cortex").join("bin");
        if bin_dir.exists() {
            match fs::remove_dir_all(&bin_dir) {
                Ok(()) => removed.push(format!("Binary: {}", bin_dir.display())),
                Err(e) => errors.push(format!("Cannot remove {}: {}", bin_dir.display(), e)),
            }
        }

        // 4. Remove ~/.cortex/ if empty after bin removal
        let cortex_home = home.join(".cortex");
        if cortex_home.exists()
            && fs::read_dir(&cortex_home)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(&cortex_home);
            removed.push(format!("Home directory: {}", cortex_home.display()));
        }

        // 5. Clean shell config PATH entries (Unix only)
        #[cfg(not(target_os = "windows"))]
        {
            let install_dir = home
                .join(".cortex")
                .join("bin")
                .to_string_lossy()
                .to_string();
            for shell_file in &[".bashrc", ".zshrc", ".profile"] {
                let path = home.join(shell_file);
                if let Ok(content) = fs::read_to_string(&path) {
                    if content.contains(&install_dir) {
                        let cleaned: String = content
                            .lines()
                            .filter(|line| {
                                !line.contains(&install_dir)
                                    && !line.contains("# Added by cortex installer")
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if fs::write(&path, cleaned).is_ok() {
                            removed.push(format!("PATH entry in {}", shell_file));
                        }
                    }
                }
            }
        }
    }

    // 6. Remove agent steering files
    let steering_locations = [
        repo_root.join(".cursor").join("rules").join("cortex.mdc"),
        repo_root.join(".claude").join("rules").join("cortex.md"),
        repo_root.join(".kiro").join("steering").join("cortex.md"),
        repo_root.join(".github").join("copilot-instructions.md"),
    ];
    for path in &steering_locations {
        let dominated_by_cortex = fs::read_to_string(path)
            .map(|c| c.contains("Cortex MCP Tools"))
            .unwrap_or(false);
        if dominated_by_cortex && fs::remove_file(path).is_ok() {
            removed.push(format!("Steering file: {}", path.display()));
        }
    }

    // Print results
    if !removed.is_empty() {
        println!("Removed:");
        for item in &removed {
            println!("  [ok] {}", item);
        }
    }

    if !errors.is_empty() {
        println!("\nErrors:");
        for err in &errors {
            println!("  [err] {}", err);
        }
    }

    if removed.is_empty() && errors.is_empty() {
        println!("Nothing to remove. Cortex was not installed in this location.");
    } else {
        println!("\nCortex has been uninstalled.");
        if home.is_some() {
            println!("Note: Restart your terminal for PATH changes to take effect.");
        }
    }
}

/// Get the user's home directory.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}
