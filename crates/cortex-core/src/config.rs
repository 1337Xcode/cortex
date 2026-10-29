//! Configuration: the settings a user may change, their defaults, and how files override them.
//!
//! Cortex reads two files, `~/.cortex/config.toml` for the user and `.cortex/config.toml` for the
//! repository; the repository's wins over the user's, which wins over the defaults. The caller
//! reads the files and passes their text here, so this module does no I/O. A key Cortex does not
//! know produces a warning and is otherwise ignored. A known key with a value of the wrong type or
//! out of range is an error that names the file and the key: Cortex never guesses what a broken
//! setting meant.

use thiserror::Error;

use crate::ScaleMode;

/// The default for `index.max_file_bytes`: one mebibyte. Larger files are almost always data or
/// generated code, and parsing them would cost more than they add.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1_048_576;

/// The default for `daemon.idle_shutdown_minutes`: half an hour without any work.
pub const DEFAULT_IDLE_SHUTDOWN_MINUTES: u64 = 30;

/// The default for `store.mmap_bytes`: 256 mebibytes. SQLite reads the mapped part of the
/// database straight from the operating system's file cache instead of copying it into its own.
pub const DEFAULT_MMAP_BYTES: u64 = 268_435_456;

/// The scale mode setting: chosen from the repository's size, or pinned by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSetting {
    /// Choose from the repository's size (`auto`, the default).
    Auto,
    /// Always use this mode.
    Pinned(ScaleMode),
}

/// The settings in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `mode`: `auto`, or `tiny`, `small`, `medium` or `huge` to pin one. Default `auto`.
    pub mode: ModeSetting,
    /// `index.max_file_bytes`: files larger than this are recorded but never read. Default
    /// [`DEFAULT_MAX_FILE_BYTES`].
    pub max_file_bytes: u64,
    /// `index.extra_ignores`: patterns in `.gitignore` syntax that the index also leaves out.
    /// Default none.
    pub extra_ignores: Vec<String>,
    /// `daemon.idle_shutdown_minutes`: the daemon stops after this many minutes without work.
    /// Default [`DEFAULT_IDLE_SHUTDOWN_MINUTES`].
    pub idle_shutdown_minutes: u64,
    /// `store.mmap_bytes`: how much of the database SQLite maps into memory; 0 turns mapping off.
    /// Default [`DEFAULT_MMAP_BYTES`].
    pub mmap_bytes: u64,
}

impl Default for Config {
    /// Every setting at its documented default.
    fn default() -> Self {
        Self {
            mode: ModeSetting::Auto,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            extra_ignores: Vec::new(),
            idle_shutdown_minutes: DEFAULT_IDLE_SHUTDOWN_MINUTES,
            mmap_bytes: DEFAULT_MMAP_BYTES,
        }
    }
}

/// One configuration file's text and a name for it in messages, such as its path.
#[derive(Debug, Clone, Copy)]
pub struct ConfigLayer<'a> {
    /// Where the text came from, shown in warnings and errors.
    pub origin: &'a str,
    /// The file's TOML text.
    pub text: &'a str,
}

/// The settings that result from the layers, with a warning for each key that was ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig {
    /// The settings in effect.
    pub config: Config,
    /// One line per unknown key, naming the file and the key.
    pub warnings: Vec<String>,
}

/// Why a configuration file cannot be used.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// The file is not valid TOML.
    #[error("{origin}: not valid TOML: {message}")]
    Syntax {
        /// The file.
        origin: String,
        /// The parser's description of the problem.
        message: String,
    },
    /// A known key has a value of the wrong type or out of range.
    #[error("{origin}: `{key}` must be {expected}")]
    Value {
        /// The file.
        origin: String,
        /// The key, written with dots.
        key: String,
        /// What the key accepts.
        expected: &'static str,
    },
}

/// The settings that result from `layers`, applied in order so that each overrides the ones
/// before it: pass the user's file first and the repository's last.
///
/// # Errors
///
/// A file that is not valid TOML, or a known key with an unusable value.
pub fn load(layers: &[ConfigLayer<'_>]) -> Result<LoadedConfig, ConfigError> {
    let mut config = Config::default();
    let mut warnings = Vec::new();
    for layer in layers {
        let table: toml::Table =
            layer
                .text
                .parse()
                .map_err(|error: toml::de::Error| ConfigError::Syntax {
                    origin: layer.origin.to_owned(),
                    message: error.message().to_owned(),
                })?;
        let mut entries = Vec::new();
        flatten("", &table, &mut entries);
        for (key, value) in entries {
            if !config.apply(&key, value, layer.origin)? {
                warnings.push(format!("{}: unknown key `{key}` is ignored", layer.origin));
            }
        }
    }
    Ok(LoadedConfig { config, warnings })
}

/// Every value in `table` that is not itself a table, with its key written with dots.
fn flatten<'a>(prefix: &str, table: &'a toml::Table, out: &mut Vec<(String, &'a toml::Value)>) {
    for (name, value) in table {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            toml::Value::Table(inner) => flatten(&key, inner, out),
            other => out.push((key, other)),
        }
    }
}

