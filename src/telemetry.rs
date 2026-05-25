//! Structured logging via tracing.
//!
//! Initializes a tracing subscriber with env-filter for log level control.
//! Never logs file contents, observation text, or source code.
//!
//! Also provides UTF-8 sanitization utilities for ensuring all tracing output
//! contains only valid UTF-8 sequences.

use std::path::Path;

use tracing_subscriber::EnvFilter;

/// Initialize the tracing subscriber with the given log level filter.
///
/// When `stderr_output` is true (e.g. during `serve`), all logs are written
/// to stderr so that stdout remains clean for JSON-RPC communication.
pub fn init_tracing(log_level: &str, stderr_output: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    if stderr_output {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .init();
    }
}

/// Sanitize a byte slice to valid UTF-8, replacing invalid sequences with U+FFFD.
///
/// Returns the sanitized string and the count of replacements made.
/// Uses `String::from_utf8_lossy` which replaces each maximal subpart of an
/// ill-formed subsequence with a single U+FFFD replacement character.
///
/// The replacement count is determined by comparing the lossy output against
/// the original bytes: each U+FFFD in the output that does not correspond to
/// a valid U+FFFD encoding in the original bytes is counted as a replacement.
pub fn sanitize_utf8(bytes: &[u8]) -> (String, usize) {
    let lossy = String::from_utf8_lossy(bytes);

    // Fast path: if from_utf8_lossy returned a Borrowed variant, the input was valid UTF-8
    // and no replacements were made.
    if matches!(lossy, std::borrow::Cow::Borrowed(_)) {
        return (lossy.into_owned(), 0);
    }

    // Count replacements: each U+FFFD in the output that wasn't a valid U+FFFD
    // sequence (EF BF BD) in the original bytes represents a replacement.
    let result = lossy.into_owned();
    let valid_replacement_count = count_valid_replacement_chars(bytes);
    let total_replacement_chars = result.chars().filter(|&c| c == '\u{FFFD}').count();
    let replacements = total_replacement_chars.saturating_sub(valid_replacement_count);

    if replacements > 0 {
        tracing::debug!(
            original_byte_length = bytes.len(),
            replacement_count = replacements,
            "UTF-8 sanitization replaced invalid byte sequences"
        );
    }

    (result, replacements)
}

/// Count the number of valid U+FFFD (EF BF BD) sequences in the raw bytes.
/// These are legitimate replacement characters in the original data, not
/// artifacts of sanitization.
fn count_valid_replacement_chars(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0xEF && bytes[i + 1] == 0xBF && bytes[i + 2] == 0xBD {
            count += 1;
            i += 3;
        } else {
            i += 1;
        }
    }
    count
}

