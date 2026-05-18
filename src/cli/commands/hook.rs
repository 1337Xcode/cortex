//! Hook command: manage git hooks for automatic re-indexing.
//!
//! `cortex hook install` sets up a post-commit hook that re-indexes
//! changed files after each commit. The hook calls `cortex index`
//! which skips unchanged files via content hashing.

use std::fs;
use std::path::Path;

const HOOK_MARKER: &str = "# cortex-auto-index";

const HOOK_SCRIPT: &str = r#"#!/bin/sh
# cortex-auto-index
# Automatically re-index changed files after commit.
# Installed by `cortex hook install`. Remove with `cortex hook remove`.

# Get list of changed files in the last commit
changed_files=$(git diff-tree --no-commit-id --name-only -r HEAD 2>/dev/null)

if [ -n "$changed_files" ]; then
    # Run cortex index in the background (non-blocking)
    cortex index >/dev/null 2>&1 &
fi
"#;

/// Install the post-commit hook.
pub fn install(repo_root: &Path) -> Result<(), anyhow::Error> {
    let hooks_dir = repo_root.join(".git").join("hooks");
    if !hooks_dir.exists() {
        anyhow::bail!(
            "No .git/hooks directory found at {}. Is this a git repository?",
            repo_root.display()
        );
    }

    let hook_path = hooks_dir.join("post-commit");

    if hook_path.exists() {
        let existing = fs::read_to_string(&hook_path)?;
        if existing.contains(HOOK_MARKER) {
            println!("Cortex post-commit hook is already installed.");
            return Ok(());
        }

        // Append to existing hook
        let mut content = existing;
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
        content.push_str(HOOK_SCRIPT.trim_start_matches("#!/bin/sh\n"));
        fs::write(&hook_path, content)?;
        println!("Appended Cortex hook to existing post-commit hook.");
    } else {
        fs::write(&hook_path, HOOK_SCRIPT)?;
        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&hook_path, perms)?;
        }
        println!("Installed Cortex post-commit hook.");
    }

    println!("Changed files will be re-indexed automatically after each commit.");
    Ok(())
}

/// Remove the Cortex hook from post-commit.
pub fn remove(repo_root: &Path) -> Result<(), anyhow::Error> {
    let hook_path = repo_root.join(".git").join("hooks").join("post-commit");

    if !hook_path.exists() {
        println!("No post-commit hook found.");
        return Ok(());
    }

    let content = fs::read_to_string(&hook_path)?;
    if !content.contains(HOOK_MARKER) {
        println!("No Cortex hook found in post-commit.");
        return Ok(());
    }

    // Remove the cortex section (everything from the marker line to end of file)
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines: Vec<&str> = Vec::new();

    for line in &lines {
        if line.contains(HOOK_MARKER) {
            break; // Stop here, skip everything from marker onwards
        }
        new_lines.push(line);
    }

    let new_content = new_lines.join("\n");
    let trimmed = new_content.trim();
    if trimmed.is_empty() || trimmed == "#!/bin/sh" || trimmed == "#!/bin/bash" {
        fs::remove_file(&hook_path)?;
        println!("Removed post-commit hook (was only Cortex).");
    } else {
        fs::write(&hook_path, new_content)?;
        println!("Removed Cortex section from post-commit hook.");
    }

    Ok(())
}

/// Show hook status.
pub fn status(repo_root: &Path) -> Result<(), anyhow::Error> {
    let hook_path = repo_root.join(".git").join("hooks").join("post-commit");

    if !hook_path.exists() {
        println!("No post-commit hook installed.");
        return Ok(());
    }

    let content = fs::read_to_string(&hook_path)?;
    if content.contains(HOOK_MARKER) {
        println!("Cortex post-commit hook: installed");
        println!("Location: {}", hook_path.display());
    } else {
        println!("Post-commit hook exists but does not contain Cortex hook.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_git_repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".git").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        tmp
    }

    #[test]
    fn test_install_creates_hook() {
        let tmp = setup_git_repo();
        install(tmp.path()).unwrap();

        let hook_path = tmp.path().join(".git").join("hooks").join("post-commit");
        assert!(hook_path.exists());

        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(HOOK_MARKER));
        assert!(content.contains("cortex index"));
    }

    #[test]
    fn test_install_idempotent() {
        let tmp = setup_git_repo();
        install(tmp.path()).unwrap();
        install(tmp.path()).unwrap();

        let hook_path = tmp.path().join(".git").join("hooks").join("post-commit");
        let content = fs::read_to_string(&hook_path).unwrap();

        // Should only contain the marker once
        let count = content.matches(HOOK_MARKER).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_remove_deletes_hook() {
        let tmp = setup_git_repo();
        install(tmp.path()).unwrap();
        remove(tmp.path()).unwrap();

        let hook_path = tmp.path().join(".git").join("hooks").join("post-commit");
        assert!(!hook_path.exists());
    }

    #[test]
    fn test_status_shows_installed() {
        let tmp = setup_git_repo();
        install(tmp.path()).unwrap();
        // Just verify it doesn't panic
        status(tmp.path()).unwrap();
    }
}
