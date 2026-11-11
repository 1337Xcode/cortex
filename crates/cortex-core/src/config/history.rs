//! The history settings: whether the code Cortex shows is given the reason it looks that way from
//! git history, and how much of it; whether past changes a prompt names or resembles start a
//! search; how long one look at git may take and which authors are bots. Both switches are off by
//! default until their evaluation shows they help.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use globset::GlobBuilder;

use super::registry::strings;
use super::{ConfigKey, SWITCH, count, string_list};

/// The default for `history.blame_timeout_ms`: two seconds to find who last changed some lines.
pub const DEFAULT_BLAME_TIMEOUT_MS: u32 = 2000;

/// The fewest milliseconds `history.blame_timeout_ms` may be set to: less would give up before git
/// has started.
const MIN_BLAME_TIMEOUT_MS: u32 = 100;

/// The most milliseconds `history.blame_timeout_ms` may be set to: a minute.
const MAX_BLAME_TIMEOUT_MS: u32 = 60_000;

/// The default for `history.max_reasons`: the most reasons attached to one shown stretch of code.
pub const DEFAULT_MAX_REASONS: u32 = 2;

/// The most `history.max_reasons` may be set to: more would bury the code they explain.
const MAX_REASONS_LIMIT: u32 = 5;

/// The default for `history.max_tokens`: the most tokens of reasons attached to one shown stretch
/// of code.
pub const DEFAULT_MAX_TOKENS: u32 = 120;

/// The fewest `history.max_tokens` may be set to: a reason shorter than this says nothing.
const MIN_TOKENS_LIMIT: u32 = 20;

/// The most `history.max_tokens` may be set to.
const MAX_TOKENS_LIMIT: u32 = 400;

/// What `history.bot_patterns` accepts, in its refusal.
const GLOBS: &str = "a list of glob patterns, such as \"*[bot]\"";

/// The settings the history features read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryConfig {
    /// `history.why`: attach the reason git history gives for how the code looks to the code
    /// shown. Default `false`.
    pub is_why: bool,
    /// `history.seeds`: use the past changes a prompt names or resembles as starting points of the
    /// search. Default `false`.
    pub is_seeds: bool,
    /// `history.blame_timeout_ms`: how long one look at who last changed some lines may take, from
    /// 100 to 60000. Default [`DEFAULT_BLAME_TIMEOUT_MS`].
    pub blame_timeout_ms: u32,
    /// `history.max_reasons`: the most reasons attached to one shown stretch of code, from 1 to
    /// 5. Default [`DEFAULT_MAX_REASONS`].
    pub max_reasons: u32,
    /// `history.max_tokens`: the most tokens of reasons attached to one shown stretch of code, from
    /// 20 to 400. Default [`DEFAULT_MAX_TOKENS`].
    pub max_tokens: u32,
    /// `history.bot_patterns`: glob patterns matched, in any case, against a commit author's name;
    /// a commit by a match is not history worth reading. Default none.
    pub bot_patterns: Vec<String>,
}

impl Default for HistoryConfig {
    /// Every history setting at its documented default.
    fn default() -> Self {
        Self {
            is_why: false,
            is_seeds: false,
            blame_timeout_ms: DEFAULT_BLAME_TIMEOUT_MS,
            max_reasons: DEFAULT_MAX_REASONS,
            max_tokens: DEFAULT_MAX_TOKENS,
            bot_patterns: Vec::new(),
        }
    }
}

/// Whether `pattern` is a glob, read as it will be matched against author names: in any case.
fn is_author_glob(pattern: &str) -> bool {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .is_ok()
}

