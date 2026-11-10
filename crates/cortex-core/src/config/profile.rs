//! Profiles: named sets of defaults for the feature switches, chosen with the `profile` key and
//! applied before any other key a file sets, so that each of those overrides it.
//!
//! - `standard` (the default) sets nothing: every key keeps its documented default.
//! - `lean` sets every feature switch off: `hints.runtime_grounding`, `hints.narrow_tests`,
//!   `hints.redundant_commands`, `compaction.keep_list` and `coordination.contracts` are `false`,
//!   leaving work orders and warnings, which no key switches. Each of those switches is off by
//!   default today, so `lean` differs from `standard` only once one of their defaults turns on.
//! - `everything` sets the same five switches on.
//!
//! The switches are the keys of the features registry ([`crate::FEATURES`]). No profile changes
//! `strict_mode`, which holds an agent's stop, or `privacy.store_prompt_excerpt`, which keeps prompt
//! text: those are the user's own choices, never a default a profile picks.

use super::{Config, ConfigError, config_key};
use crate::FEATURES;

/// The key that chooses the profile.
pub const PROFILE_KEY: &str = "profile";

/// What the profile key changes, in one line.
pub(super) const PROFILE_DESCRIPTION: &str = "The defaults the feature switches start from: standard, lean (every switch off) or \
     everything (every switch on).";

/// What the profile key accepts, in its refusal.
const PROFILES: &str = "one of standard, lean, everything";

/// A named set of defaults for the feature switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Profile {
    /// Every key at its documented default.
    #[default]
    Standard,
    /// Every feature switch off.
    Lean,
    /// Every feature switch on.
    Everything,
}

impl Profile {
    /// Every profile.
    pub const ALL: [Self; 3] = [Self::Standard, Self::Lean, Self::Everything];

    /// The profile's name, as the key takes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Lean => "lean",
            Self::Everything => "everything",
        }
    }

    /// The profile a file's value names.
    ///
    /// # Errors
    ///
    /// The value names no profile: the error says what the key accepts.
    pub fn read(value: &toml::Value) -> Result<Self, &'static str> {
        value
            .as_str()
            .and_then(|name| Self::ALL.into_iter().find(|each| each.name() == name))
            .ok_or(PROFILES)
    }

    /// Each key the profile sets, with the value it gives it, in the order of the features
    /// registry.
    #[must_use]
    pub fn keys(self) -> Vec<(&'static str, bool)> {
        let is_on = match self {
            Self::Standard => return Vec::new(),
            Self::Lean => false,
            Self::Everything => true,
        };
        FEATURES
            .iter()
            .filter_map(|feature| feature.key)
            .map(|key| (key, is_on))
            .collect()
    }

    /// The settings the profile starts from: the defaults, with each of its keys set through the
    /// configuration registry.
    ///
    /// # Errors
    ///
    /// A key the profile sets that the registry does not read as true or false; the tests make
    /// sure there is none.
    pub(super) fn defaults(self) -> Result<Config, ConfigError> {
        let mut config = Config::default();
        for (name, is_on) in self.keys() {
            let refused = |expected| ConfigError::Value {
                origin: format!("the {} profile", self.name()),
                key: name.to_owned(),
                expected,
            };
            let key = config_key(name).ok_or_else(|| refused("a key Cortex reads"))?;
            key.read(&mut config, &toml::Value::Boolean(is_on))
                .map_err(refused)?;
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigLayer, load};
    use crate::{CompactionConfig, HintsConfig};

    /// Loads one file holding `text`.
    fn loaded(text: &str) -> Result<crate::LoadedConfig, ConfigError> {
        load(&[ConfigLayer { origin: "f", text }])
    }

    /// Each profile sets exactly the keys its documentation names, to the value it names.
    #[test]
    fn each_profile_sets_its_keys() {
        let switches = [
            "hints.runtime_grounding",
            "hints.narrow_tests",
            "hints.redundant_commands",
            "compaction.keep_list",
            "coordination.contracts",
        ];
        assert_eq!(Profile::Standard.keys(), Vec::new());
        assert_eq!(
            Profile::Lean.keys(),
            switches.map(|key| (key, false)).to_vec()
        );
        assert_eq!(
            Profile::Everything.keys(),
            switches.map(|key| (key, true)).to_vec()
        );
    }

    /// `everything` turns every switch on, `lean` and `standard` leave them off, and nothing else
    /// moves from its default.
    #[test]
    fn profiles_give_their_defaults() -> Result<(), ConfigError> {
        let everything = loaded("profile = \"everything\"\n")?;
        assert_eq!(everything.profile, Profile::Everything);
        assert_eq!(
            everything.config,
            Config {
                hints: HintsConfig {
                    is_runtime_grounding: true,
                    is_narrow_tests: true,
                    is_redundant_commands: true,
                },
                compaction: CompactionConfig { is_keep_list: true },
                coordination: crate::CoordinationConfig {
                    is_contracts: true,
                    ..crate::CoordinationConfig::default()
                },
                ..Config::default()
            }
        );
        assert_eq!(loaded("profile = \"lean\"\n")?.config, Config::default());
        assert_eq!(
            loaded("profile = \"standard\"\n")?.config,
            Config::default()
        );
        assert_eq!(loaded("")?.profile, Profile::Standard);
        Ok(())
    }

    /// A key a file sets overrides its profile, whichever layer either is in and in whatever order
    /// the file writes them; the last layer's profile is the one applied.
    #[test]
    fn keys_override_the_profile() -> Result<(), ConfigError> {
        let user = ConfigLayer {
            origin: "user",
            text: "hints.narrow_tests = false\nprofile = \"lean\"\n",
        };
        let repository = ConfigLayer {
            origin: "repository",
            text: "[compaction]\nkeep_list = false\n\n[hints]\nprofile = 1\n\n[x]\n",
        };
        let top = ConfigLayer {
            origin: "top",
            text: "profile = \"everything\"\n",
        };
        let loaded = load(&[user, repository, top])?;
        assert_eq!(loaded.profile, Profile::Everything);
        assert_eq!(
            loaded.config.hints,
            HintsConfig {
                is_runtime_grounding: true,
                is_narrow_tests: false,
                is_redundant_commands: true,
            }
        );
        assert!(!loaded.config.compaction.is_keep_list);
        assert_eq!(loaded.set_by.get(PROFILE_KEY), Some(&2));
        assert_eq!(loaded.set_by.get("hints.narrow_tests"), Some(&0));
        assert_eq!(loaded.set_by.get("compaction.keep_list"), Some(&1));
        assert_eq!(
            loaded.warnings,
            ["repository: unknown key `hints.profile` is ignored"]
        );
        Ok(())
    }

    /// A profile Cortex does not have is refused, naming the file, the key and what it accepts.
    #[test]
    fn an_unknown_profile_is_refused() {
        for text in ["profile = \"heavy\"", "profile = true", "[profile]\nx = 1"] {
            assert_eq!(
                loaded(text).err(),
                Some(ConfigError::Value {
                    origin: "f".to_owned(),
                    key: "profile".to_owned(),
                    expected: "one of standard, lean, everything",
                }),
                "{text}"
            );
        }
    }
}
