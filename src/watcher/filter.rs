//! Watch filter for excluding files from file system events.
//!
//! Loads patterns from .gitignore and .cortex-ignore files and applies
//! the same exclusion logic as the indexing pipeline.

use std::fs;
use std::path::Path;

/// Directories to always exclude from watching.
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".cortex",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
];

/// Filter for determining which file events should be emitted.
///
/// Loads patterns from .gitignore and .cortex-ignore and always excludes
/// well-known directories like .git, node_modules, target, etc.
#[derive(Debug, Clone)]
pub struct WatchFilter {
    excluded_dirs: Vec<String>,
    gitignore_patterns: Vec<String>,
    cortex_ignore_patterns: Vec<String>,
}

impl WatchFilter {
    /// Create a new WatchFilter by loading patterns from the repository root.
    ///
    /// Reads .gitignore and .cortex-ignore files if they exist.
    pub fn new(repo_root: &Path) -> Self {
        let gitignore_patterns = load_ignore_patterns(repo_root, ".gitignore");
        let cortex_ignore_patterns = load_ignore_patterns(repo_root, ".cortex-ignore");

        Self {
            excluded_dirs: EXCLUDED_DIRS.iter().map(|s| s.to_string()).collect(),
            gitignore_patterns,
            cortex_ignore_patterns,
        }
    }

    /// Determine if a path should be included (i.e., not filtered out).
    ///
    /// Returns `true` if the path should produce an event, `false` if it
    /// should be excluded.
    pub fn should_include(&self, path: &Path, repo_root: &Path) -> bool {
        // Get relative path from repo root
        let rel_path = match path.strip_prefix(repo_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => return false,
        };

        // Check if any path component is an excluded directory
        for component in rel_path.split('/') {
            if self.excluded_dirs.iter().any(|d| d == component) {
                return false;
            }
        }

        // Check gitignore patterns
        if matches_any_pattern(&rel_path, &self.gitignore_patterns) {
            return false;
        }

        // Check cortex-ignore patterns
        if matches_any_pattern(&rel_path, &self.cortex_ignore_patterns) {
            return false;
        }

        true
    }
}

/// Load ignore patterns from a file (one pattern per line).
fn load_ignore_patterns(repo_root: &Path, filename: &str) -> Vec<String> {
    let path = repo_root.join(filename);
    match fs::read_to_string(&path) {
        Ok(content) => content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Basic pattern matching for gitignore-style patterns.
///
/// Supports:
/// - Exact filename matches (e.g., "file.txt")
/// - Directory prefix matches (e.g., "build/")
/// - Wildcard extension matches (e.g., "*.log")
/// - Path prefix matches (e.g., "dist/")
fn matches_any_pattern(rel_path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if pattern.ends_with('/') {
            // Directory pattern: matches if path starts with this prefix
            let dir_prefix = &pattern[..pattern.len() - 1];
            if rel_path.starts_with(dir_prefix) || rel_path.contains(&format!("/{}/", dir_prefix))
            {
                return true;
            }
        } else if pattern.starts_with("*.") {
            // Wildcard extension pattern
            let ext = &pattern[1..]; // e.g., ".log"
            if rel_path.ends_with(ext) {
                return true;
            }
        } else if pattern.contains('/') {
            // Path pattern
            if rel_path.starts_with(pattern) || rel_path == *pattern {
                return true;
            }
        } else {
            // Simple filename or directory name match
            let segments: Vec<&str> = rel_path.split('/').collect();
            if segments.iter().any(|s| *s == pattern) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_excluded_dirs_filtered() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();
        let filter = WatchFilter::new(repo_root);

        // Files in excluded directories should be filtered
        assert!(!filter.should_include(&repo_root.join(".git/config"), repo_root));
        assert!(!filter.should_include(&repo_root.join("node_modules/pkg/index.js"), repo_root));
        assert!(!filter.should_include(&repo_root.join("target/debug/binary"), repo_root));
        assert!(!filter.should_include(&repo_root.join("__pycache__/mod.pyc"), repo_root));
        assert!(!filter.should_include(&repo_root.join(".venv/lib/site.py"), repo_root));
        assert!(!filter.should_include(&repo_root.join(".cortex/db.sqlite"), repo_root));

        // Normal files should be included
        assert!(filter.should_include(&repo_root.join("src/main.rs"), repo_root));
        assert!(filter.should_include(&repo_root.join("lib/utils.py"), repo_root));
    }

    #[test]
    fn test_gitignore_patterns_loaded() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Create a .gitignore
        fs::write(repo_root.join(".gitignore"), "*.log\nbuild/\n").unwrap();

        let filter = WatchFilter::new(repo_root);

        assert!(!filter.should_include(&repo_root.join("app.log"), repo_root));
        assert!(!filter.should_include(&repo_root.join("build/output.js"), repo_root));
        assert!(filter.should_include(&repo_root.join("src/main.rs"), repo_root));
    }

    #[test]
    fn test_cortex_ignore_patterns_loaded() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        // Create a .cortex-ignore
        fs::write(repo_root.join(".cortex-ignore"), "*.generated.ts\nvendor/\n").unwrap();

        let filter = WatchFilter::new(repo_root);

        assert!(!filter.should_include(&repo_root.join("api.generated.ts"), repo_root));
        assert!(!filter.should_include(&repo_root.join("vendor/lib.js"), repo_root));
        assert!(filter.should_include(&repo_root.join("src/app.ts"), repo_root));
    }

    #[test]
    fn test_pattern_matching() {
        // Wildcard extension
        assert!(matches_any_pattern("src/file.log", &["*.log".to_string()]));
        assert!(!matches_any_pattern("src/file.py", &["*.log".to_string()]));

        // Directory pattern
        assert!(matches_any_pattern("build/output.js", &["build/".to_string()]));
        assert!(!matches_any_pattern("src/build.py", &["build/".to_string()]));

        // Simple name match
        assert!(matches_any_pattern("src/temp/file.py", &["temp".to_string()]));

        // Path pattern
        assert!(matches_any_pattern("dist/bundle.js", &["dist/bundle.js".to_string()]));
    }
}
