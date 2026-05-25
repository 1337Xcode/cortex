//! Release pipeline utilities.
//!
//! Contains functions that replicate release workflow logic in Rust for testing
//! and potential reuse. Currently covers version sync to package.json.

/// Update the `"version"` field in a package.json string to the given version.
///
/// This replicates the Node.js one-liner used in the release workflow:
/// ```js
/// const pkg = require('./package.json');
/// pkg.version = '<version>';
/// require('fs').writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
/// ```
///
/// Returns the updated JSON string (pretty-printed with 2-space indent, trailing newline).
/// Returns an error if the input is not valid JSON or not a JSON object.
pub fn sync_version_to_package_json(package_json: &str, version: &str) -> Result<String, String> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(package_json).map_err(|e| format!("invalid JSON: {e}"))?;

    let obj = parsed
        .as_object_mut()
        .ok_or_else(|| "package.json root is not an object".to_string())?;

    obj.insert(
        "version".to_string(),
        serde_json::Value::String(version.to_string()),
    );

    let output =
        serde_json::to_string_pretty(&parsed).map_err(|e| format!("serialization failed: {e}"))?;

    Ok(format!("{}\n", output))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_version_basic() {
        let input = r#"{
  "name": "@1337xcode/cortex",
  "version": "0.0.0"
}"#;
        let result = sync_version_to_package_json(input, "1.2.3").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["version"], "1.2.3");
    }

    #[test]
    fn test_sync_version_preserves_other_fields() {
        let input = r#"{
  "name": "test-pkg",
  "version": "0.0.0",
  "description": "A test package"
}"#;
        let result = sync_version_to_package_json(input, "2.0.0").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["name"], "test-pkg");
        assert_eq!(parsed["version"], "2.0.0");
        assert_eq!(parsed["description"], "A test package");
    }

    #[test]
    fn test_sync_version_adds_version_field() {
        let input = r#"{"name": "no-version"}"#;
        let result = sync_version_to_package_json(input, "3.0.0").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["version"], "3.0.0");
    }

    #[test]
    fn test_sync_version_invalid_json() {
        let result = sync_version_to_package_json("not json at all", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_version_non_object() {
        let result = sync_version_to_package_json("[1, 2, 3]", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_version_trailing_newline() {
        let input = r#"{"name": "test"}"#;
        let result = sync_version_to_package_json(input, "1.0.0").unwrap();
        assert!(result.ends_with('\n'));
    }
}

/// Property-based tests for version sync to package.json.
///
/// **Validates: Requirements 2.5**
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Strategy to generate a semver component (0-999).
    fn semver_component() -> impl Strategy<Value = u32> {
        0u32..1000
    }

    /// Strategy to generate a valid semver string "X.Y.Z".
    fn arb_semver() -> impl Strategy<Value = String> {
        (semver_component(), semver_component(), semver_component())
            .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
    }

    /// Strategy to generate a minimal but valid package.json string.
    /// Includes a "name" field and an existing "version" field with a random value.
    fn arb_package_json() -> impl Strategy<Value = String> {
        (
            "[a-z][a-z0-9-]{0,20}", // package name
            arb_semver(),           // existing version
        )
            .prop_map(|(name, existing_version)| {
                serde_json::json!({
                    "name": name,
                    "version": existing_version,
                    "description": "A test package"
                })
                .to_string()
            })
    }

    proptest! {
        /// **Property 15: Version sync to package.json**
        ///
        /// For any valid semver from a git tag `vX.Y.Z`, the version sync produces
        /// a package.json with `"version": "X.Y.Z"`.
        ///
        /// **Validates: Requirements 2.5**
        #[test]
        fn version_sync_produces_correct_version(
            package_json in arb_package_json(),
            version in arb_semver(),
        ) {
            // Simulate extracting version from tag (strip 'v' prefix)
            let tag = format!("v{}", version);
            let extracted_version = tag.trim_start_matches('v');

            // Run the version sync
            let result = sync_version_to_package_json(&package_json, extracted_version)
                .expect("sync_version_to_package_json should succeed for valid inputs");

            // Parse the result back
            let parsed: serde_json::Value = serde_json::from_str(&result)
                .expect("result should be valid JSON");

            // Verify the version field matches
            let result_version = parsed
                .get("version")
                .and_then(|v| v.as_str())
                .expect("result should have a string 'version' field");

            prop_assert_eq!(
                result_version,
                extracted_version,
                "Expected version '{}' but got '{}' in output: {}",
                extracted_version,
                result_version,
                result
            );
        }
    }
}
