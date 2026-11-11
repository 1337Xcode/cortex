//! The adoption settings: which folders Cortex never sets up on its own once adoption is on
//! (`cortex init --global`). They are read where adoption is decided, by the daemon started for
//! the folder, never by a hook. Whether adoption is on is no line of a file but a switch (see
//! [`super::switches`]).
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use std::path::Path;

use globset::{Glob, GlobBuilder};

use super::registry::strings;
use super::{ConfigKey, string_list};

/// What `adopt.exclude` accepts, in its refusal.
const GLOBS: &str = "a list of glob patterns, such as \"~/Downloads/**\"";

/// A home folder for checking a pattern when it is read: any one that is text will do, since a
/// pattern that compiles against one compiles against all.
const ANY_HOME: &str = "/home";

/// The settings of setting folders up on their own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptConfig {
    /// `adopt.exclude`: glob patterns of folders never set up on their own, with `/` between
    /// folders and `~` standing for the home folder at a pattern's start. A folder is left alone
    /// when a pattern matches it or any folder above it. Default none.
    pub exclude: Vec<String>,
}

impl AdoptConfig {
    /// The first pattern of `adopt.exclude` that matches `folder` or a folder above it, `home`
    /// being the home folder `~` stands for; none when none does. A pattern that cannot be read
    /// against this home folder (one starting with `~`, when the home folder's path is not text)
    /// leaves every folder out: a pattern never matches nothing in silence.
    #[must_use]
    pub fn excluding(&self, folder: &Path, home: &Path) -> Option<&str> {
        self.excluding_against(folder, home.to_str())
    }

    /// [`AdoptConfig::excluding`], `home` being the home folder's path as text, or none when it
    /// is not text.
    fn excluding_against(&self, folder: &Path, home: Option<&str>) -> Option<&str> {
        self.exclude.iter().map(String::as_str).find(|pattern| {
            compiled(pattern, home).is_none_or(|glob| {
                let matcher = glob.compile_matcher();
                folder.ancestors().any(|above| matcher.is_match(above))
            })
        })
    }
}

/// `pattern` as a glob, a leading `~` standing for `home`, the home folder's path as text: none
/// when the pattern is no glob, or starts with `~` and there is no such text. On Windows a
/// backslash is a folder's separator, as in the paths matched, not an escape. Folders and file
/// names are compared in any case where the file system usually ignores it.
fn compiled(pattern: &str, home: Option<&str>) -> Option<Glob> {
    let pattern = match pattern.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            format!("{}{rest}", globset::escape(home?))
        }
        _ => pattern.to_owned(),
    };
    let pattern = if cfg!(windows) {
        pattern.replace('\\', "/")
    } else {
        pattern
    };
    GlobBuilder::new(&pattern)
        .literal_separator(true)
        .case_insensitive(cfg!(any(windows, target_os = "macos")))
        .build()
        .ok()
}

/// The adoption keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[ConfigKey::user_only(
    "adopt.exclude",
    "Glob patterns of folders Cortex never sets up on its own, ~ standing for the home folder; a \
     match also leaves out every folder below it. Read only from the user's own file.",
    |config, value| {
        let patterns = string_list(value).ok_or(GLOBS)?;
        if !patterns
            .iter()
            .all(|pattern| compiled(pattern, Some(ANY_HOME)).is_some())
        {
            return Err(GLOBS);
        }
        config.adopt.exclude = patterns;
        Ok(())
    },
    |config| strings(&config.adopt.exclude),
)];

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The settings with `patterns` as `adopt.exclude`.
    fn excluding(patterns: &[&str]) -> AdoptConfig {
        AdoptConfig {
            exclude: patterns
                .iter()
                .map(|pattern| (*pattern).to_owned())
                .collect(),
        }
    }

    /// A home folder in this platform's spelling.
    fn home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\dev")
        } else {
            PathBuf::from("/home/dev")
        }
    }

    /// `~/Downloads` leaves out that folder and every folder below it; `~/Downloads/*` only the
    /// folders in it; `**/vendor` any folder named `vendor` and what is below it. A folder beside
    /// them, or one whose name only starts the same, is not left out.
    #[test]
    fn a_pattern_leaves_out_its_folder_and_below() {
        let home = home();
        let downloads = home.join("Downloads");
        let config = excluding(&["~/Downloads"]);
        assert_eq!(config.excluding(&downloads, &home), Some("~/Downloads"));
        assert_eq!(
            config.excluding(&downloads.join("a").join("b"), &home),
            Some("~/Downloads")
        );
        assert_eq!(config.excluding(&home.join("Downloads2"), &home), None);
        assert_eq!(config.excluding(&home.join("code"), &home), None);
        let inside = excluding(&["~/Downloads/*"]);
        assert_eq!(inside.excluding(&downloads, &home), None);
        assert_eq!(
            inside.excluding(&downloads.join("clone"), &home),
            Some("~/Downloads/*")
        );
        let vendor = excluding(&["**/vendor"]);
        assert_eq!(
            vendor.excluding(&home.join("code").join("vendor").join("x"), &home),
            Some("**/vendor")
        );
        assert_eq!(vendor.excluding(&home.join("code"), &home), None);
    }

    /// A pattern starting with `~` cannot be read without the home folder's path as text: it is
    /// no glob then, and leaves every folder out, so that a home folder whose path is not text
    /// refuses every folder rather than none. A pattern without `~` needs no home folder.
    #[test]
    fn a_home_pattern_needs_the_home_folder_as_text() {
        assert!(compiled("~/Downloads", None).is_none());
        assert!(compiled("~", None).is_none());
        assert!(compiled("**/vendor", None).is_some());
        assert!(compiled("~/Downloads", Some("/home/dev")).is_some());
        let folder = home().join("code").join("repo");
        let config = excluding(&["**/vendor", "~/Downloads"]);
        assert_eq!(config.excluding_against(&folder, None), Some("~/Downloads"));
        assert_eq!(
            excluding(&["**/vendor"]).excluding_against(&folder, None),
            None
        );
    }

    /// A pattern that is no glob is refused when read, with what the key accepts; a list of
    /// globs is kept as written.
    #[test]
    fn a_pattern_that_is_no_glob_is_refused() {
        let mut config = super::super::Config::default();
        let key = &KEYS[0];
        let bad = toml::Value::Array(vec![toml::Value::String("a[".to_owned())]);
        assert_eq!(key.read(&mut config, &bad), Err(GLOBS));
        assert_eq!(config.adopt, AdoptConfig::default());
        let good = toml::Value::Array(vec![toml::Value::String("~/x/**".to_owned())]);
        assert_eq!(key.read(&mut config, &good), Ok(()));
        assert_eq!(config.adopt.exclude, ["~/x/**"]);
    }
}
