//! Build version, format version constants, and async update checker.
//!
//! Used by CLI --version, bundle format_version, and startup update notifications.

use std::time::Duration;

/// The current version of the Cortex binary, sourced from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub releases API endpoint for the latest release.
const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/1337Xcode/cortex/releases/latest";

/// Timeout for the update check HTTP request.
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Compare two semver strings. Returns true if `remote` is newer than `local`.
///
/// Parses each string as (major, minor, patch) tuples, stripping any leading 'v'.
/// Returns false on parse failure.
pub fn is_newer(local: &str, remote: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let s = s.trim_start_matches('v');
        let parts: Vec<&str> = s.split('.').collect();
        let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(remote) > parse(local)
}

/// Check for updates by fetching the latest release tag from GitHub.
///
/// Returns `Some(latest_version)` if an update is available (remote > local).
/// Returns `None` on any error (network failure, timeout, parse failure, etc.).
pub async fn check_for_update() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent(format!("cortex/{}", VERSION))
        .build()
        .ok()?;

    let response = client.get(GITHUB_RELEASES_URL).send().await.ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body: serde_json::Value = response.json().await.ok()?;
    let tag_name = body.get("tag_name")?.as_str()?;

    // Strip leading 'v' for comparison
    let latest = tag_name.trim_start_matches('v').to_string();

    if is_newer(VERSION, &latest) {
        Some(latest)
    } else {
        None
    }
}

/// Format the update notification message for stderr.
pub fn update_notification(latest: &str) -> String {
    format!(
        "Update available: v{} -> v{}\nRun: npx @1337xcode/cortex install",
        VERSION, latest
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("1.0.0", "2.0.0"));
    }

    #[test]
    fn test_is_newer_equal() {
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn test_is_newer_with_v_prefix() {
        assert!(is_newer("v1.0.0", "v1.0.1"));
        assert!(is_newer("1.0.0", "v2.0.0"));
        assert!(is_newer("v1.0.0", "2.0.0"));
    }

    #[test]
    fn test_is_newer_partial_versions() {
        // Missing patch treated as 0
        assert!(is_newer("1.0", "1.0.1"));
        assert!(!is_newer("1.0.1", "1.0"));
    }

    #[test]
    fn test_is_newer_invalid_input() {
        // Invalid strings parse as (0, 0, 0)
        assert!(!is_newer("abc", "def"));
        assert!(is_newer("abc", "0.0.1"));
    }

    #[test]
    fn test_update_notification_format() {
        let msg = update_notification("1.1.0");
        assert!(msg.contains(&format!("v{}", VERSION)));
        assert!(msg.contains("v1.1.0"));
        assert!(msg.contains("npx @1337xcode/cortex install"));
    }

    #[test]
    fn test_update_notification_contains_both_versions() {
        let msg = update_notification("2.0.0");
        assert!(msg.starts_with("Update available:"));
        assert!(msg.contains("Run: npx @1337xcode/cortex install"));
    }
}

/// Property-based tests for version comparison.
///
/// **Validates: Requirements 3.3, 3.4**
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Strategy to generate semver component values (u64).
    /// We use a reasonable range to keep tests fast while covering edge cases.
    fn semver_component() -> impl Strategy<Value = u64> {
        prop_oneof![
            3 => 0u64..100,   // common small values
            1 => 100u64..10000, // larger values
        ]
    }

    /// Strategy to generate a semver tuple (major, minor, patch).
    fn semver_tuple() -> impl Strategy<Value = (u64, u64, u64)> {
        (semver_component(), semver_component(), semver_component())
    }

    /// Format a semver tuple as a version string, optionally with 'v' prefix.
    fn format_version(major: u64, minor: u64, patch: u64, with_v: bool) -> String {
        if with_v {
            format!("v{}.{}.{}", major, minor, patch)
        } else {
            format!("{}.{}.{}", major, minor, patch)
        }
    }

    proptest! {
        /// **Property 1: Version comparison correctness**
        ///
        /// For any two valid semver tuples, `is_newer(local, remote)` returns true
        /// if and only if (remote_major, remote_minor, remote_patch) is strictly
        /// greater than (local_major, local_minor, local_patch) in lexicographic
        /// tuple comparison.
        ///
        /// **Validates: Requirements 3.3, 3.4**
        #[test]
        fn version_comparison_correctness(
            local in semver_tuple(),
            remote in semver_tuple(),
            local_has_v in proptest::bool::ANY,
            remote_has_v in proptest::bool::ANY,
        ) {
            let (l_major, l_minor, l_patch) = local;
            let (r_major, r_minor, r_patch) = remote;

            let local_str = format_version(l_major, l_minor, l_patch, local_has_v);
            let remote_str = format_version(r_major, r_minor, r_patch, remote_has_v);

            let result = is_newer(&local_str, &remote_str);
            let expected = (r_major, r_minor, r_patch) > (l_major, l_minor, l_patch);

            prop_assert_eq!(
                result, expected,
                "is_newer({:?}, {:?}) = {} but expected {} (local tuple: {:?}, remote tuple: {:?})",
                local_str, remote_str, result, expected, local, remote
            );
        }
    }
}
