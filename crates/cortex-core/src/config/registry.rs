//! The registry of configuration keys: one entry per key Cortex reads, saying what it is for, how
//! a file's value is read into a [`Config`] and how the value in effect is shown.
//!
//! Each section of the configuration keeps its keys in a table of its own, beside the struct that
//! holds them; [`SECTIONS`] lists the tables. Adding a key is one entry in its section's table and
//! the field it sets; no other code matches on a key's name to read or show it.

use super::{Config, adopt, compaction, coordination, hints, main_keys, retrieval, warnings};

/// How an entry reads a file's value into the settings: `Err` holds what the key accepts, in the
/// words its refusal uses.
type Read = fn(&mut Config, &toml::Value) -> Result<(), &'static str>;

/// How an entry shows the value in effect, as the TOML value a file would set it with.
type Show = fn(&Config) -> toml::Value;

/// One configuration key.
#[derive(Debug, Clone, Copy)]
pub struct ConfigKey {
    /// The key, written with dots: `retrieval.max_files`.
    pub name: &'static str,
    /// What the key changes, in one line.
    pub description: &'static str,
    /// Reads a file's value into the settings.
    read: Read,
    /// Shows the value in effect.
    show: Show,
}

impl ConfigKey {
    /// A key, either file setting it alike.
    pub(super) const fn new(
        name: &'static str,
        description: &'static str,
        read: Read,
        show: Show,
    ) -> Self {
        Self {
            name,
            description,
            read,
            show,
        }
    }

    /// Sets the key in `config` to `value`.
    ///
    /// # Errors
    ///
    /// The value is unusable: the error says what the key accepts, such as `a whole number above
    /// 0`, and `config` is left as it was.
    pub fn read(&self, config: &mut Config, value: &toml::Value) -> Result<(), &'static str> {
        (self.read)(config, value)
    }

    /// The key's value in `config`, as a file would set it.
    #[must_use]
    pub fn show(&self, config: &Config) -> toml::Value {
        (self.show)(config)
    }
}

/// Every section's table of keys.
const SECTIONS: [&[ConfigKey]; 7] = [
    main_keys::KEYS,
    retrieval::KEYS,
    warnings::KEYS,
    coordination::KEYS,
    hints::KEYS,
    compaction::KEYS,
    adopt::KEYS,
];

/// Every key Cortex reads but `profile`, section by section.
pub fn config_keys() -> impl Iterator<Item = &'static ConfigKey> {
    SECTIONS.into_iter().flatten()
}

/// The key named `name`, written with dots, when Cortex reads one.
#[must_use]
pub fn config_key(name: &str) -> Option<&'static ConfigKey> {
    config_keys().find(|key| key.name == name)
}

/// A whole number as a TOML value. Every number a key holds was read from a TOML integer or is a
/// default far below its limit, so it fits; one that did not would show as the largest integer.
pub(super) fn integer(number: u64) -> toml::Value {
    toml::Value::Integer(i64::try_from(number).unwrap_or(i64::MAX))
}

