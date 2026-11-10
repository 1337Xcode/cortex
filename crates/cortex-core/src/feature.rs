//! The registry of features: what Cortex gives agents, whether each is on, the configuration key
//! that switches it, and the measurement that decides whether it should be on.
//!
//! Adding a feature is one entry in [`FEATURES`]; a switchable one names its key, which must be a
//! true-or-false key of the configuration registry. The profiles (`lean`, `everything`) switch
//! exactly the features listed here that have a key.
//!
//! Whether a feature is on follows the settings alone, as the daemon reads them: a feature can
//! be off though no key of its own says so, as warnings are when `warnings.max_per_edit` is 0,
//! and every feature is when `holdout.share` is 1, since a held-out task is given nothing. A
//! repository whose size makes Cortex choose tiny mode by itself gets no work orders or hints
//! either; that depends on the index, not on the settings, and is not shown here.

use crate::{Config, ModeSetting, ScaleMode};

/// Where a feature stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    /// Part of what Cortex is: on by default, with no key of its own.
    On,
    /// Measured and found not to help: off by default.
    Off,
    /// Off by default until its measurement shows it helps.
    Experimental,
}

impl FeatureState {
    /// The state's word: `on`, `off` or `experimental`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Experimental => "experimental",
        }
    }
}

/// One feature.
#[derive(Debug, Clone, Copy)]
pub struct Feature {
    /// Its name on the command line: `narrow_tests`.
    pub name: &'static str,
    /// What it gives an agent, in one line.
    pub description: &'static str,
    /// Where it stands.
    pub state: FeatureState,
    /// The true-or-false key that switches it; `None` when it has none of its own.
    pub key: Option<&'static str>,
    /// What decides whether it should be on: the comparison its evaluation makes.
    pub measure: &'static str,
    /// Whether the settings let it reach a task that is not held out.
    reaches: fn(&Config) -> bool,
}

/// Every feature, in the order `cortex features` lists them.
pub const FEATURES: &[Feature] = &[
    Feature {
        name: "work_orders",
        description: "Each prompt is given the code its task most likely needs, checked against \
                      the files on disk, when the gate is sure enough.",
        state: FeatureState::On,
        key: None,
        measure: "turns, tokens and passing checks of the tasks given work orders, against the \
                  held-out tasks",
        reaches: |config| !is_tiny(config),
    },
    Feature {
        name: "warnings",
        description: "An edit that leaves a file it usually changes with untouched, or call sites \
                      of a signature it changed, is told so.",
        state: FeatureState::On,
        key: None,
        measure: "companion files and call sites left unchanged at a task's end, against the \
                  held-out tasks",
        reaches: |config| config.warnings.max_per_edit > 0,
    },
    Feature {
        name: "runtime_grounding",
        description: "Beside a failed check's output, name the repository lines its stack ran \
                      through.",
        state: FeatureState::Experimental,
        key: Some("hints.runtime_grounding"),
        measure: "turns and tokens from a failed check to a passing one, with it on against off",
        reaches: |config| config.hints.is_runtime_grounding && !is_tiny(config),
    },
    Feature {
        name: "narrow_tests",
        description: "After a whole test-suite run, name the tests of what the task edited and the \
                      command that runs only them.",
        state: FeatureState::Experimental,
        key: Some("hints.narrow_tests"),
        measure: "whole-suite runs and the time checks take per task, with it on against off",
        reaches: |config| config.hints.is_narrow_tests && !is_tiny(config),
    },
    Feature {
        name: "redundant_commands",
        description: "Before a check runs again with no source change since, say what its last \
                      run gave.",
        state: FeatureState::Experimental,
        key: Some("hints.redundant_commands"),
        measure: "checks run again with nothing changed per task, with it on against off",
        reaches: |config| config.hints.is_redundant_commands && !is_tiny(config),
    },
    Feature {
        name: "keep_list",
        description: "Before Claude Code compacts a session's context, give its summary the open \
                      task's working set to keep.",
        state: FeatureState::Experimental,
        key: Some("compaction.keep_list"),
        measure: "files read again after a compaction per task, with it on against off",
        reaches: |config| config.compaction.is_keep_list && !is_tiny(config),
    },
];