impl Config {
    /// Sets the key to the value; `Ok(false)` when the key is not one Cortex reads.
    fn apply(&mut self, key: &str, value: &toml::Value, origin: &str) -> Result<bool, ConfigError> {
        let refuse = |expected: &'static str| ConfigError::Value {
            origin: origin.to_owned(),
            key: key.to_owned(),
            expected,
        };
        match key {
            "mode" => {
                self.mode = value
                    .as_str()
                    .and_then(mode_setting)
                    .ok_or_else(|| refuse("one of auto, tiny, small, medium, huge"))?;
            }
            "index.max_file_bytes" => {
                self.max_file_bytes =
                    positive_integer(value).ok_or_else(|| refuse("a whole number above 0"))?;
            }
            "index.extra_ignores" => {
                self.extra_ignores =
                    string_list(value).ok_or_else(|| refuse("a list of strings"))?;
            }
            "daemon.idle_shutdown_minutes" => {
                self.idle_shutdown_minutes =
                    positive_integer(value).ok_or_else(|| refuse("a whole number above 0"))?;
            }
            "store.mmap_bytes" => {
                self.mmap_bytes =
                    whole_number(value).ok_or_else(|| refuse("a whole number, 0 or more"))?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// The mode setting a configured word names.
fn mode_setting(word: &str) -> Option<ModeSetting> {
    if word == "auto" {
        Some(ModeSetting::Auto)
    } else {
        ScaleMode::from_name(word).map(ModeSetting::Pinned)
    }
}

/// The value as a whole number above zero.
fn positive_integer(value: &toml::Value) -> Option<u64> {
    whole_number(value).filter(|number| *number > 0)
}

/// The value as a whole number, zero included.
fn whole_number(value: &toml::Value) -> Option<u64> {
    value
        .as_integer()
        .and_then(|number| u64::try_from(number).ok())
}

/// The value as a list of strings.
fn string_list(value: &toml::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer named `origin` holding `text`.
    fn layer<'a>(origin: &'a str, text: &'a str) -> ConfigLayer<'a> {
        ConfigLayer { origin, text }
    }

    /// With no files, every setting has its documented default.
    #[test]
    fn no_files_give_the_defaults() -> Result<(), ConfigError> {
        let loaded = load(&[])?;
        assert_eq!(
            loaded.config,
            Config {
                mode: ModeSetting::Auto,
                max_file_bytes: 1_048_576,
                extra_ignores: Vec::new(),
                idle_shutdown_minutes: 30,
                mmap_bytes: 256 * 1024 * 1024,
            }
        );
        assert_eq!(loaded.warnings, Vec::<String>::new());
        Ok(())
    }

    /// The repository's file overrides the user's, which overrides the defaults, key by key.
    #[test]
    fn later_layers_override_earlier_ones() -> Result<(), ConfigError> {
        let user = layer(
            "user",
            "mode = \"small\"\n[index]\nmax_file_bytes = 2000\nextra_ignores = [\"*.log\"]\n",
        );
        let repository = layer(
            "repository",
            "[index]\nmax_file_bytes = 5000\n[daemon]\nidle_shutdown_minutes = 5\n[store]\nmmap_bytes = 0\n",
        );
        let loaded = load(&[user, repository])?;
        assert_eq!(
            loaded.config,
            Config {
                mode: ModeSetting::Pinned(ScaleMode::Small),
                max_file_bytes: 5000,
                extra_ignores: vec!["*.log".to_owned()],
                idle_shutdown_minutes: 5,
                mmap_bytes: 0,
            }
        );
        Ok(())
    }

    /// Dotted keys and tables name the same key; unknown keys, at any depth, warn with their file
    /// and change nothing.
    #[test]
    fn unknown_keys_warn() -> Result<(), ConfigError> {
        let loaded = load(&[layer(
            "repo config",
            "index.max_file_bytes = 10\nretrieval.max_tokenz = 3\n[index.extra]\nflavour = \"x\"\n",
        )])?;
        assert_eq!(loaded.config.max_file_bytes, 10);
        assert_eq!(
            loaded.warnings,
            vec![
                "repo config: unknown key `index.extra.flavour` is ignored".to_owned(),
                "repo config: unknown key `retrieval.max_tokenz` is ignored".to_owned(),
            ]
        );
        Ok(())
    }

    /// A known key with an unusable value is an error naming the file, the key and what it
    /// accepts.
    #[test]
    fn bad_values_are_refused() {
        let cases = [
            (
                "mode = \"gigantic\"",
                "mode",
                "one of auto, tiny, small, medium, huge",
            ),
            ("mode = 3", "mode", "one of auto, tiny, small, medium, huge"),
            (
                "index.max_file_bytes = 0",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.max_file_bytes = -5",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.max_file_bytes = \"1MB\"",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.extra_ignores = [\"a\", 2]",
                "index.extra_ignores",
                "a list of strings",
            ),
            (
                "daemon.idle_shutdown_minutes = 0",
                "daemon.idle_shutdown_minutes",
                "a whole number above 0",
            ),
            (
                "store.mmap_bytes = -1",
                "store.mmap_bytes",
                "a whole number, 0 or more",
            ),
            (
                "store.mmap_bytes = \"256MB\"",
                "store.mmap_bytes",
                "a whole number, 0 or more",
            ),
        ];
        for (text, key, expected) in cases {
            assert_eq!(
                load(&[layer("f", text)]),
                Err(ConfigError::Value {
                    origin: "f".to_owned(),
                    key: key.to_owned(),
                    expected,
                }),
                "{text}"
            );
        }
    }

    /// A file that is not TOML is an error naming the file.
    #[test]
    fn broken_files_are_refused() {
        let error = load(&[layer("user config", "mode = \"auto")]).err();
        assert!(
            matches!(&error, Some(ConfigError::Syntax { origin, .. }) if origin == "user config"),
            "{error:?}"
        );
    }
}