/// A list of strings as a TOML value.
pub(super) fn strings(list: &[String]) -> toml::Value {
    toml::Value::Array(list.iter().cloned().map(toml::Value::String).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::config::{
        ConfigError, ConfigLayer, CoordinationConfig, ModeSetting, RetrievalConfig, TypeWeights,
        WarningsConfig, load,
    };
    use crate::{AdoptConfig, CompactionConfig, HintsConfig, ScaleMode};

    /// Every key Cortex reads set to a value other than its default, one line each, with the
    /// settings that result written out by hand below.
    const EVERY_KEY: &str = "mode = \"huge\"\n\
        index.max_file_bytes = 2000\n\
        index.extra_ignores = [\"*.log\"]\n\
        index.parse_timeout_ms = 250\n\
        index.crawl_files_per_second = 7\n\
        daemon.idle_shutdown_minutes = 5\n\
        store.mmap_bytes = 0\n\
        privacy.store_prompt_excerpt = true\n\
        holdout.share = 0.25\n\
        learning.eta = 0.2\n\
        learning.link_decay = 0.75\n\
        learning.interest_decay = 0.5\n\
        git.max_commits = 100\n\
        git.max_age_days = 7\n\
        git.max_files_per_commit = 5\n\
        git.half_life_commits = 50\n\
        tests.extra_patterns = [\"^just test\"]\n\
        strict_mode = true\n\
        retrieval.max_tokens = 1200\n\
        retrieval.max_files = 20\n\
        retrieval.max_code_spans = 0\n\
        retrieval.span_max_lines = 1\n\
        retrieval.teleport = 1.0\n\
        retrieval.min_confidence = 0.8\n\
        retrieval.type_weights.structural = 0.0\n\
        retrieval.type_weights.cochange = 1.0\n\
        retrieval.type_weights.coaccess = 0.25\n\
        retrieval.type_weights.about = 0.5\n\
        retrieval.type_weights.containment = 0.05\n\
        warnings.max_per_edit = 0\n\
        warnings.cochange_min_confidence = 1.0\n\
        warnings.cochange_min_count = 7\n\
        warnings.cochange_min_link_weight = 2.5\n\
        warnings.cochange_min_link_tasks = 6\n\
        coordination.read_minutes = 1\n\
        coordination.edit_minutes = 2\n\
        coordination.edit_after_task_minutes = 3\n\
        coordination.claim_minutes = 1440\n\
        coordination.live_minutes = 4\n\
        coordination.same_file_minutes = 6\n\
        coordination.handoff_idle_minutes = 45\n\
        coordination.structural_min_weight = 0.9\n\
        coordination.cochange_min_confidence = 0.0\n\
        coordination.max_notices = 0\n\
        coordination.max_tokens = 400\n\
        coordination.contracts = true\n\
        hints.runtime_grounding = true\n\
        hints.narrow_tests = true\n\
        hints.redundant_commands = true\n\
        compaction.keep_list = true\n\
        adopt.exclude = [\"~/Downloads/**\"]\n";

    /// The settings [`EVERY_KEY`] gives, every field away from its default.
    fn every_key_set() -> Config {
        Config {
            mode: ModeSetting::Pinned(ScaleMode::Huge),
            max_file_bytes: 2000,
            extra_ignores: vec!["*.log".to_owned()],
            parse_timeout_ms: 250,
            idle_shutdown_minutes: 5,
            mmap_bytes: 0,
            crawl_files_per_second: 7,
            store_prompt_excerpt: true,
            holdout_share: 0.25,
            learning_eta: 0.2,
            link_decay: 0.75,
            interest_decay: 0.5,
            git_max_commits: 100,
            git_max_age_days: 7,
            git_max_files_per_commit: 5,
            git_half_life_commits: 50,
            test_extra_patterns: vec!["^just test".to_owned()],
            retrieval: RetrievalConfig {
                max_tokens: 1200,
                max_files: 20,
                max_code_spans: 0,
                span_max_lines: 1,
                teleport: 1.0,
                type_weights: TypeWeights {
                    structural: 0.0,
                    cochange: 1.0,
                    coaccess: 0.25,
                    about: 0.5,
                    containment: 0.05,
                },
                min_confidence: 0.8,
            },
            warnings: WarningsConfig {
                max_per_edit: 0,
                cochange_min_confidence: 1.0,
                cochange_min_count: 7,
                cochange_min_link_weight: 2.5,
                cochange_min_link_tasks: 6,
            },
            coordination: CoordinationConfig {
                read_minutes: 1,
                edit_minutes: 2,
                edit_after_task_minutes: 3,
                claim_minutes: 1440,
                live_minutes: 4,
                same_file_minutes: 6,
                handoff_idle_minutes: 45,
                structural_min_weight: 0.9,
                cochange_min_confidence: 0.0,
                max_notices: 0,
                max_tokens: 400,
                is_contracts: true,
            },
            is_strict_mode: true,
            hints: HintsConfig {
                is_runtime_grounding: true,
                is_narrow_tests: true,
                is_redundant_commands: true,
            },
            compaction: CompactionConfig { is_keep_list: true },
            adopt: AdoptConfig {
                exclude: vec!["~/Downloads/**".to_owned()],
            },
        }
    }

    /// The registry names each key once, and [`EVERY_KEY`] sets exactly the registry's keys. Each
    /// field of the settings literal above differs from its default, and the compiler makes the
    /// literal name every field, so loading [`EVERY_KEY`] reaches every field: a key that set
    /// another key's field would leave one at its default, and the comparison would fail. Each
    /// entry then shows the value its line set: show and read name the same field.
    #[test]
    fn every_field_has_exactly_one_key() -> Result<(), ConfigError> {
        let names: Vec<&str> = config_keys().map(|key| key.name).collect();
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "a key is in the registry twice");
        let lines: BTreeSet<&str> = EVERY_KEY
            .lines()
            .filter_map(|line| line.split(" = ").next())
            .collect();
        assert_eq!(lines, unique);
        let loaded = load(&[ConfigLayer {
            origin: "f",
            text: EVERY_KEY,
        }])?;
        assert_eq!(loaded.warnings, Vec::<String>::new());
        assert_eq!(loaded.config, every_key_set());
        assert_ne!(every_key_set(), Config::default());
        for line in EVERY_KEY.lines() {
            let (name, value) = line.split_once(" = ").ok_or(ConfigError::Syntax {
                origin: line.to_owned(),
                message: "no ` = `".to_owned(),
            })?;
            let key = config_key(name).ok_or(ConfigError::Syntax {
                origin: name.to_owned(),
                message: "not in the registry".to_owned(),
            })?;
            assert_eq!(key.show(&loaded.config).to_string(), value, "{name}");
        }
        Ok(())
    }

    /// Each key's default, shown as text and read back from it, is the default again: read into
    /// settings where every key is away from its default, the key shows its default once more; read
    /// into the defaults, it changes nothing. Each key says what it does.
    #[test]
    fn every_default_round_trips() {
        let defaults = Config::default();
        for key in config_keys() {
            let shown = key.show(&defaults);
            let reparsed: Result<toml::Value, _> = shown.to_string().parse();
            assert_eq!(reparsed.as_ref(), Ok(&shown), "{}", key.name);
            let mut config = every_key_set();
            assert_eq!(key.read(&mut config, &shown), Ok(()), "{}", key.name);
            let mut fresh = Config::default();
            assert_eq!(key.read(&mut fresh, &shown), Ok(()), "{}", key.name);
            assert_eq!(fresh, defaults, "{}", key.name);
            assert_eq!(key.show(&config), shown, "{}", key.name);
            assert!(!key.description.is_empty(), "{}", key.name);
        }
    }

    /// An unusable value leaves the settings as they were and says what the key accepts.
    #[test]
    fn an_unusable_value_changes_nothing() {
        let mut config = Config::default();
        let key = config_key("retrieval.max_files");
        assert_eq!(
            key.map(|key| key.read(&mut config, &toml::Value::Integer(21))),
            Some(Err("a whole number from 1 to 20"))
        );
        assert_eq!(config, Config::default());
        assert!(config_key("retrieval").is_none());
        assert!(config_key("profile").is_none());
    }
}
