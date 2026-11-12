//! The map's settings: the port the daemon serves the map on, and the address that opens a file in
//! the person's editor. The map listens on 127.0.0.1 only, whatever the port.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use super::registry::integer;
use super::{ConfigKey, whole_number};

/// What `ui.port` accepts, in its refusal.
const PORT: &str = "a whole number from 0 to 65535";

/// What `ui.editor_url` accepts, in its refusal.
const EDITOR_URL: &str = "an address of at most 200 characters that holds {path}, starts with an \
    editor's own scheme (such as vscode:) rather than http, https, file, javascript, data, blob or \
    vbscript, and has no space or control character; or nothing, to show no editor link";

/// The editor address the map uses when none is set: Visual Studio Code's, which the editors built
/// on it (Cursor, Windsurf) and several others answer too.
const DEFAULT_EDITOR_URL: &str = "vscode://file{path}:{line}";

/// The longest editor address read, in characters: room for any editor's scheme and its options.
const MAX_EDITOR_URL_CHARS: usize = 200;

/// Schemes an editor address may not use: they open web pages, local files in the browser, or run
/// script, where the map's link is meant only to hand a file to an editor.
const REFUSED_SCHEMES: [&str; 7] = [
    "http",
    "https",
    "file",
    "javascript",
    "data",
    "blob",
    "vbscript",
];

/// The map's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiConfig {
    /// `ui.port`: the port on 127.0.0.1 the map is served on, from 0 to 65535; 0 asks the
    /// operating system for a free one each time the daemon first serves the map. Default 0.
    pub port: u16,
    /// `ui.editor_url`: the address that opens a file in the person's editor, `{path}` standing
    /// for the file's absolute path written as a URL path (`/home/me/repo/a.rs`, or on Windows
    /// `/C:/repo/a.rs`) and `{line}` for a line number; empty shows no editor link.
    /// Default [`DEFAULT_EDITOR_URL`].
    pub editor_url: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            port: 0,
            editor_url: DEFAULT_EDITOR_URL.to_owned(),
        }
    }
}

/// `text` when an editor address may be it: empty, or within [`MAX_EDITOR_URL_CHARS`], holding
/// `{path}`, with no space or control character, and starting with a scheme of a letter then
/// letters, digits, `+`, `-` or `.`, that is not among [`REFUSED_SCHEMES`] in any case.
fn editor_url(text: &str) -> Option<String> {
    if text.is_empty() {
        return Some(String::new());
    }
    if text.chars().count() > MAX_EDITOR_URL_CHARS
        || !text.contains("{path}")
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let (scheme, _) = text.split_once(':')?;
    let mut letters = scheme.chars();
    let is_scheme = letters.next().is_some_and(|c| c.is_ascii_alphabetic())
        && letters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let is_refused = REFUSED_SCHEMES
        .iter()
        .any(|refused| scheme.eq_ignore_ascii_case(refused));
    (is_scheme && !is_refused).then(|| text.to_owned())
}

/// The map's keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[
    ConfigKey::new(
        "ui.port",
        "The port on 127.0.0.1 the map is served on; 0 picks a free one.",
        |config, value| {
            config.ui.port = whole_number(value)
                .and_then(|number| u16::try_from(number).ok())
                .ok_or(PORT)?;
            Ok(())
        },
        |config| integer(u64::from(config.ui.port)),
    ),
    ConfigKey::new(
        "ui.editor_url",
        "The address that opens a file in your editor, {path} and {line} filled in; empty for none.",
        |config, value| {
            config.ui.editor_url = value.as_str().and_then(editor_url).ok_or(EDITOR_URL)?;
            Ok(())
        },
        |config| toml::Value::String(config.ui.editor_url.clone()),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    /// The key named `name` among the map's.
    fn key(name: &str) -> Result<&'static ConfigKey, &'static str> {
        KEYS.iter()
            .find(|key| key.name == name)
            .ok_or("the map has no such key")
    }

    /// A port is a whole number from 0 to 65535: both ends are read, one past either end and a
    /// value of another type are refused with what the key accepts, and the settings stay as they
    /// were.
    #[test]
    fn a_port_is_read_within_its_range() -> Result<(), &'static str> {
        let key = key("ui.port")?;
        for (number, port) in [(0, 0), (65_535, 65_535)] {
            let mut config = Config::default();
            assert_eq!(key.read(&mut config, &toml::Value::Integer(number)), Ok(()));
            assert_eq!(config.ui.port, port);
        }
        for value in [
            toml::Value::Integer(65_536),
            toml::Value::Integer(-1),
            toml::Value::String("8080".to_owned()),
        ] {
            let mut config = Config::default();
            assert_eq!(key.read(&mut config, &value), Err(PORT));
            assert_eq!(config, Config::default());
        }
        Ok(())
    }

    /// An editor address is read when it is an editor's own scheme holding `{path}`, or empty to
    /// show none; a web, file or script scheme, an address without `{path}`, one with a space, a
    /// scheme that does not start with a letter, one of 201 characters and a value of another type
    /// are refused with what the key accepts, the settings as they were.
    #[test]
    fn an_editor_address_is_an_editors_scheme_holding_the_path() -> Result<(), &'static str> {
        let key = key("ui.editor_url")?;
        assert_eq!(
            Config::default().ui.editor_url,
            "vscode://file{path}:{line}"
        );
        for text in [
            "",
            "vscode://file{path}:{line}",
            "idea://open?file={path}&line={line}",
            "x-my.editor+1://{path}",
        ] {
            let mut config = Config::default();
            let value = toml::Value::String(text.to_owned());
            assert_eq!(key.read(&mut config, &value), Ok(()), "{text}");
            assert_eq!(config.ui.editor_url, text);
        }
        // "x://", 190 letters and "{path}" are 4 + 190 + 6 = 200 characters; one more is 201.
        let longest = format!("x://{}{{path}}", "a".repeat(190));
        let too_long = format!("x://{}{{path}}", "a".repeat(191));
        let mut config = Config::default();
        assert_eq!(key.read(&mut config, &toml::Value::String(longest)), Ok(()));
        for value in [
            toml::Value::String("https://example.com/{path}".to_owned()),
            toml::Value::String("HTTP://example.com/{path}".to_owned()),
            toml::Value::String("file:///{path}".to_owned()),
            toml::Value::String("javascript:alert({path})".to_owned()),
            toml::Value::String("data:text/html,{path}".to_owned()),
            toml::Value::String("vscode://file/".to_owned()),
            toml::Value::String("vscode://file{path} :{line}".to_owned()),
            toml::Value::String("1x://{path}".to_owned()),
            toml::Value::String("{path}".to_owned()),
            toml::Value::String(too_long),
            toml::Value::Integer(1),
        ] {
            let mut config = Config::default();
            assert_eq!(key.read(&mut config, &value), Err(EDITOR_URL), "{value}");
            assert_eq!(config, Config::default());
        }
        Ok(())
    }
}
