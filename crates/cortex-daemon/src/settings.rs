//! The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the
//! repository's `.cortex/config.toml`, each read when it exists, the repository's winning.

use std::path::{Path, PathBuf};

use cortex_core::{
    ConfigLayer, LayerKind, LoadedConfig, MAX_SETTINGS_BYTES, UserFolder, load_config,
    read_text_file,
};

/// Which of the two configuration files a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The user's own, `~/.cortex/config.toml`, for every repository.
    User,
    /// The repository's, `.cortex/config.toml`.
    Repository,
}

impl Place {
    /// The place's word: `user` or `repository`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Repository => "repository",
        }
    }

    /// Whose file a configuration layer read from this place is.
    #[must_use]
    pub fn kind(self) -> LayerKind {
        match self {
            Self::User => LayerKind::User,
            Self::Repository => LayerKind::Repository,
        }
    }

    /// The configuration file of this place for the repository at `root`; `None` for the user's
    /// when the operating system reports no home folder.
    #[must_use]
    pub fn path(self, root: &Path) -> Option<PathBuf> {
        match self {
            Self::User => UserFolder::of_current_user().map(|folder| folder.config_path()),
            Self::Repository => Some(root.join(".cortex").join("config.toml")),
        }
    }
}

/// A configuration file that exists, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    /// Which file it is.
    pub place: Place,
    /// Where it is.
    pub path: PathBuf,
    /// Its text.
    pub text: String,
}

/// The configuration files of the repository at `root` that exist, the user's first.
///
/// # Errors
///
/// A file that cannot be read, is over 1 MiB or is not UTF-8.
pub fn read_files(root: &Path) -> anyhow::Result<Vec<ConfigFile>> {
    let mut files = Vec::new();
    for place in [Place::User, Place::Repository] {
        let Some(path) = place.path(root) else {
            continue;
        };
        if let Some(text) = read_text_file(&path, MAX_SETTINGS_BYTES)? {
            files.push(ConfigFile { place, path, text });
        }
    }
    Ok(files)
}

/// The settings `files` give, applied in order, with a warning for each unknown key: a source
/// position in the result is a position in `files`.
///
/// # Errors
///
/// A file that is not valid TOML or gives a known key an unusable value.
pub fn load_files(files: &[ConfigFile]) -> anyhow::Result<LoadedConfig> {
    let origins: Vec<String> = files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect();
    let layers: Vec<ConfigLayer<'_>> = files
        .iter()
        .zip(&origins)
        .map(|(file, origin)| ConfigLayer {
            kind: file.place.kind(),
            origin,
            text: &file.text,
        })
        .collect();
    Ok(load_config(&layers)?)
}

/// The settings for the repository at `root`, with a warning for each unknown key.
///
/// # Errors
///
/// A file that cannot be read, is over 1 MiB, is not UTF-8 or valid TOML, or gives a known key an
/// unusable value.
pub fn load(root: &Path) -> anyhow::Result<LoadedConfig> {
    load_files(&read_files(root)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration file of `place` holding `text`.
    fn file(place: Place, text: &str) -> ConfigFile {
        ConfigFile {
            place,
            path: PathBuf::from(place.name()),
            text: text.to_owned(),
        }
    }

    /// A repository's file that turns on `privacy.store_prompt_excerpt` leaves prompt excerpts off
    /// and is warned of by its path; the user's file turns them on, even under a repository file
    /// that says otherwise.
    #[test]
    fn only_the_users_file_turns_prompt_excerpts_on() -> anyhow::Result<()> {
        let on = "privacy.store_prompt_excerpt = true\n";
        let from_repository = load_files(&[file(Place::Repository, on)])?;
        assert!(!from_repository.config.store_prompt_excerpt);
        assert_eq!(
            from_repository.warnings,
            [
                "repository: `privacy.store_prompt_excerpt` is read only from the user's file; this \
              value is ignored"
            ]
        );
        let from_user = load_files(&[
            file(Place::User, on),
            file(Place::Repository, "privacy.store_prompt_excerpt = false\n"),
        ])?;
        assert!(from_user.config.store_prompt_excerpt);
        assert_eq!(from_user.warnings.len(), 1);
        Ok(())
    }
}
