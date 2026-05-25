//! Release notes extraction from CHANGELOG.md.
//!
//! Implements the same logic as the shell script in `.github/workflows/release.yml`:
//! given a CHANGELOG.md and a target version, extract the content between the
//! `## [X.Y.Z]` header and the next `## [` header (or end of file).

/// Extract release notes for a specific version from changelog content.
///
/// Looks for a line starting with `## [<version>]` and returns all content
/// between that header and the next `## [` header (or EOF). The matching
/// header line itself is excluded from the output.
///
/// Returns `None` if no matching version header is found.
pub fn extract_release_notes(changelog: &str, version: &str) -> Option<String> {
    let target_header = format!("## [{}]", version);

    let mut lines = changelog.lines();
    let mut found = false;
    let mut result_lines: Vec<&str> = Vec::new();

    for line in lines.by_ref() {
        if found {
            // Check if we hit the next version header
            if line.starts_with("## [") {
                break;
            }
            result_lines.push(line);
        } else if line.starts_with(&target_header) {
            found = true;
        }
    }

    if found {
        Some(result_lines.join("\n"))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ─── Unit Tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_extract_basic() {
        let changelog = "\
# Changelog

## [1.0.1] - 2026-05-19

### Added
- Feature A

## [1.0.0] - 2026-05-18

### Added
- Initial release
";
        let result = extract_release_notes(changelog, "1.0.1").unwrap();
        assert!(result.contains("### Added"));
        assert!(result.contains("- Feature A"));
        assert!(!result.contains("## [1.0.0]"));
        assert!(!result.contains("- Initial release"));
    }

    #[test]
    fn test_extract_last_section_to_eof() {
        let changelog = "\
# Changelog

## [1.0.0] - 2026-05-18

### Added
- Initial release
";
        let result = extract_release_notes(changelog, "1.0.0").unwrap();
        assert!(result.contains("### Added"));
        assert!(result.contains("- Initial release"));
    }

    #[test]
    fn test_extract_missing_version() {
        let changelog = "\
# Changelog

## [1.0.0] - 2026-05-18

### Added
- Initial release
";
        assert!(extract_release_notes(changelog, "2.0.0").is_none());
    }

    #[test]
    fn test_extract_empty_section() {
        let changelog = "\
## [2.0.0] - 2026-06-01

## [1.0.0] - 2026-05-18

### Added
- Initial release
";
        let result = extract_release_notes(changelog, "2.0.0").unwrap();
        // The section between 2.0.0 and 1.0.0 headers is just an empty line
        assert!(!result.contains("### Added"));
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    /// Strategy to generate a valid semver version string (X.Y.Z).
    fn arb_version() -> impl Strategy<Value = String> {
        (0u32..100, 0u32..100, 0u32..100)
            .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
    }

    /// Strategy to generate a non-empty section body (lines of text that don't
    /// start with `## [`).
    fn arb_section_body() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            "[a-zA-Z0-9 \\-\\*\\.,:;!?#/]{1,80}"
                .prop_filter("must not start with ## [", |s| !s.starts_with("## [")),
            1..=10,
        )
        .prop_map(|lines| lines.join("\n"))
    }

    /// Strategy to generate a valid CHANGELOG.md with multiple version sections.
    /// Returns (changelog_content, target_version, expected_body).
    fn arb_changelog_with_target() -> impl Strategy<Value = (String, String, String)> {
        // Generate 2-5 version sections
        let sections = proptest::collection::vec((arb_version(), arb_section_body()), 2..=5);

        sections
            .prop_flat_map(|secs| {
                // Pick a random index to be the target
                let len = secs.len();
                (Just(secs), 0..len)
            })
            .prop_map(|(sections, target_idx)| {
                let target_version = sections[target_idx].0.clone();
                let expected_body = sections[target_idx].1.clone();

                // Build the changelog
                let mut changelog = String::from("# Changelog\n\n");
                for (version, body) in &sections {
                    changelog.push_str(&format!("## [{}] - 2026-01-01\n\n", version));
                    changelog.push_str(body);
                    changelog.push_str("\n\n");
                }

                (changelog, target_version, expected_body)
            })
    }

    // **Validates: Requirements 2.3**
    proptest! {
        /// Property 14: Release notes extraction correctness
        ///
        /// For any valid CHANGELOG.md containing `## [X.Y.Z]`, extraction returns
        /// content between that header and the next `## [` header or EOF.
        #[test]
        fn prop_release_notes_extraction_correctness(
            (changelog, target_version, expected_body) in arb_changelog_with_target()
        ) {
            let result = extract_release_notes(&changelog, &target_version);

            // The extraction must succeed since we know the version is present
            prop_assert!(result.is_some(), "Expected Some for version {} in changelog", target_version);

            let extracted = result.unwrap();

            // The extracted content must contain the expected body
            prop_assert!(
                extracted.contains(&expected_body),
                "Extracted content does not contain expected body.\nExtracted: {:?}\nExpected to contain: {:?}",
                extracted,
                expected_body
            );

            // The extracted content must NOT contain any `## [` header line
            for line in extracted.lines() {
                prop_assert!(
                    !line.starts_with("## ["),
                    "Extracted content contains a version header: {:?}",
                    line
                );
            }

            // The extracted content must NOT contain the target header itself
            let target_header = format!("## [{}]", target_version);
            prop_assert!(
                !extracted.contains(&target_header),
                "Extracted content contains the target header"
            );
        }

        /// Property 14b: Extraction returns None for versions not in the changelog.
        #[test]
        fn prop_extraction_returns_none_for_missing_version(
            (changelog, _, _) in arb_changelog_with_target(),
            missing_version in arb_version()
        ) {
            // Only test if the missing_version is actually not in the changelog
            let header = format!("## [{}]", missing_version);
            if !changelog.contains(&header) {
                let result = extract_release_notes(&changelog, &missing_version);
                prop_assert!(
                    result.is_none(),
                    "Expected None for missing version {}, got Some",
                    missing_version
                );
            }
        }
    }
}
