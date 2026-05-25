//! Self-update command for Cortex.
//!
//! Downloads and installs the latest release from GitHub, verifies the SHA-256
//! checksum, replaces the binary, and triggers a reindex.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use sha2::{Digest, Sha256};

use crate::version::VERSION;

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

/// A GitHub release response (subset of fields we need).
#[derive(Debug, serde::Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub assets: Vec<ReleaseAsset>,
}

/// A single asset attached to a GitHub release.
#[derive(Debug, serde::Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

// ---------------------------------------------------------------------------
// Backward-compatible types (kept for existing callers)
// ---------------------------------------------------------------------------

/// Result of an update check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub message: String,
}

/// Check for updates (stub implementation, kept for backward compatibility).
///
/// When the `update-check` feature is not enabled, this returns
/// the current version without making network requests.
pub fn check_for_updates(enabled: bool) -> UpdateCheckResult {
    if !enabled {
        return UpdateCheckResult {
            current_version: VERSION.to_string(),
            latest_version: None,
            update_available: false,
            message: "Update check is disabled.".to_string(),
        };
    }

    UpdateCheckResult {
        current_version: VERSION.to_string(),
        latest_version: None,
        update_available: false,
        message: format!(
            "cortex {} (update check: network request not available in this build)",
            VERSION
        ),
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// GitHub releases API endpoint for the latest release.
const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/1337Xcode/cortex/releases/latest";

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the self-update flow: check version, download, verify, replace, reindex.
pub async fn run_update() -> Result<(), anyhow::Error> {
    let current = env!("CARGO_PKG_VERSION");

    let release = fetch_latest_release().await?;
    let latest = release.tag_name.trim_start_matches('v');

    if !is_newer(latest, current) {
        println!("cortex {} is already up to date.", current);
        return Ok(());
    }

    let archive_name = platform_archive_name()?;
    let archive_bytes = download_asset(&release, &archive_name).await?;
    let expected_hash =
        download_asset_text(&release, &format!("{}.sha256", archive_name)).await?;

    verify_sha256(&archive_bytes, &expected_hash, &archive_name)?;

    let bin_dir = home_dir()?.join(".cortex").join("bin");
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("Failed to create directory: {}", bin_dir.display()))?;

    replace_binary(&bin_dir, &archive_bytes)?;

    // Trigger reindex with the new binary
    let binary = bin_dir.join(binary_name());
    let status = std::process::Command::new(&binary)
        .arg("reindex")
        .status();

    if let Err(e) = status {
        eprintln!("Warning: failed to run reindex after update: {}", e);
    }

    println!("Updated cortex: {} \u{2192} {}", current, latest);
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Fetch the latest release metadata from GitHub.
async fn fetch_latest_release() -> Result<GitHubRelease, anyhow::Error> {
    let client = reqwest::Client::builder()
        .user_agent(format!("cortex/{}", VERSION))
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .get(GITHUB_RELEASES_URL)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Network error: could not reach GitHub releases. {}", e))?;

    if !response.status().is_success() {
        bail!(
            "GitHub API returned status {}: could not fetch latest release.",
            response.status()
        );
    }

    let release: GitHubRelease = response
        .json()
        .await
        .context("Failed to parse GitHub release JSON")?;

    Ok(release)
}

/// Compare two semver strings. Returns true if `latest` is newer than `current`.
///
/// Strips leading 'v' and compares as (major, minor, patch) tuples.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let v = v.trim_start_matches('v');
        let parts: Vec<u64> = v.split('.').filter_map(|s| s.parse().ok()).collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };
    parse(latest) > parse(current)
}

/// Determine the platform-specific archive name for the current OS/arch.
fn platform_archive_name() -> Result<String, anyhow::Error> {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => "cortex-darwin-x64.tar.gz",
        ("macos", "aarch64") => "cortex-darwin-arm64.tar.gz",
        ("linux", "x86_64") => "cortex-linux-x64.tar.gz",
        ("linux", "aarch64") => "cortex-linux-arm64.tar.gz",
        ("windows", "x86_64") => "cortex-win32-x64.tar.gz",
        ("windows", "x86") => "cortex-win32-ia32.tar.gz",
        (os, arch) => bail!("Unsupported platform: {}-{}", os, arch),
    };
    Ok(name.to_string())
}

