//! Secret detection and redaction module.
//!
//! Provides regex-based pattern matching and Shannon entropy scoring
//! to detect secrets (API keys, passwords, tokens, connection strings)
//! in source code. Detected secrets are tagged on nodes during indexing
//! and redacted in code snippets at query time.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// The type/category of a detected secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretType {
    AwsAccessKey,
    GitHubToken,
    GenericPassword,
    ConnectionString,
    PrivateKey,
    HighEntropy,
}

impl SecretType {
    /// Returns the string label used in redaction placeholders and node attributes.
    pub fn label(&self) -> &'static str {
        match self {
            SecretType::AwsAccessKey => "aws_access_key",
            SecretType::GitHubToken => "github_token",
            SecretType::GenericPassword => "password",
            SecretType::ConnectionString => "connection_string",
            SecretType::PrivateKey => "private_key",
            SecretType::HighEntropy => "high_entropy_secret",
        }
    }
}

/// A detected secret match within source code.
#[derive(Debug, Clone)]
pub struct SecretMatch {
    /// The type of secret detected.
    pub secret_type: SecretType,
    /// 1-based line number where the secret was found.
    pub line: u32,
    /// Byte offset start within the source string.
    pub start: usize,
    /// Byte offset end within the source string.
    pub end: usize,
    /// The matched text (the secret value itself).
    pub matched_text: String,
}

// ---------------------------------------------------------------------------
// Regex patterns (compiled once via LazyLock)
// ---------------------------------------------------------------------------

static AWS_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());

static GITHUB_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(ghp_|gho_|ghs_|ghr_)[A-Za-z0-9_]{36,}").unwrap());

static GENERIC_PASSWORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(password|secret|token|api_key|apikey)\s*[=:]\s*["']([^"']{8,})["']"#)
        .unwrap()
});

static CONNECTION_STRING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(postgres|postgresql|mysql|mongodb|redis|amqp)://[^\s"'`]+"#).unwrap()
});

static PRIVATE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----BEGIN[A-Z ]*PRIVATE KEY-----").unwrap());

// ---------------------------------------------------------------------------
// Shannon entropy
// ---------------------------------------------------------------------------

