//! Setting one key in a configuration file's text: the key and its value are checked as loading
//! checks them, then the value is written into the text with the file's other keys, tables,
//! comments and layout kept as they were.

use thiserror::Error;

use super::edit::placed;
use super::{
    Config, ConfigError, ConfigKey, ConfigLayer, LayerKind, PROFILE_KEY, Profile, config_key,
    config_keys,
};

/// Why a key cannot be set in a file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SetError {
    /// Cortex reads no key of that name.
    #[error("unknown key `{key}`; the nearest key Cortex reads is `{nearest}`")]
    UnknownKey {
        /// The key asked for.
        key: String,
        /// The known key nearest to it by edit distance.
        nearest: &'static str,
    },
    /// The value is one the key does not accept.
    #[error("`{key}` must be {expected}")]
    Value {
        /// The key.
        key: &'static str,
        /// What the key accepts.
        expected: &'static str,
    },
    /// The key is read only from the user's own file, and the file is a repository's.
    #[error("`{key}` is read only from the user's own file; set it with --user")]
    UserOnly {
        /// The key.
        key: &'static str,
    },
    /// The file is not valid TOML.
    #[error(transparent)]
    File(#[from] ConfigError),
    /// The value cannot be written into the file without changing something else it says, as
    /// when the key's current value spans several lines.
    #[error(
        "{origin}: cannot set `{key}` without changing anything else it says; set it there by hand"
    )]
    CannotPlace {
        /// The file.
        origin: String,
        /// The key.
        key: &'static str,
    },
}

/// The value a command line's `text` stands for: the TOML value it reads as (`5`, `true`, `0.5`,
/// `["*.log"]`, `"small"`), and otherwise the text itself as a string, so that `small` needs no
/// quotes.
#[must_use]
pub fn setting_value(text: &str) -> toml::Value {
    text.parse()
        .unwrap_or_else(|_| toml::Value::String(text.to_owned()))
}

/// The text of the configuration file `file` with `key` set to `value`, every other key, table,
/// comment and line of it kept. When the key's line is added rather than changed, `note`, a line of
/// text, goes above it (or above its new table) as a comment, saying who wrote it.
///
/// # Errors
///
/// A key Cortex does not read (naming the nearest one), a value the key does not accept (in the
/// words loading refuses it with), a key only the user's own file may set in a repository's file,
/// a file that is not TOML, or a file the value cannot be written into without changing anything
/// else it says.
pub fn with_setting(
    file: &ConfigLayer<'_>,
    key: &str,
    value: &toml::Value,
    note: Option<&str>,
) -> Result<String, SetError> {
    let name = checked(key, value)?;
    if file.kind == LayerKind::Repository && config_key(name).is_some_and(ConfigKey::is_user_only) {
        return Err(SetError::UserOnly { key: name });
    }
    placed(file, name, value, note)
}

/// The name of `key`, after checking that Cortex reads it and accepts `value` for it.
///
/// # Errors
///
/// The key is unknown, or the value one it does not accept.
fn checked(key: &str, value: &toml::Value) -> Result<&'static str, SetError> {
    if key == PROFILE_KEY {
        return Profile::read(value)
            .map(|_| PROFILE_KEY)
            .map_err(|expected| SetError::Value {
                key: PROFILE_KEY,
                expected,
            });
    }
    let Some(known) = config_key(key) else {
        return Err(SetError::UnknownKey {
            key: key.to_owned(),
            nearest: nearest(key),
        });
    };
    known
        .read(&mut Config::default(), value)
        .map(|()| known.name)
        .map_err(|expected| SetError::Value {
            key: known.name,
            expected,
        })
}

/// The known key nearest to `key` by edit distance, the first by name among equals.
fn nearest(key: &str) -> &'static str {
    let mut names: Vec<&'static str> = config_keys().map(|known| known.name).collect();
    names.push(PROFILE_KEY);
    names.sort_unstable();
    names
        .into_iter()
        .min_by_key(|name| edit_distance(key, name))
        .unwrap_or(PROFILE_KEY)
}