/// Download a binary asset from the release by name.
async fn download_asset(
    release: &GitHubRelease,
    asset_name: &str,
) -> Result<Vec<u8>, anyhow::Error> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .with_context(|| {
            format!(
                "Release {} does not contain asset '{}'",
                release.tag_name, asset_name
            )
        })?;

    let client = reqwest::Client::builder()
        .user_agent(format!("cortex/{}", VERSION))
        .build()
        .context("Failed to build HTTP client")?;

    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download {}: {}", asset_name, e))?
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body for {}: {}", asset_name, e))?;

    Ok(bytes.to_vec())
}

/// Download a text asset (e.g. `.sha256` file) from the release by name.
async fn download_asset_text(
    release: &GitHubRelease,
    asset_name: &str,
) -> Result<String, anyhow::Error> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .with_context(|| {
            format!(
                "Release {} does not contain asset '{}'",
                release.tag_name, asset_name
            )
        })?;

    let client = reqwest::Client::builder()
        .user_agent(format!("cortex/{}", VERSION))
        .build()
        .context("Failed to build HTTP client")?;

    let text = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download {}: {}", asset_name, e))?
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response text for {}: {}", asset_name, e))?;

    Ok(text)
}

/// Verify the SHA-256 checksum of downloaded bytes against the expected hash.
///
/// The expected hash string may be in the format `<hash>  <filename>` or just `<hash>`.
/// Aborts with an error if the checksum does not match.
pub fn verify_sha256(
    data: &[u8],
    expected_hash_line: &str,
    archive_name: &str,
) -> Result<(), anyhow::Error> {
    // The .sha256 file may contain "<hash>  <filename>" or just "<hash>"
    let expected = expected_hash_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        bail!(
            "Checksum mismatch for '{}' \u{2014} possible tampering.\n  Expected: {}\n  Got:      {}",
            archive_name,
            expected,
            actual
        );
    }

    Ok(())
}

/// Get the user's home directory.
fn home_dir() -> Result<PathBuf, anyhow::Error> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .context("Cannot determine home directory (USERPROFILE/HOME not set)")
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .context("Cannot determine home directory (HOME not set)")
    }
}

/// Get the binary filename for the current platform.
fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "cortex.exe"
    } else {
        "cortex"
    }
}

// ---------------------------------------------------------------------------
// Binary replacement (platform-specific)
// ---------------------------------------------------------------------------

/// Replace the binary on Windows using the rename-then-replace pattern.
///
/// Windows cannot overwrite a running executable, so we rename the current
/// binary to `.old`, extract the new one, then clean up.
#[cfg(target_os = "windows")]
fn replace_binary(bin_dir: &Path, archive_bytes: &[u8]) -> Result<(), anyhow::Error> {
    let exe_path = bin_dir.join("cortex.exe");
    let old_path = bin_dir.join("cortex.old.exe");

    // Rename running binary out of the way
    if exe_path.exists() {
        let _ = fs::remove_file(&old_path); // clean up previous .old if present
        fs::rename(&exe_path, &old_path)
            .context("Failed to rename current binary to cortex.old.exe")?;
    }

    extract_tar_gz(archive_bytes, bin_dir)?;

    // Clean up old binary (best-effort, may fail if still in use)
    let _ = fs::remove_file(&old_path);
    Ok(())
}

/// Replace the binary on Unix (simple overwrite via extraction).
#[cfg(not(target_os = "windows"))]
fn replace_binary(bin_dir: &Path, archive_bytes: &[u8]) -> Result<(), anyhow::Error> {
    extract_tar_gz(archive_bytes, bin_dir)?;

    // Ensure the binary is executable
    let exe_path = bin_dir.join("cortex");
    if exe_path.exists() {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&exe_path, perms)
            .context("Failed to set executable permissions on cortex binary")?;
    }

    Ok(())
}