/// The feature named `name`.
#[must_use]
pub fn feature(name: &str) -> Option<&'static Feature> {
    FEATURES.iter().find(|feature| feature.name == name)
}

impl Feature {
    /// Whether the feature is on with `config`: whether its own condition holds (its key on, and
    /// for the texts that need an index of some size, a mode other than tiny pinned), and not
    /// every task is held out.
    #[must_use]
    pub fn is_on(&self, config: &Config) -> bool {
        config.holdout_share < 1.0 && (self.reaches)(config)
    }
}

/// Whether the settings pin tiny mode, in which Cortex retrieves nothing and gives no hint.
fn is_tiny(config: &Config) -> bool {
    config.mode == ModeSetting::Pinned(ScaleMode::Tiny)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_key;

    /// Every switch is a true-or-false key of the registry that is off by default, and turns its
    /// feature on and off; a feature without a key is on by default.
    #[test]
    fn every_switch_is_a_registry_switch() {
        for feature in FEATURES {
            let Some(name) = feature.key else {
                assert_eq!(feature.state, FeatureState::On, "{}", feature.name);
                assert!(feature.is_on(&Config::default()), "{}", feature.name);
                continue;
            };
            let key = config_key(name);
            assert!(key.is_some(), "{name}");
            let mut config = Config::default();
            assert!(!feature.is_on(&config), "{}", feature.name);
            for is_on in [true, false] {
                let read = key.map(|key| key.read(&mut config, &toml::Value::Boolean(is_on)));
                assert_eq!(read, Some(Ok(())), "{name}");
                assert_eq!(feature.is_on(&config), is_on, "{}", feature.name);
            }
        }
    }

    /// The names of the features on with `config`.
    fn on_with(config: &Config) -> Vec<&'static str> {
        FEATURES
            .iter()
            .filter(|feature| feature.is_on(config))
            .map(|feature| feature.name)
            .collect()
    }

    /// A feature is off when the settings keep it from every task though its key is on or it has
    /// none: warnings with no warning allowed an edit, work orders and the hints in tiny mode,
    /// everything with every task held out. Worked by hand from the daemon's conditions.
    #[test]
    fn a_feature_the_settings_keep_from_every_task_is_off() {
        let everything = Config {
            hints: crate::HintsConfig {
                is_runtime_grounding: true,
                is_narrow_tests: true,
                is_redundant_commands: true,
            },
            compaction: crate::CompactionConfig { is_keep_list: true },
            ..Config::default()
        };
        assert_eq!(on_with(&everything).len(), 6);
        let no_warnings = Config {
            warnings: crate::WarningsConfig {
                max_per_edit: 0,
                ..crate::WarningsConfig::default()
            },
            ..everything.clone()
        };
        assert!(!on_with(&no_warnings).contains(&"warnings"));
        assert_eq!(on_with(&no_warnings).len(), 5);
        let tiny = Config {
            mode: ModeSetting::Pinned(ScaleMode::Tiny),
            ..everything.clone()
        };
        assert_eq!(on_with(&tiny), ["warnings"]);
        let small = Config {
            mode: ModeSetting::Pinned(ScaleMode::Small),
            ..everything.clone()
        };
        assert_eq!(on_with(&small).len(), 6);
        let all_held_out = Config {
            holdout_share: 1.0,
            ..everything.clone()
        };
        assert_eq!(on_with(&all_held_out), Vec::<&str>::new());
        let most_held_out = Config {
            holdout_share: 0.99,
            ..everything
        };
        assert_eq!(on_with(&most_held_out).len(), 6);
    }

    /// The features, by name, in order; names are unique and found by name.
    #[test]
    fn the_features_are_listed() {
        let names: Vec<&str> = FEATURES.iter().map(|feature| feature.name).collect();
        assert_eq!(
            names,
            [
                "work_orders",
                "warnings",
                "runtime_grounding",
                "narrow_tests",
                "redundant_commands",
                "keep_list"
            ]
        );
        for name in names {
            assert_eq!(feature(name).map(|feature| feature.name), Some(name));
        }
        assert!(feature("hints").is_none());
        assert_eq!(FeatureState::Experimental.name(), "experimental");
    }
}
