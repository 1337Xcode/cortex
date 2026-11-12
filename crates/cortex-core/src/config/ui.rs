//! The map's settings: the port the daemon serves the map on. The map listens on 127.0.0.1 only,
//! whatever the port.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use super::registry::integer;
use super::{ConfigKey, whole_number};

/// What `ui.port` accepts, in its refusal.
const PORT: &str = "a whole number from 0 to 65535";

/// The map's settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiConfig {
    /// `ui.port`: the port on 127.0.0.1 the map is served on, from 0 to 65535; 0 asks the
    /// operating system for a free one each time the daemon first serves the map. Default 0.
    pub port: u16,
}

/// The map's keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[ConfigKey::new(
    "ui.port",
    "The port on 127.0.0.1 the map is served on; 0 picks a free one.",
    |config, value| {
        config.ui.port = whole_number(value)
            .and_then(|number| u16::try_from(number).ok())
            .ok_or(PORT)?;
        Ok(())
    },
    |config| integer(u64::from(config.ui.port)),
)];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    /// A port is a whole number from 0 to 65535: both ends are read, one past either end and a
    /// value of another type are refused with what the key accepts, and the settings stay as they
    /// were.
    #[test]
    fn a_port_is_read_within_its_range() -> Result<(), &'static str> {
        let key = KEYS.first().ok_or("the map has no key")?;
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
}