/// Extract a tar.gz archive into the target directory using the system `tar` command.
///
/// This matches the approach used by the npm installer script.
fn extract_tar_gz(archive_bytes: &[u8], target_dir: &Path) -> Result<(), anyhow::Error> {
    let tmp_archive = target_dir.join(".cortex-update.tar.gz");
    fs::write(&tmp_archive, archive_bytes)
        .with_context(|| format!("Failed to write temp archive to {}", tmp_archive.display()))?;

    let status = std::process::Command::new("tar")
        .args(["xzf", &tmp_archive.to_string_lossy(), "-C", &target_dir.to_string_lossy()])
        .status()
        .context("Failed to execute tar command. Is tar available on PATH?")?;

    // Clean up temp archive
    let _ = fs::remove_file(&tmp_archive);

    if !status.success() {
        bail!(
            "tar extraction failed with exit code: {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting helper (public for property tests)
// ---------------------------------------------------------------------------

/// Format the update success message.
pub fn format_update_message(old_version: &str, new_version: &str) -> String {
    format!("Updated cortex: {} \u{2192} {}", old_version, new_version)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_check_disabled() {
        let result = check_for_updates(false);
        assert!(!result.update_available);
        assert!(result.message.contains("disabled"));
        assert_eq!(result.current_version, VERSION);
    }

    #[test]
    fn test_update_check_enabled_stub() {
        let result = check_for_updates(true);
        assert!(!result.update_available);
        assert!(result.latest_version.is_none());
    }

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("2.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_equal() {
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }

    #[test]
    fn test_is_newer_with_v_prefix() {
        assert!(is_newer("v1.0.1", "v1.0.0"));
        assert!(is_newer("v2.0.0", "1.0.0"));
    }

    #[test]
    fn test_platform_archive_name_format() {
        // Just verify it returns Ok on the current platform (whatever it is)
        let result = platform_archive_name();
        assert!(result.is_ok());
        let name = result.unwrap();
        assert!(name.starts_with("cortex-"));
        assert!(name.ends_with(".tar.gz"));
    }

    #[test]
    fn test_verify_sha256_valid() {
        let data = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = format!("{:x}", hasher.finalize());

        assert!(verify_sha256(data, &hash, "test.tar.gz").is_ok());
    }

    #[test]
    fn test_verify_sha256_with_filename_suffix() {
        let data = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = format!("{:x}", hasher.finalize());

        // Format: "<hash>  <filename>"
        let hash_line = format!("{}  test.tar.gz", hash);
        assert!(verify_sha256(data, &hash_line, "test.tar.gz").is_ok());
    }

    #[test]
    fn test_verify_sha256_mismatch() {
        let data = b"hello world";
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let result = verify_sha256(data, wrong_hash, "test.tar.gz");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Checksum mismatch"));
        assert!(err_msg.contains("possible tampering"));
    }

    #[test]
    fn test_format_update_message() {
        let msg = format_update_message("1.0.2", "1.0.3");
        assert_eq!(msg, "Updated cortex: 1.0.2 \u{2192} 1.0.3");
    }

    #[test]
    fn test_format_update_message_contains_both_versions() {
        let msg = format_update_message("0.9.0", "2.0.0");
        assert!(msg.contains("0.9.0"));
        assert!(msg.contains("2.0.0"));
        assert!(msg.contains("\u{2192}"));
    }

    #[test]
    fn test_binary_name() {
        let name = binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(name, "cortex.exe");
        } else {
            assert_eq!(name, "cortex");
        }
    }
}


// ---------------------------------------------------------------------------
// Property-based tests
// ---------------------------------------------------------------------------

/// Property-based tests for semver comparison and SHA-256 verification.
///
/// **Validates: Requirements 5.2, 5.3, 5.4, 9.4**
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use sha2::{Digest, Sha256};

    fn semver_component() -> impl Strategy<Value = u64> {
        prop_oneof![
            3 => 0u64..100,
            1 => 100u64..10000,
        ]
    }

    fn semver_tuple() -> impl Strategy<Value = (u64, u64, u64)> {
        (semver_component(), semver_component(), semver_component())
    }

    proptest! {
        /// Property 5: Semver comparison
        ///
        /// For any two valid semver version strings (major.minor.patch), the `is_newer`
        /// function SHALL return `true` if and only if the first version is strictly
        /// greater than the second when compared as tuples `(major, minor, patch)`.
        ///
        /// **Validates: Requirements 5.2, 9.4**
        #[test]
        fn prop_semver_comparison(
            latest in semver_tuple(),
            current in semver_tuple(),
        ) {
            let latest_str = format!("{}.{}.{}", latest.0, latest.1, latest.2);
            let current_str = format!("{}.{}.{}", current.0, current.1, current.2);

            let result = is_newer(&latest_str, &current_str);
            let expected = latest > current;

            prop_assert_eq!(result, expected,
                "is_newer({:?}, {:?}) = {} but expected {}",
                latest_str, current_str, result, expected
            );
        }

        /// Property 6: SHA-256 verification round-trip
        ///
        /// For any byte buffer, computing its SHA-256 hash and then calling the
        /// verification function with that hash SHALL succeed.
        ///
        /// **Validates: Requirements 5.3, 5.4**
        #[test]
        fn prop_sha256_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
            // Compute correct hash
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let correct_hash = format!("{:x}", hasher.finalize());

            // Verification with correct hash should succeed
            prop_assert!(verify_sha256(&data, &correct_hash, "test.tar.gz").is_ok());

            // Verification with correct hash in "<hash>  <filename>" format should succeed
            let hash_with_filename = format!("{}  test.tar.gz", correct_hash);
            prop_assert!(verify_sha256(&data, &hash_with_filename, "test.tar.gz").is_ok());
        }

        /// Property 6b: SHA-256 verification with wrong hash should fail
        ///
        /// For any byte buffer, calling the verification function with a different
        /// hash SHALL fail.
        ///
        /// **Validates: Requirements 5.3, 5.4**
        #[test]
        fn prop_sha256_wrong_hash_fails(
            data in proptest::collection::vec(any::<u8>(), 1..4096),
            _wrong_byte in any::<u8>(),
        ) {
            // Compute correct hash
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let correct_hash = format!("{:x}", hasher.finalize());

            // Create a different hash by flipping the first character
            let mut wrong_hash = correct_hash.clone();
            if let Some(c) = wrong_hash.chars().next() {
                let replacement = if c == '0' { '1' } else { '0' };
                wrong_hash = format!("{}{}", replacement, &correct_hash[1..]);
            }

            // Only test if the hashes are actually different
            if wrong_hash != correct_hash {
                prop_assert!(verify_sha256(&data, &wrong_hash, "test.tar.gz").is_err());
            }
        }

        /// Property 8: Update message formatting
        ///
        /// For any pair of version strings (old, new) where new is strictly greater
        /// than old, the update completion message SHALL contain both version strings
        /// and match the format `Updated cortex: {old} → {new}`.
        ///
        /// **Validates: Requirements 5.7**
        #[test]
        fn prop_update_message_format(
            old_major in 0u32..100,
            old_minor in 0u32..100,
            old_patch in 0u32..100,
            new_major in 0u32..100,
            new_minor in 0u32..100,
            new_patch in 0u32..100,
        ) {
            let old = format!("{}.{}.{}", old_major, old_minor, old_patch);
            let new_ver = format!("{}.{}.{}", new_major, new_minor, new_patch);

            let msg = format_update_message(&old, &new_ver);

            // Message must contain both versions
            prop_assert!(msg.contains(&old), "Message missing old version: {}", msg);
            prop_assert!(msg.contains(&new_ver), "Message missing new version: {}", msg);

            // Message must contain the arrow separator
            prop_assert!(msg.contains("\u{2192}"), "Message missing arrow: {}", msg);

            // Message must start with "Updated cortex:"
            prop_assert!(msg.starts_with("Updated cortex:"), "Wrong prefix: {}", msg);
        }
    }
}