/// The fewest one-character insertions, deletions and substitutions that turn `from` into `to`.
fn edit_distance(from: &str, to: &str) -> usize {
    let to: Vec<char> = to.chars().collect();
    let mut previous: Vec<usize> = (0..=to.len()).collect();
    for (row, from_char) in from.chars().enumerate() {
        let mut current = Vec::with_capacity(previous.len());
        current.push(row + 1);
        for (column, to_char) in to.iter().enumerate() {
            let substitution = previous
                .get(column)
                .map_or(usize::MAX, |cost| cost + usize::from(from_char != *to_char));
            let deletion = previous.get(column + 1).map_or(usize::MAX, |cost| cost + 1);
            let insertion = current.get(column).map_or(usize::MAX, |cost| cost + 1);
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file named `f` holding `text`.
    fn file(text: &str) -> ConfigLayer<'_> {
        ConfigLayer {
            kind: LayerKind::User,
            origin: "f",
            text,
        }
    }

    /// Edit distances counted by hand: one substitution, one insertion, one deletion, a whole word.
    #[test]
    fn edit_distances_are_counted() {
        assert_eq!(edit_distance("max_filez", "max_files"), 1);
        assert_eq!(edit_distance("mode", "modes"), 1);
        assert_eq!(edit_distance("modes", "mode"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    /// An unknown key names the nearest known one; a misspelt profile is caught too.
    #[test]
    fn an_unknown_key_names_the_nearest() {
        let value = toml::Value::Integer(5);
        assert_eq!(
            with_setting(&file(""), "retrieval.max_filez", &value, None),
            Err(SetError::UnknownKey {
                key: "retrieval.max_filez".to_owned(),
                nearest: "retrieval.max_files",
            })
        );
        assert_eq!(
            with_setting(&file(""), "profiles", &value, None).map_err(|error| error.to_string()),
            Err("unknown key `profiles`; the nearest key Cortex reads is `profile`".to_owned())
        );
    }

    /// A value the key does not accept is refused in the words loading uses.
    #[test]
    fn a_bad_value_is_refused_as_loading_refuses_it() {
        let refused = |key: &str, text: &str| {
            with_setting(&file(""), key, &setting_value(text), None)
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            refused("retrieval.max_files", "21"),
            Err("`retrieval.max_files` must be a whole number from 1 to 20".to_owned())
        );
        assert_eq!(
            refused("mode", "gigantic"),
            Err("`mode` must be one of auto, tiny, small, medium, huge".to_owned())
        );
        assert_eq!(
            refused("profile", "heavy"),
            Err("`profile` must be one of standard, lean, everything".to_owned())
        );
        assert_eq!(
            refused("hints.narrow_tests", "yes"),
            Err("`hints.narrow_tests` must be true or false".to_owned())
        );
    }

    /// A key only the user's file may set is refused in a repository's file, with the flag that
    /// writes the user's, and written in the user's; other keys are written in either.
    #[test]
    fn a_user_only_key_is_written_only_in_the_users_file() {
        let repository = ConfigLayer {
            kind: LayerKind::Repository,
            origin: "r",
            text: "",
        };
        let value = toml::Value::Boolean(true);
        assert_eq!(
            with_setting(&repository, "privacy.store_prompt_excerpt", &value, None)
                .map_err(|error| error.to_string()),
            Err(
                "`privacy.store_prompt_excerpt` is read only from the user's own file; set it \
                 with --user"
                    .to_owned()
            )
        );
        assert_eq!(
            with_setting(&file(""), "privacy.store_prompt_excerpt", &value, None),
            Ok("[privacy]\nstore_prompt_excerpt = true\n".to_owned())
        );
        assert_eq!(
            with_setting(&repository, "hints.narrow_tests", &value, None),
            Ok("[hints]\nnarrow_tests = true\n".to_owned())
        );
    }

    /// A command line's text reads as the TOML value it writes, and a bare word as a string.
    #[test]
    fn command_line_values_are_read() {
        assert_eq!(setting_value("5"), toml::Value::Integer(5));
        assert_eq!(setting_value("0.5"), toml::Value::Float(0.5));
        assert_eq!(setting_value("true"), toml::Value::Boolean(true));
        assert_eq!(
            setting_value("small"),
            toml::Value::String("small".to_owned())
        );
        assert_eq!(
            setting_value("\"small\""),
            toml::Value::String("small".to_owned())
        );
        assert_eq!(
            setting_value("[\"*.log\"]"),
            toml::Value::Array(vec![toml::Value::String("*.log".to_owned())])
        );
        assert_eq!(
            setting_value("5\nmode = 1"),
            toml::Value::String("5\nmode = 1".to_owned())
        );
    }
}