/// Sanitize an OS path to a valid UTF-8 string for use in tracing output.
///
/// On Unix, paths may contain arbitrary bytes that are not valid UTF-8.
/// On Windows, paths use UTF-16 internally but can still produce lossy
/// conversions. This function ensures the resulting string is always valid UTF-8,
/// replacing any invalid sequences with U+FFFD.
///
/// Logs a debug message if any replacements were made.
pub fn sanitize_path(path: &Path) -> String {
    // Path::to_string_lossy already handles OS-specific encoding issues,
    // replacing invalid sequences with U+FFFD.
    let lossy = path.to_string_lossy();

    // Check if any replacement occurred by seeing if we got a Borrowed or Owned variant
    if matches!(lossy, std::borrow::Cow::Borrowed(_)) {
        return lossy.into_owned();
    }

    // Replacements occurred
    let result = lossy.into_owned();
    let replacement_count = result.chars().filter(|&c| c == '\u{FFFD}').count();

    if replacement_count > 0 {
        tracing::debug!(
            path_byte_length = path.as_os_str().len(),
            replacement_count = replacement_count,
            "Path sanitization replaced invalid sequences"
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_sanitize_utf8_valid_input() {
        let input = b"Hello, world!";
        let (result, replacements) = sanitize_utf8(input);
        assert_eq!(result, "Hello, world!");
        assert_eq!(replacements, 0);
    }

    #[test]
    fn test_sanitize_utf8_with_invalid_bytes() {
        // 0xFF is not valid in any UTF-8 sequence
        let input = b"Hello \xFF world";
        let (result, replacements) = sanitize_utf8(input);
        assert_eq!(result, "Hello \u{FFFD} world");
        assert_eq!(replacements, 1);
        // Result must be valid UTF-8
        assert!(String::from_utf8(result.into_bytes()).is_ok());
    }

    #[test]
    fn test_sanitize_utf8_multiple_invalid_sequences() {
        let input = b"\xC0\xC1 test \xFE\xFF";
        let (result, replacements) = sanitize_utf8(input);
        assert!(replacements >= 2);
        // Result must be valid UTF-8
        assert!(String::from_utf8(result.into_bytes()).is_ok());
    }

    #[test]
    fn test_sanitize_utf8_preserves_valid_replacement_char() {
        // Input contains a valid U+FFFD encoding (EF BF BD)
        let input = b"before \xEF\xBF\xBD after";
        let (result, replacements) = sanitize_utf8(input);
        assert!(result.contains('\u{FFFD}'));
        // The U+FFFD was already valid in the input, so no replacements
        assert_eq!(replacements, 0);
    }

    #[test]
    fn test_sanitize_utf8_empty_input() {
        let (result, replacements) = sanitize_utf8(b"");
        assert_eq!(result, "");
        assert_eq!(replacements, 0);
    }

    #[test]
    fn test_sanitize_utf8_all_invalid() {
        let input = b"\xFF\xFE\xFD";
        let (result, replacements) = sanitize_utf8(input);
        assert!(replacements > 0);
        // Result must be valid UTF-8
        assert!(String::from_utf8(result.into_bytes()).is_ok());
    }

    #[test]
    fn test_sanitize_utf8_valid_multibyte() {
        // Valid UTF-8 multibyte: "日本語" (Japanese)
        let input = "日本語".as_bytes();
        let (result, replacements) = sanitize_utf8(input);
        assert_eq!(result, "日本語");
        assert_eq!(replacements, 0);
    }

    #[test]
    fn test_sanitize_path_valid() {
        let path = Path::new("/home/user/project/src/main.rs");
        let result = sanitize_path(path);
        assert_eq!(result, "/home/user/project/src/main.rs");
    }

    #[test]
    fn test_sanitize_path_with_unicode() {
        let path = Path::new("/home/user/проект/src/main.rs");
        let result = sanitize_path(path);
        assert_eq!(result, "/home/user/проект/src/main.rs");
        // Result must be valid UTF-8
        assert!(String::from_utf8(result.into_bytes()).is_ok());
    }

    // Property-based tests using proptest

    proptest! {
        /// **Validates: Requirements 5.1**
        ///
        /// Property 11: For any arbitrary byte sequence, the `sanitize_utf8` function
        /// SHALL produce output that is valid UTF-8 (parseable by any UTF-8 decoder
        /// without errors). Invalid byte sequences in the input SHALL be replaced with
        /// U+FFFD in the output.
        #[test]
        fn prop_sanitize_utf8_produces_valid_utf8(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let (result, replacements) = sanitize_utf8(&bytes);

            // The result must always be valid UTF-8
            prop_assert!(
                String::from_utf8(result.clone().into_bytes()).is_ok(),
                "sanitize_utf8 produced invalid UTF-8 for input of length {}",
                bytes.len()
            );

            // If the input was already valid UTF-8, the output must match the input exactly
            if let Ok(valid_input) = String::from_utf8(bytes.clone()) {
                prop_assert_eq!(
                    &result, &valid_input,
                    "sanitize_utf8 altered valid UTF-8 input"
                );
                prop_assert_eq!(
                    replacements, 0,
                    "sanitize_utf8 reported replacements on valid UTF-8 input"
                );
            }

            // Replacement count is always >= 0 (usize is non-negative by definition)
            // This is guaranteed by the type system.
        }
    }
}