/// The history keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[
    ConfigKey::new(
        "history.why",
        "Attach the reason git history gives for how the code looks to the code Cortex shows.",
        |config, value| {
            config.history.is_why = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.history.is_why),
    ),
    ConfigKey::new(
        "history.seeds",
        "Use the past changes a prompt names or resembles as starting points of the search for \
         the code a task needs.",
        |config, value| {
            config.history.is_seeds = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.history.is_seeds),
    ),
    ConfigKey::new(
        "history.blame_timeout_ms",
        "How long, in milliseconds, one look at who last changed some lines may take before the \
         code is shown without a reason.",
        |config, value| {
            config.history.blame_timeout_ms =
                count(value, MIN_BLAME_TIMEOUT_MS, MAX_BLAME_TIMEOUT_MS)
                    .ok_or("a whole number from 100 to 60000")?;
            Ok(())
        },
        |config| config.history.blame_timeout_ms.into(),
    ),
    ConfigKey::new(
        "history.max_reasons",
        "The most reasons attached to one shown stretch of code.",
        |config, value| {
            config.history.max_reasons =
                count(value, 1, MAX_REASONS_LIMIT).ok_or("a whole number from 1 to 5")?;
            Ok(())
        },
        |config| config.history.max_reasons.into(),
    ),
    ConfigKey::new(
        "history.max_tokens",
        "The most tokens of reasons attached to one shown stretch of code.",
        |config, value| {
            config.history.max_tokens = count(value, MIN_TOKENS_LIMIT, MAX_TOKENS_LIMIT)
                .ok_or("a whole number from 20 to 400")?;
            Ok(())
        },
        |config| config.history.max_tokens.into(),
    ),
    ConfigKey::new(
        "history.bot_patterns",
        "Glob patterns, matched in any case against a commit author's name, of the authors whose \
         commits are not history worth reading.",
        |config, value| {
            config.history.bot_patterns = string_list(value)
                .filter(|patterns| patterns.iter().all(|pattern| is_author_glob(pattern)))
                .ok_or(GLOBS)?;
            Ok(())
        },
        |config| strings(&config.history.bot_patterns),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ConfigError, ConfigLayer, LayerKind, load};

    /// Loads a user's file holding `text`.
    fn loaded(text: &str) -> Result<Config, ConfigError> {
        load(&[ConfigLayer {
            kind: LayerKind::User,
            origin: "f",
            text,
        }])
        .map(|loaded| loaded.config)
    }

    /// With no file, the history keys hold the defaults the documentation gives: both switches
    /// off, two seconds, two reasons, 120 tokens and no bot patterns.
    #[test]
    fn the_defaults_are_the_documented_ones() {
        assert_eq!(
            Config::default().history,
            HistoryConfig {
                is_why: false,
                is_seeds: false,
                blame_timeout_ms: 2000,
                max_reasons: 2,
                max_tokens: 120,
                bot_patterns: Vec::new(),
            }
        );
    }

    /// Each number key takes its lowest and highest value and refuses one beyond either and a
    /// word, in the words the key's range gives.
    #[test]
    fn each_number_has_its_range() {
        let ranges = [
            (
                "history.blame_timeout_ms",
                100,
                60_000,
                "a whole number from 100 to 60000",
            ),
            ("history.max_reasons", 1, 5, "a whole number from 1 to 5"),
            (
                "history.max_tokens",
                20,
                400,
                "a whole number from 20 to 400",
            ),
        ];
        for (key, low, high, expected) in ranges {
            for edge in [low, high] {
                assert!(loaded(&format!("{key} = {edge}")).is_ok(), "{key} = {edge}");
            }
            for bad in [
                (low - 1).to_string(),
                (high + 1).to_string(),
                "\"3\"".to_owned(),
            ] {
                assert_eq!(
                    loaded(&format!("{key} = {bad}")),
                    Err(ConfigError::Value {
                        origin: "f".to_owned(),
                        key: key.to_owned(),
                        expected,
                    }),
                    "{key} = {bad}"
                );
            }
        }
    }

    /// A pattern that is no glob (an unclosed bracket) refuses the whole list, and so does a list
    /// holding a number; globs, brackets and an empty list are kept as written.
    #[test]
    fn bot_patterns_must_be_globs() -> Result<(), ConfigError> {
        let expected = "a list of glob patterns, such as \"*[bot]\"";
        for bad in ["[\"a[\"]", "[\"*[bot]\", \"b{\"]", "[1]", "\"dependabot\""] {
            assert_eq!(
                loaded(&format!("history.bot_patterns = {bad}")),
                Err(ConfigError::Value {
                    origin: "f".to_owned(),
                    key: "history.bot_patterns".to_owned(),
                    expected,
                }),
                "{bad}"
            );
        }
        assert_eq!(
            loaded("history.bot_patterns = [\"dependabot*\", \"*[bot]\"]")?
                .history
                .bot_patterns,
            ["dependabot*", "*[bot]"]
        );
        assert_eq!(
            loaded("history.bot_patterns = []")?.history.bot_patterns,
            Vec::<String>::new()
        );
        Ok(())
    }
}
