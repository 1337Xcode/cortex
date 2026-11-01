//! The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the
//! repository's `.cortex/config.toml`, each read when it exists, the repository's winning.

use std::path::Path;

use cortex_core::{
    ConfigLayer, LoadedConfig, MAX_SETTINGS_BYTES, UserFolder, load_config, read_text_file,
};

/// The settings for the repository at `root`, with a warning for each unknown key.
///
/// # Errors
///
/// A file that cannot be read, is over 1 MiB, is not UTF-8 or valid TOML, or gives a known key an
/// unusable value.
pub fn load(root: &Path) -> anyhow::Result<LoadedConfig> {
    let user = UserFolder::of_current_user().map(|folder| (folder.config_path(), true));
    let repository = (root.join(".cortex").join("config.toml"), false);
    let mut texts = Vec::new();
    for (path, is_users) in user.into_iter().chain([repository]) {
        if let Some(text) = read_text_file(&path, MAX_SETTINGS_BYTES)? {
            texts.push((path.display().to_string(), text, is_users));
        }
    }
    let layers: Vec<ConfigLayer<'_>> = texts
        .iter()
        .map(|(origin, text, is_users)| ConfigLayer {
            origin,
            text,
            is_users: *is_users,
        })
        .collect();
    Ok(load_config(&layers)?)
}
