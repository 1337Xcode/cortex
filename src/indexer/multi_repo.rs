//! Multi-repo indexing support.
//!
//! Provides FQN namespacing for additional repositories. When indexing
//! multiple repos, FQNs from non-primary repos are prefixed with the
//! repo name to avoid collisions.

use std::path::Path;

/// Namespace an FQN with a repo identifier.
///
/// For additional repos, the FQN is prefixed with `{repo_name}::`.
/// The primary repo's FQNs remain unchanged.
///
/// # Examples
///
/// ```
/// use cortex::indexer::multi_repo::namespace_fqn;
///
/// let fqn = namespace_fqn("backend", "src/main.rs::main");
/// assert_eq!(fqn, "backend::src/main.rs::main");
/// ```
pub fn namespace_fqn(repo_name: &str, fqn: &str) -> String {
    if repo_name.is_empty() {
        fqn.to_string()
    } else {
        format!("{}::{}", repo_name, fqn)
    }
}

/// Extract the repo name from a namespaced FQN.
///
/// Returns `None` if the FQN is not namespaced (belongs to primary repo).
pub fn extract_repo_name(fqn: &str) -> Option<&str> {
    // A namespaced FQN has the form: repo_name::file_path::symbol
    // We need to distinguish from regular FQNs like: src/main.rs::main
    // Repo names don't contain '/' or '.' characters
    if let Some(first_sep) = fqn.find("::") {
        let prefix = &fqn[..first_sep];
        // If the prefix doesn't contain path separators, it's a repo name
        if !prefix.contains('/') && !prefix.contains('.') && !prefix.contains('\\') {
            return Some(prefix);
        }
    }
    None
}

/// Derive a repo name from a path.
///
/// Uses the last component of the path as the repo identifier.
pub fn repo_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_namespace_fqn_with_repo_name() {
        let result = namespace_fqn("backend", "src/main.rs::main");
        assert_eq!(result, "backend::src/main.rs::main");
    }

    #[test]
    fn test_namespace_fqn_empty_repo_name() {
        let result = namespace_fqn("", "src/main.rs::main");
        assert_eq!(result, "src/main.rs::main");
    }

    #[test]
    fn test_namespace_fqn_preserves_nested_symbols() {
        let result = namespace_fqn("api", "src/handlers/auth.ts::AuthController::login");
        assert_eq!(result, "api::src/handlers/auth.ts::AuthController::login");
    }

    #[test]
    fn test_extract_repo_name_namespaced() {
        let repo = extract_repo_name("backend::src/main.rs::main");
        assert_eq!(repo, Some("backend"));
    }

    #[test]
    fn test_extract_repo_name_not_namespaced() {
        let repo = extract_repo_name("src/main.rs::main");
        assert_eq!(repo, None);
    }

    #[test]
    fn test_repo_name_from_path() {
        let path = PathBuf::from("/home/user/projects/backend");
        assert_eq!(repo_name_from_path(&path), "backend");
    }

    #[test]
    fn test_repo_name_from_path_with_trailing_slash() {
        let path = PathBuf::from("/home/user/projects/api-service");
        assert_eq!(repo_name_from_path(&path), "api-service");
    }
}
