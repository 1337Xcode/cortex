//! Clone command: clone a remote repository and index it.
//!
//! `cortex clone <url>` clones a git repository to a local directory,
//! then runs a full index on it. Useful for quickly analyzing any
//! public repository without manual setup.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the clone command: git clone + cortex index.
pub fn run(url: &str, dir: Option<&Path>) -> Result<(), anyhow::Error> {
    // Determine target directory from URL if not specified
    let target_dir = match dir {
        Some(d) => d.to_path_buf(),
        None => extract_repo_name(url)?,
    };

    println!("Cloning {} into {}", url, target_dir.display());

    // Run git clone
    let status = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(&target_dir)
        .status()?;

    if !status.success() {
        anyhow::bail!("git clone failed with exit code: {}", status);
    }

    println!("Clone complete. Indexing...");

    // Run cortex index on the cloned directory
    let cortex_bin = std::env::current_exe()?;
    let status = Command::new(&cortex_bin)
        .arg("index")
        .env("CORTEX_REPO_ROOT", &target_dir)
        .status()?;

    if !status.success() {
        anyhow::bail!("cortex index failed with exit code: {}", status);
    }

    println!();
    println!("Done. Repository indexed at {}", target_dir.display());
    println!();
    println!("Next steps:");
    println!("  cortex serve    # start MCP server (from the cloned directory)");
    println!("  cortex report   # generate a human-readable report");
    println!("  cortex security report   # run security analysis");
    println!("  cortex viz --export graph.html   # export interactive graph");

    Ok(())
}

/// Extract repository name from a git URL.
///
/// Handles:
/// - https://github.com/user/repo.git -> repo
/// - https://github.com/user/repo -> repo
/// - git@github.com:user/repo.git -> repo
fn extract_repo_name(url: &str) -> Result<PathBuf, anyhow::Error> {
    let name = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .or_else(|| url.rsplit(':').next().map(|s| s.trim_end_matches(".git")))
        .ok_or_else(|| anyhow::anyhow!("could not extract repository name from URL: {}", url))?;

    Ok(PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_name_https() {
        let name = extract_repo_name("https://github.com/torvalds/linux.git").unwrap();
        assert_eq!(name, PathBuf::from("linux"));
    }

    #[test]
    fn test_extract_repo_name_https_no_git() {
        let name = extract_repo_name("https://github.com/torvalds/linux").unwrap();
        assert_eq!(name, PathBuf::from("linux"));
    }

    #[test]
    fn test_extract_repo_name_ssh() {
        let name = extract_repo_name("git@github.com:torvalds/linux.git").unwrap();
        assert_eq!(name, PathBuf::from("linux"));
    }

    #[test]
    fn test_extract_repo_name_trailing_slash() {
        let name = extract_repo_name("https://github.com/user/repo/").unwrap();
        assert_eq!(name, PathBuf::from("repo"));
    }
}
