// Configurable model pricing loaded from ~/.cortex/pricing.toml.
//
// Supports prefix-based model name matching with longest-prefix resolution.
// Falls back to built-in defaults when the file is missing or contains
// invalid TOML.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// Top-level pricing configuration containing a list of model pricing entries.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PricingConfig {
    /// List of model pricing entries for prefix matching.
    #[serde(default)]
    pub models: Vec<PricingEntry>,
}

/// A single pricing entry mapping a model name prefix to token costs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PricingEntry {
    /// Prefix pattern to match model names (e.g., "claude-3-5-sonnet").
    pub pattern: String,
    /// Cost per million input tokens in USD.
    pub input_cost_per_million: f64,
    /// Cost per million output tokens in USD.
    pub output_cost_per_million: f64,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl PricingConfig {
    /// Load pricing from `~/.cortex/pricing.toml`, falling back to defaults.
    ///
    /// - If the file does not exist, returns defaults silently.
    /// - If the file contains invalid TOML, logs a warning and returns defaults.
    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => Self::parse_or_default(&content),
            Err(_) => Self::defaults(),
        }
    }

    /// Load pricing from a specific TOML string, falling back to defaults on
    /// parse failure. Useful for testing without filesystem access.
    pub fn load_from_str(content: &str) -> Self {
        Self::parse_or_default(content)
    }

    /// Resolve pricing for a model name using longest-prefix matching.
    ///
    /// Filters entries where `model_name.starts_with(pattern)`, then returns
    /// the entry with the longest pattern string (most specific match).
    /// Returns `None` if no pattern matches.
    pub fn resolve(&self, model_name: &str) -> Option<&PricingEntry> {
        self.models
            .iter()
            .filter(|entry| model_name.starts_with(&entry.pattern))
            .max_by_key(|entry| entry.pattern.len())
    }

    /// Returns the built-in default pricing configuration for common models (May 2026).
    pub fn defaults() -> Self {
        PricingConfig {
            models: vec![
                // Anthropic Claude
                PricingEntry {
                    pattern: "claude-opus-4".into(),
                    input_cost_per_million: 5.0,
                    output_cost_per_million: 25.0,
                },
                PricingEntry {
                    pattern: "claude-sonnet-4".into(),
                    input_cost_per_million: 3.0,
                    output_cost_per_million: 15.0,
                },
                PricingEntry {
                    pattern: "claude-haiku-4".into(),
                    input_cost_per_million: 1.0,
                    output_cost_per_million: 5.0,
                },
                // OpenAI GPT
                PricingEntry {
                    pattern: "gpt-5-5".into(),
                    input_cost_per_million: 5.0,
                    output_cost_per_million: 30.0,
                },
                PricingEntry {
                    pattern: "gpt-5-4".into(),
                    input_cost_per_million: 2.5,
                    output_cost_per_million: 15.0,
                },
                PricingEntry {
                    pattern: "gpt-4-1".into(),
                    input_cost_per_million: 2.0,
                    output_cost_per_million: 8.0,
                },
                PricingEntry {
                    pattern: "gpt-4o".into(),
                    input_cost_per_million: 2.5,
                    output_cost_per_million: 10.0,
                },
                PricingEntry {
                    pattern: "gpt-4o-mini".into(),
                    input_cost_per_million: 0.15,
                    output_cost_per_million: 0.6,
                },
                // OpenAI Reasoning
                PricingEntry {
                    pattern: "o4-mini".into(),
                    input_cost_per_million: 0.55,
                    output_cost_per_million: 2.2,
                },
                PricingEntry {
                    pattern: "o3".into(),
                    input_cost_per_million: 2.0,
                    output_cost_per_million: 8.0,
                },
                // Google Gemini
                PricingEntry {
                    pattern: "gemini-2-5-pro".into(),
                    input_cost_per_million: 1.0,
                    output_cost_per_million: 10.0,
                },
                PricingEntry {
                    pattern: "gemini-2-5-flash".into(),
                    input_cost_per_million: 0.3,
                    output_cost_per_million: 2.5,
                },
                PricingEntry {
                    pattern: "gemini-2-0-flash".into(),
                    input_cost_per_million: 0.1,
                    output_cost_per_million: 0.4,
                },
                // xAI Grok
                PricingEntry {
                    pattern: "grok-4".into(),
                    input_cost_per_million: 2.0,
                    output_cost_per_million: 6.0,
                },
                // DeepSeek
                PricingEntry {
                    pattern: "deepseek-v4".into(),
                    input_cost_per_million: 0.14,
                    output_cost_per_million: 0.28,
                },
                // Mistral
                PricingEntry {
                    pattern: "mistral-large".into(),
                    input_cost_per_million: 0.5,
                    output_cost_per_million: 1.5,
                },
                PricingEntry {
                    pattern: "codestral".into(),
                    input_cost_per_million: 0.3,
                    output_cost_per_million: 0.9,
                },
            ],
        }
    }

    /// Path to the user's pricing configuration file.
    fn config_path() -> PathBuf {
        home_dir()
            .unwrap_or_default()
            .join(".cortex")
            .join("pricing.toml")
    }

    /// Parse TOML content into a PricingConfig, logging a warning and falling
    /// back to defaults on failure.
    fn parse_or_default(content: &str) -> Self {
        match toml::from_str::<PricingConfig>(content) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!("Invalid pricing.toml: {e}. Using defaults.");
                Self::defaults()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the user's home directory (cross-platform).
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_defaults_has_entries() {
        let cfg = PricingConfig::defaults();
        assert!(
            cfg.models.len() >= 10,
            "defaults should have many model entries"
        );
    }

    #[test]
    fn test_defaults_contains_expected_models() {
        let cfg = PricingConfig::defaults();
        let patterns: Vec<&str> = cfg.models.iter().map(|e| e.pattern.as_str()).collect();
        assert!(patterns.contains(&"claude-sonnet-4"));
        assert!(patterns.contains(&"gpt-4o"));
        assert!(patterns.contains(&"gemini-2-5-pro"));
        assert!(patterns.contains(&"o3"));
    }

    #[test]
    fn test_resolve_exact_match() {
        let cfg = PricingConfig::defaults();
        let entry = cfg.resolve("gpt-4o").unwrap();
        assert_eq!(entry.pattern, "gpt-4o");
        assert_eq!(entry.input_cost_per_million, 2.5);
        assert_eq!(entry.output_cost_per_million, 10.0);
    }

    #[test]
    fn test_resolve_prefix_match() {
        let cfg = PricingConfig::defaults();
        let entry = cfg.resolve("claude-opus-4-7-20260501").unwrap();
        assert_eq!(entry.pattern, "claude-opus-4");
    }

    #[test]
    fn test_resolve_longest_prefix_wins() {
        let cfg = PricingConfig::defaults();
        // "gpt-4o-mini-2026" matches both "gpt-4o" and "gpt-4o-mini"
        let entry = cfg.resolve("gpt-4o-mini-2026").unwrap();
        assert_eq!(entry.pattern, "gpt-4o-mini");
        assert_eq!(entry.input_cost_per_million, 0.15);
    }

    #[test]
    fn test_resolve_no_match() {
        let cfg = PricingConfig::defaults();
        let entry = cfg.resolve("llama-3-70b");
        assert!(entry.is_none());
    }

    #[test]
    fn test_load_from_valid_toml() {
        let toml = r#"
[[models]]
pattern = "custom-model"
input_cost_per_million = 1.5
output_cost_per_million = 5.0
"#;
        let cfg = PricingConfig::load_from_str(toml);
        assert_eq!(cfg.models.len(), 1);
        assert_eq!(cfg.models[0].pattern, "custom-model");
        assert_eq!(cfg.models[0].input_cost_per_million, 1.5);
        assert_eq!(cfg.models[0].output_cost_per_million, 5.0);
    }

    #[test]
    fn test_load_from_invalid_toml_returns_defaults() {
        let invalid = "this is not valid [[[ toml {{{";
        let cfg = PricingConfig::load_from_str(invalid);
        // Should fall back to defaults
        let defaults = PricingConfig::defaults();
        assert_eq!(cfg.models.len(), defaults.models.len());
    }

    #[test]
    fn test_load_from_empty_string_returns_empty_models() {
        // Empty string is valid TOML with no models key, so serde default kicks in
        let cfg = PricingConfig::load_from_str("");
        assert_eq!(cfg.models.len(), 0);
    }

    #[test]
    fn test_resolve_with_empty_models() {
        let cfg = PricingConfig { models: vec![] };
        assert!(cfg.resolve("anything").is_none());
    }

    #[test]
    fn test_resolve_with_multiple_custom_entries() {
        let cfg = PricingConfig {
            models: vec![
                PricingEntry {
                    pattern: "gpt-4".into(),
                    input_cost_per_million: 30.0,
                    output_cost_per_million: 60.0,
                },
                PricingEntry {
                    pattern: "gpt-4-turbo".into(),
                    input_cost_per_million: 10.0,
                    output_cost_per_million: 30.0,
                },
                PricingEntry {
                    pattern: "gpt-4-turbo-2024".into(),
                    input_cost_per_million: 8.0,
                    output_cost_per_million: 24.0,
                },
            ],
        };

        // Most specific match wins
        let entry = cfg.resolve("gpt-4-turbo-2024-04-09").unwrap();
        assert_eq!(entry.pattern, "gpt-4-turbo-2024");
        assert_eq!(entry.input_cost_per_million, 8.0);

        // Less specific
        let entry = cfg.resolve("gpt-4-turbo-preview").unwrap();
        assert_eq!(entry.pattern, "gpt-4-turbo");

        // Least specific
        let entry = cfg.resolve("gpt-4-0613").unwrap();
        assert_eq!(entry.pattern, "gpt-4");
    }

    #[test]
    fn test_serialization_round_trip() {
        let original = PricingConfig::defaults();
        let toml_str = toml::to_string(&original).expect("serialize");
        let deserialized: PricingConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(original.models.len(), deserialized.models.len());
        for (a, b) in original.models.iter().zip(deserialized.models.iter()) {
            assert_eq!(a.pattern, b.pattern);
            assert_eq!(a.input_cost_per_million, b.input_cost_per_million);
            assert_eq!(a.output_cost_per_million, b.output_cost_per_million);
        }
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    /// Strategy that generates a base prefix and extends it to create multiple
    /// prefix patterns of increasing length, plus a model name that starts with
    /// the longest prefix.
    fn arb_prefix_hierarchy() -> impl Strategy<Value = (Vec<String>, String)> {
        // Generate a base prefix (1-8 lowercase alpha chars)
        "[a-z]{1,8}".prop_flat_map(|base: String| {
            // Generate 1-4 extensions to create progressively longer prefixes
            prop::collection::vec("[a-z]{1,5}", 1..=4).prop_flat_map(move |extensions| {
                let base = base.clone();
                // Build prefix hierarchy: base, base+ext1, base+ext1+ext2, ...
                let mut prefixes = vec![base.clone()];
                let mut current = base;
                for ext in &extensions {
                    current = format!("{}-{}", current, ext);
                    prefixes.push(current.clone());
                }
                // The longest prefix is the last one
                let longest = prefixes.last().unwrap().clone();
                // Generate a suffix to append to the longest prefix for the model name
                "[a-z0-9]{0,10}".prop_map(move |suffix| {
                    let model_name = if suffix.is_empty() {
                        longest.clone()
                    } else {
                        format!("{}-{}", longest, suffix)
                    };
                    (prefixes.clone(), model_name)
                })
            })
        })
    }

    // **Validates: Requirements 5.3, 5.4**

    proptest! {
        #[test]
        fn prop_pricing_resolve_returns_longest_matching_prefix(
            (prefixes, model_name) in arb_prefix_hierarchy()
        ) {
            // Build a PricingConfig with entries for each prefix pattern,
            // each with distinct costs so we can verify the correct one is returned.
            let models: Vec<PricingEntry> = prefixes
                .iter()
                .enumerate()
                .map(|(i, pattern)| PricingEntry {
                    pattern: pattern.clone(),
                    input_cost_per_million: (i + 1) as f64,
                    output_cost_per_million: (i + 1) as f64 * 2.0,
                })
                .collect();

            let cfg = PricingConfig { models };

            // resolve() should return the entry with the longest matching prefix
            let result = cfg.resolve(&model_name);
            prop_assert!(result.is_some(), "resolve() should find a match for model_name={}", model_name);

            let resolved = result.unwrap();
            // The longest prefix is the last in our hierarchy
            let longest_prefix = prefixes.last().unwrap();
            prop_assert_eq!(
                &resolved.pattern,
                longest_prefix,
                "Expected longest prefix '{}' but got '{}' for model '{}'",
                longest_prefix,
                resolved.pattern,
                model_name
            );
        }
    }

    /// Strategy to generate a valid PricingEntry with arbitrary pattern and costs.
    fn arb_pricing_entry() -> impl Strategy<Value = PricingEntry> {
        (
            "[a-z][a-z0-9\\-]{0,20}", // pattern: non-empty, TOML-safe string
            0.001f64..1000.0f64,      // input_cost_per_million: positive finite
            0.001f64..1000.0f64,      // output_cost_per_million: positive finite
        )
            .prop_map(|(pattern, input_cost, output_cost)| PricingEntry {
                pattern,
                input_cost_per_million: input_cost,
                output_cost_per_million: output_cost,
            })
    }

    /// Strategy to generate a valid PricingConfig with 0-10 entries.
    fn arb_pricing_config() -> impl Strategy<Value = PricingConfig> {
        prop::collection::vec(arb_pricing_entry(), 0..=10)
            .prop_map(|models| PricingConfig { models })
    }

    // **Validates: Requirements 5.2**

    proptest! {
        /// Property 3: Pricing config deserialization round-trip.
        /// For any valid PricingConfig, serializing to TOML and deserializing
        /// back produces an equivalent struct with identical field values.
        #[test]
        fn prop_pricing_config_round_trip(config in arb_pricing_config()) {
            // Serialize to TOML
            let toml_str = toml::to_string(&config)
                .expect("PricingConfig should always serialize to valid TOML");

            // Deserialize back
            let deserialized: PricingConfig = toml::from_str(&toml_str)
                .expect("Serialized TOML should always deserialize back");

            // Verify equivalence: same number of entries
            prop_assert_eq!(
                config.models.len(),
                deserialized.models.len(),
                "Model count mismatch after round-trip"
            );

            // Verify each entry has identical field values
            for (original, restored) in config.models.iter().zip(deserialized.models.iter()) {
                prop_assert_eq!(
                    &original.pattern,
                    &restored.pattern,
                    "Pattern mismatch after round-trip"
                );
                prop_assert_eq!(
                    original.input_cost_per_million,
                    restored.input_cost_per_million,
                    "input_cost_per_million mismatch after round-trip"
                );
                prop_assert_eq!(
                    original.output_cost_per_million,
                    restored.output_cost_per_million,
                    "output_cost_per_million mismatch after round-trip"
                );
            }
        }
    }

    // **Validates: Requirements 5.7**

    proptest! {
        /// Property 4: Invalid TOML falls back to defaults.
        ///
        /// For any string that is not valid TOML (i.e., cannot be deserialized
        /// into a PricingConfig), `PricingConfig::load_from_str()` returns the
        /// default pricing configuration (4 entries) without panicking.
        #[test]
        fn prop_invalid_toml_falls_back_to_defaults(
            input in "\\PC*"
                .prop_filter("must not be valid PricingConfig TOML",
                    |s| toml::from_str::<PricingConfig>(s).is_err())
        ) {
            let cfg = PricingConfig::load_from_str(&input);
            let defaults = PricingConfig::defaults();

            // Must return the default entries
            prop_assert_eq!(
                cfg.models.len(),
                defaults.models.len(),
                "Expected {} default entries, got {}",
                defaults.models.len(),
                cfg.models.len()
            );

            // Each entry must match the defaults exactly
            for (actual, expected) in cfg.models.iter().zip(defaults.models.iter()) {
                prop_assert_eq!(
                    &actual.pattern, &expected.pattern,
                    "Pattern mismatch: got '{}', expected '{}'",
                    actual.pattern, expected.pattern
                );
                prop_assert_eq!(
                    actual.input_cost_per_million, expected.input_cost_per_million,
                    "Input cost mismatch for pattern '{}'",
                    actual.pattern
                );
                prop_assert_eq!(
                    actual.output_cost_per_million, expected.output_cost_per_million,
                    "Output cost mismatch for pattern '{}'",
                    actual.pattern
                );
            }
        }
    }
}
