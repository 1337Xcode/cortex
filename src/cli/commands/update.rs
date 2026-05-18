//! Update check stub.
//!
//! Checks for new versions of Cortex against GitHub releases.
//! This is a stub implementation that does not make actual network requests
//! unless the `update-check` feature is enabled.

use crate::version::VERSION;

/// Result of an update check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub message: String,
}

/// Check for updates (stub implementation).
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

    // Stub: actual HTTP request to GitHub releases API would go here
    // Requires `reqwest` feature to be enabled
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
}
