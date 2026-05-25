//! Federate command: manage multi-repo federation.
//!
//! `cortex federate add ../auth-service` adds a repository to the federation.
//! `cortex federate list` shows all federated repos.
//! `cortex federate remove auth-service` removes one.
//!
//! Federated queries search across all repos in the federation.
//! Each repo maintains its own .cortex/ directory. Federation stores
//! a manifest at .cortex/federation.json listing all member repos.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Federation manifest stored at `.cortex/federation.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationManifest {
    pub repos: Vec<FederatedRepo>,
}

/// A single federated repository entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedRepo {
    pub name: String,
    pub path: String,
    pub added_at: u64,
}

/// Add a repository to the federation.
pub fn add(data_dir: &Path, repo_path: &Path) -> Result<(), anyhow::Error> {
    // Resolve the path
    let resolved = if repo_path.is_absolute() {
        repo_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(repo_path)
    };

    // Validate the path exists
    if !resolved.exists() {
        anyhow::bail!("path does not exist: {}", resolved.display());
    }

    // Check if it has a .cortex/ directory
    let cortex_dir = resolved.join(".cortex");
    if !cortex_dir.exists() {
        anyhow::bail!(
            "path '{}' does not have a .cortex/ directory. Run `cortex index` in that repository first.",
            resolved.display()
        );
    }

    // Derive the repo name from the directory name
    let name = resolved
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Load existing manifest
    let manifest_path = data_dir.join("federation.json");
    let mut manifest = load_manifest(&manifest_path)?;

    // Check for duplicates
    if manifest.repos.iter().any(|r| r.name == name) {
        anyhow::bail!("repository '{}' is already in the federation", name);
    }

    // Add the new repo
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    manifest.repos.push(FederatedRepo {
        name,
        path: resolved.to_string_lossy().to_string(),
        added_at: now,
    });

    // Save manifest
    save_manifest(&manifest_path, &manifest)?;

    println!(
        "Added '{}' to federation ({} total repos)",
        resolved.display(),
        manifest.repos.len()
    );

    Ok(())
}

/// List all federated repositories.
pub fn list(data_dir: &Path) -> Result<(), anyhow::Error> {
    let manifest_path = data_dir.join("federation.json");
    let manifest = load_manifest(&manifest_path)?;

    if manifest.repos.is_empty() {
        println!("No federated repositories. Use `cortex federate add <path>` to add one.");
        return Ok(());
    }

    println!("Federated repositories ({}):", manifest.repos.len());
    println!("{:<20} {:<50} ADDED", "NAME", "PATH");
    println!("{}", "-".repeat(80));

    for repo in &manifest.repos {
        let path_display = if repo.path.len() > 48 {
            format!("...{}", &repo.path[repo.path.len() - 45..])
        } else {
            repo.path.clone()
        };

        // Check if the repo is still accessible
        let status = if Path::new(&repo.path).join(".cortex").exists() {
            ""
        } else {
            " (unreachable)"
        };

        println!(
            "{:<20} {:<50} {}{}",
            repo.name, path_display, repo.added_at, status
        );
    }

    Ok(())
}

/// Remove a repository from the federation by name.
pub fn remove(data_dir: &Path, name: &str) -> Result<(), anyhow::Error> {
    let manifest_path = data_dir.join("federation.json");
    let mut manifest = load_manifest(&manifest_path)?;

    let original_len = manifest.repos.len();
    manifest.repos.retain(|r| r.name != name);

    if manifest.repos.len() == original_len {
        anyhow::bail!("repository '{}' not found in federation", name);
    }

    save_manifest(&manifest_path, &manifest)?;

    println!(
        "Removed '{}' from federation ({} remaining)",
        name,
        manifest.repos.len()
    );

    Ok(())
}

/// Load the federation manifest from disk, returning an empty manifest if the file doesn't exist.
fn load_manifest(path: &Path) -> Result<FederationManifest, anyhow::Error> {
    if !path.exists() {
        return Ok(FederationManifest { repos: Vec::new() });
    }

    let content = std::fs::read_to_string(path)?;
    let manifest: FederationManifest = serde_json::from_str(&content)?;
    Ok(manifest)
}

/// Save the federation manifest to disk.
fn save_manifest(path: &Path, manifest: &FederationManifest) -> Result<(), anyhow::Error> {
    let json = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Get all federated repo paths (for use by cross-repo queries).
pub fn get_federated_paths(data_dir: &Path) -> Result<Vec<PathBuf>, anyhow::Error> {
    let manifest_path = data_dir.join("federation.json");
    let manifest = load_manifest(&manifest_path)?;

    Ok(manifest
        .repos
        .iter()
        .map(|r| PathBuf::from(&r.path))
        .filter(|p| p.exists())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_manifest_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("federation.json");
        let manifest = load_manifest(&path).unwrap();
        assert!(manifest.repos.is_empty());
    }

    #[test]
    fn test_save_and_load_manifest() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("federation.json");

        let manifest = FederationManifest {
            repos: vec![FederatedRepo {
                name: "test-repo".to_string(),
                path: "/tmp/test-repo".to_string(),
                added_at: 1234567890,
            }],
        };

        save_manifest(&path, &manifest).unwrap();
        let loaded = load_manifest(&path).unwrap();

        assert_eq!(loaded.repos.len(), 1);
        assert_eq!(loaded.repos[0].name, "test-repo");
        assert_eq!(loaded.repos[0].path, "/tmp/test-repo");
        assert_eq!(loaded.repos[0].added_at, 1234567890);
    }

    #[test]
    fn test_add_validates_cortex_dir() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // Create a repo dir without .cortex/
        let repo_dir = tmp.path().join("no-cortex-repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let result = add(&data_dir, &repo_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".cortex/"));
    }

    #[test]
    fn test_add_and_remove() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // Create a repo dir with .cortex/
        let repo_dir = tmp.path().join("my-repo");
        std::fs::create_dir_all(repo_dir.join(".cortex")).unwrap();

        // Add it
        add(&data_dir, &repo_dir).unwrap();

        // Verify it's in the manifest
        let manifest_path = data_dir.join("federation.json");
        let manifest = load_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.repos.len(), 1);
        assert_eq!(manifest.repos[0].name, "my-repo");

        // Remove it
        remove(&data_dir, "my-repo").unwrap();

        let manifest = load_manifest(&manifest_path).unwrap();
        assert!(manifest.repos.is_empty());
    }

    #[test]
    fn test_remove_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let result = remove(&data_dir, "nonexistent");
        assert!(result.is_err());
    }
}