/// Compute Shannon entropy in bits per character for a string.
///
/// Returns 0.0 for empty strings.
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }

    let len = s.len() as f64;
    let mut freq: HashMap<u8, usize> = HashMap::new();
    for &byte in s.as_bytes() {
        *freq.entry(byte).or_insert(0) += 1;
    }

    let mut entropy = 0.0_f64;
    for &count in freq.values() {
        let p = count as f64 / len;
        if p > 0.0 {
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Check if a string is suspicious based on Shannon entropy.
///
/// A string is flagged if it is longer than 20 characters and has
/// entropy greater than 4.5 bits/char.
pub fn is_high_entropy(s: &str) -> bool {
    s.len() > 20 && shannon_entropy(s) > 4.5
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Scan source code for secrets using regex patterns and entropy scoring.
///
/// Returns a list of `SecretMatch` values with type, line number, and byte offsets.
pub fn detect_secrets(source: &str) -> Vec<SecretMatch> {
    let mut matches: Vec<SecretMatch> = Vec::new();

    // Helper: compute 1-based line number from byte offset
    let line_of = |offset: usize| -> u32 {
        source[..offset].matches('\n').count() as u32 + 1
    };

    // AWS access keys
    for m in AWS_KEY_RE.find_iter(source) {
        matches.push(SecretMatch {
            secret_type: SecretType::AwsAccessKey,
            line: line_of(m.start()),
            start: m.start(),
            end: m.end(),
            matched_text: m.as_str().to_string(),
        });
    }

    // GitHub tokens
    for m in GITHUB_TOKEN_RE.find_iter(source) {
        matches.push(SecretMatch {
            secret_type: SecretType::GitHubToken,
            line: line_of(m.start()),
            start: m.start(),
            end: m.end(),
            matched_text: m.as_str().to_string(),
        });
    }

    // Generic passwords (capture group 2 is the value)
    for caps in GENERIC_PASSWORD_RE.captures_iter(source) {
        if caps.get(2).is_some() {
            // Use the full match for replacement range
            let full = caps.get(0).unwrap();
            matches.push(SecretMatch {
                secret_type: SecretType::GenericPassword,
                line: line_of(full.start()),
                start: full.start(),
                end: full.end(),
                matched_text: full.as_str().to_string(),
            });
        }
    }

    // Connection strings
    for m in CONNECTION_STRING_RE.find_iter(source) {
        matches.push(SecretMatch {
            secret_type: SecretType::ConnectionString,
            line: line_of(m.start()),
            start: m.start(),
            end: m.end(),
            matched_text: m.as_str().to_string(),
        });
    }

    // Private keys
    for m in PRIVATE_KEY_RE.find_iter(source) {
        matches.push(SecretMatch {
            secret_type: SecretType::PrivateKey,
            line: line_of(m.start()),
            start: m.start(),
            end: m.end(),
            matched_text: m.as_str().to_string(),
        });
    }

    // High-entropy strings: scan for quoted string literals
    let string_literal_re: Regex =
        Regex::new(r#"["']([^"']{21,})["']"#).unwrap();
    for caps in string_literal_re.captures_iter(source) {
        if let Some(inner) = caps.get(1) {
            let text = inner.as_str();
            // Skip if already matched by another pattern
            let already_matched = matches.iter().any(|existing| {
                existing.start <= inner.start() && existing.end >= inner.end()
            });
            if !already_matched && is_high_entropy(text) {
                let full = caps.get(0).unwrap();
                matches.push(SecretMatch {
                    secret_type: SecretType::HighEntropy,
                    line: line_of(full.start()),
                    start: full.start(),
                    end: full.end(),
                    matched_text: full.as_str().to_string(),
                });
            }
        }
    }

    // Sort by byte offset for consistent ordering
    matches.sort_by_key(|m| m.start);
    matches
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Redact detected secrets in source code, replacing matched text with
/// `[REDACTED:type]` placeholders.
///
/// Preserves code structure (line count, surrounding syntax) while removing
/// credential values.
pub fn redact_secrets(source: &str, matches: &[SecretMatch]) -> String {
    if matches.is_empty() {
        return source.to_string();
    }

    let mut result = String::with_capacity(source.len());
    let mut last_end = 0;

    for m in matches {
        // Append text before this match
        if m.start > last_end {
            result.push_str(&source[last_end..m.start]);
        }
        // Append redaction placeholder
        result.push_str(&format!("[REDACTED:{}]", m.secret_type.label()));
        last_end = m.end;
    }

    // Append remaining text after last match
    if last_end < source.len() {
        result.push_str(&source[last_end..]);
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_aws_key() {
        let source = r#"let key = "AKIAIOSFODNN7EXAMPLE";"#;
        let matches = detect_secrets(source);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].secret_type, SecretType::AwsAccessKey);
        assert_eq!(matches[0].matched_text, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(matches[0].line, 1);
    }

    #[test]
    fn test_detect_github_token() {
        let source = r#"token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij""#;
        let matches = detect_secrets(source);
        assert!(matches.iter().any(|m| m.secret_type == SecretType::GitHubToken));
    }

    #[test]
    fn test_detect_generic_password() {
        let source = r#"password = "super_secret_value_123""#;
        let matches = detect_secrets(source);
        assert!(matches.iter().any(|m| m.secret_type == SecretType::GenericPassword));
    }

    #[test]
    fn test_detect_connection_string() {
        let source = r#"db_url = "postgres://user:pass@host:5432/db""#;
        let matches = detect_secrets(source);
        assert!(matches.iter().any(|m| m.secret_type == SecretType::ConnectionString));
    }

    #[test]
    fn test_detect_private_key() {
        let source = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQ...";
        let matches = detect_secrets(source);
        assert!(matches.iter().any(|m| m.secret_type == SecretType::PrivateKey));
    }

    #[test]
    fn test_shannon_entropy_low() {
        // Repeated characters have low entropy
        assert!(shannon_entropy("aaaaaaaaaa") < 1.0);
    }

    #[test]
    fn test_shannon_entropy_high() {
        // Random-looking string should have high entropy
        let high = "aB3$xZ9!mK7@pQ2&wL5#";
        assert!(shannon_entropy(high) > 4.0);
    }

    #[test]
    fn test_is_high_entropy_short_string() {
        // Short strings are not flagged even if high entropy
        assert!(!is_high_entropy("aB3$xZ9!"));
    }

    #[test]
    fn test_redact_secrets_aws_key() {
        let source = r#"let key = "AKIAIOSFODNN7EXAMPLE";"#;
        let matches = detect_secrets(source);
        let redacted = redact_secrets(source, &matches);
        assert!(redacted.contains("[REDACTED:aws_access_key]"));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_redact_secrets_preserves_structure() {
        let source = "line1\nlet key = \"AKIAIOSFODNN7EXAMPLE\";\nline3";
        let matches = detect_secrets(source);
        let redacted = redact_secrets(source, &matches);
        assert!(redacted.contains("line1\n"));
        assert!(redacted.contains(";\nline3"));
        assert!(redacted.contains("[REDACTED:aws_access_key]"));
    }

    #[test]
    fn test_no_secrets_returns_unchanged() {
        let source = "fn main() {\n    println!(\"hello\");\n}";
        let matches = detect_secrets(source);
        assert!(matches.is_empty());
        let redacted = redact_secrets(source, &matches);
        assert_eq!(redacted, source);
    }

    #[test]
    fn test_multiple_secrets_all_redacted() {
        let source = r#"
let aws = "AKIAIOSFODNN7EXAMPLE";
let db = "postgres://admin:pass@localhost/mydb";
"#;
        let matches = detect_secrets(source);
        assert!(matches.len() >= 2);
        let redacted = redact_secrets(source, &matches);
        assert!(redacted.contains("[REDACTED:aws_access_key]"));
        assert!(redacted.contains("[REDACTED:connection_string]"));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!redacted.contains("postgres://"));
    }

    #[test]
    fn test_line_numbers_correct() {
        let source = "line1\nline2\nAKIAIOSFODNN7EXAMPLE\nline4";
        let matches = detect_secrets(source);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 3);
    }
}
