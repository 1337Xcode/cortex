//! The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the
//! repository's `.cortex/config.toml`, each read when it exists, the repository's winning.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, bail};
use cortex_core::{ConfigLayer, LoadedConfig, UserFolder, load_config};

/// The largest configuration file read. A larger one is refused instead of being read whole.
const MAX_CONFIG_BYTES: u64 = 1_048_576;

/// The settings for the repository at `root`, with a warning for each unknown key.
///
/// # Errors
///
/// A file that cannot be read, is over 1 MiB, is not valid TOML or gives a known key an unusable
/// value.
pub(crate) fn load(root: &Path) -> anyhow::Result<LoadedConfig> {
    let user = UserFolder::of_current_user().map(|folder| folder.config_path());
    let repository = root.join(".cortex").join("config.toml");
    let mut texts = Vec::new();
    for path in user.iter().chain([&repository]) {
        if let Some(text) = read(path)? {
            texts.push((path.display().to_string(), text));
        }
    }
    let layers: Vec<ConfigLayer<'_>> = texts
        .iter()
        .map(|(origin, text)| ConfigLayer { origin, text })
        .collect();
    Ok(load_config(&layers)?)
}

/// The text of the configuration file at `path`, or `None` when there is none.
fn read(path: &Path) -> anyhow::Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("opening {}", path.display())),
    };
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .with_context(|| format!("reading {}", path.display()))?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_CONFIG_BYTES {
        bail!(
            "{} is over 1 MiB, too large for a configuration file",
            path.display()
        );
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing file is no file; a file of exactly 1 MiB is read whole, and one byte more is
    /// refused.
    #[test]
    fn missing_and_oversized_files() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("config.toml");
        assert_eq!(read(&path)?, None);
        std::fs::write(&path, "#".repeat(1_048_576))?;
        assert_eq!(read(&path)?.map(|text| text.len()), Some(1_048_576));
        std::fs::write(&path, "#".repeat(1_048_577))?;
        assert!(read(&path).is_err());
        Ok(())
    }
}
