//! The switches: configuration keys whose value is whether a file is in the user's own folder,
//! not a line of a configuration file, so that a hook reads one with a single look at a file's
//! metadata and parses no configuration. The file is the only place the value is kept: `cortex
//! config` shows a switch as the file says, and `cortex config set` writes or removes the file.
//! A configuration file that names a switch's key is warned of as naming a key Cortex does not
//! read there.

use std::path::PathBuf;

use crate::UserFolder;

/// One switch: its key, what it does, and the file whose presence turns it on.
#[derive(Debug, Clone, Copy)]
pub struct SwitchKey {
    /// The key, written with dots: `adopt.enabled`.
    pub name: &'static str,
    /// What the switch does, in one line.
    pub description: &'static str,
    /// The file, in the user's folder, that turns it on.
    file: fn(&UserFolder) -> PathBuf,
}

impl SwitchKey {
    /// The file, in the user's folder `user`, whose presence turns the switch on.
    #[must_use]
    pub fn file(&self, user: &UserFolder) -> PathBuf {
        (self.file)(user)
    }

    /// Whether the switch is on for the user whose folder is `user`.
    #[must_use]
    pub fn is_on(&self, user: &UserFolder) -> bool {
        self.file(user).is_file()
    }
}

/// Every switch.
pub const SWITCH_KEYS: &[SwitchKey] = &[SwitchKey {
    name: "adopt.enabled",
    description: "Set up on its own each git repository below the home folder that an agent works \
                  in (cortex init --global turns it on).",
    file: UserFolder::adopt_path,
}];

/// The switch named `name`, written with dots, when there is one.
#[must_use]
pub fn switch_key(name: &str) -> Option<&'static SwitchKey> {
    SWITCH_KEYS.iter().find(|switch| switch.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `adopt.enabled` is the file `~/.cortex/adopt`: on while it is there, off once it is gone. No
    /// switch has the name of a key of the configuration's files.
    #[test]
    fn adoption_is_the_file_s_presence() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let user = UserFolder::in_home(home.path());
        let switch = switch_key("adopt.enabled").ok_or(std::io::ErrorKind::NotFound)?;
        assert_eq!(
            switch.file(&user),
            home.path().join(".cortex").join("adopt")
        );
        assert!(!switch.is_on(&user));
        std::fs::create_dir_all(user.path())?;
        std::fs::write(user.adopt_path(), "")?;
        assert!(switch.is_on(&user));
        for switch in SWITCH_KEYS {
            assert!(
                super::super::config_key(switch.name).is_none(),
                "{}",
                switch.name
            );
        }
        Ok(())
    }
}
