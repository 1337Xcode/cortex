//! The settings in effect, key by key, each with where its value came from: the default, the
//! profile, or a file.

use super::profile::PROFILE_DESCRIPTION;
use super::{LoadedConfig, PROFILE_KEY, config_keys};

/// Where a setting's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSource {
    /// Its documented default.
    Default,
    /// The profile in effect.
    Profile,
    /// The configuration file at this position in the layers given to [`super::load`].
    Layer(usize),
}

/// One key's value in effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// The key, written with dots.
    pub key: &'static str,
    /// What the key changes, in one line.
    pub description: &'static str,
    /// Its value, as a file would set it.
    pub value: toml::Value,
    /// Where the value came from.
    pub source: SettingSource,
}

impl LoadedConfig {
    /// Every key Cortex reads, `profile` included, with its value in effect and where that came
    /// from, in the order of the keys' names.
    #[must_use]
    pub fn settings(&self) -> Vec<Setting> {
        let profile_keys = self.profile.keys();
        let source_of = |name: &str| match self.set_by.get(name) {
            Some(&position) => SettingSource::Layer(position),
            None if profile_keys.iter().any(|(key, _)| *key == name) => SettingSource::Profile,
            None => SettingSource::Default,
        };
        let mut settings = vec![Setting {
            key: PROFILE_KEY,
            description: PROFILE_DESCRIPTION,
            value: toml::Value::String(self.profile.name().to_owned()),
            source: source_of(PROFILE_KEY),
        }];
        settings.extend(config_keys().map(|key| Setting {
            key: key.name,
            description: key.description,
            value: key.show(&self.config),
            source: source_of(key.name),
        }));
        settings.sort_by_key(|setting| setting.key);
        settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigError, ConfigLayer, load};

    /// The setting of `key` among `settings`.
    fn setting<'a>(settings: &'a [Setting], key: &str) -> Option<&'a Setting> {
        settings.iter().find(|setting| setting.key == key)
    }

    /// Each key shows the layer that set it last, the profile for a switch it set and no file did,
    /// and the default otherwise; the list is in the order of the keys and holds each key once.
    #[test]
    fn each_setting_names_its_source() -> Result<(), ConfigError> {
        let user = ConfigLayer {
            origin: "user",
            text: "profile = \"everything\"\nretrieval.max_files = 7\nholdout.share = 0\n",
        };
        let repository = ConfigLayer {
            origin: "repository",
            text: "[retrieval]\nmax_files = 9\n[hints]\nnarrow_tests = false\n",
        };
        let settings = load(&[user, repository])?.settings();
        let expected = [
            ("profile", "\"everything\"", SettingSource::Layer(0)),
            ("holdout.share", "0.0", SettingSource::Layer(0)),
            ("retrieval.max_files", "9", SettingSource::Layer(1)),
            ("hints.narrow_tests", "false", SettingSource::Layer(1)),
            ("hints.runtime_grounding", "true", SettingSource::Profile),
            ("compaction.keep_list", "true", SettingSource::Profile),
            ("retrieval.max_tokens", "900", SettingSource::Default),
            ("mode", "\"auto\"", SettingSource::Default),
        ];
        for (key, value, source) in expected {
            let found = setting(&settings, key).map(|each| (each.value.to_string(), each.source));
            assert_eq!(found, Some((value.to_owned(), source)), "{key}");
        }
        let keys: Vec<&str> = settings.iter().map(|each| each.key).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(keys, sorted);
        assert_eq!(keys.len(), config_keys().count() + 1);
        Ok(())
    }
}
