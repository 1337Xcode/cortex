//! Writing one key into a configuration file Cortex reads. The configuration's registry checks the
//! key and the value and places the value in the file's text without touching its other lines;
//! then the file is replaced through a new file renamed over it. The first time Cortex changes a
//! file, once the change is made, what the file held before is kept beside it as
//! `config.toml.cortex-backup`, a name of Cortex's own; later changes keep that first copy.
//!
//! Only the two files Cortex reads are written. The repository's only in a `.cortex` folder that
//! is the repository's own: a repository Cortex was never set up in is refused, and so is a
//! `.cortex` that is a link or leads elsewhere, as a cloned repository's link to the user's own
//! folder would. The user's file is in the user's own folder, followed through a link as every
//! user file Cortex writes is. A link or a folder where the file or its copy should be is refused.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{
    ConfigLayer, MAX_SETTINGS_BYTES, SwitchKey, UserFolder, read_text_file, setting_value,
    switch_key, with_setting,
};
use cortex_daemon::settings::Place;

use crate::backup::replace_unchanged;
use crate::commands::init::own_folder;
use crate::output;

/// The extension of the copy Cortex keeps of a configuration file before its first change.
const BACKUP_EXTENSION: &str = "toml.cortex-backup";

/// Sets `key` to the value `value_text` stands for in the configuration file of `place` for the
/// repository at `root`, and says what it wrote.
///
/// # Errors
///
/// No home folder for the user's file, a folder Cortex has not made or that leads elsewhere, a
/// link or folder where the file or its copy should be, a file that cannot be read, a key or value
/// the configuration refuses, a file the value cannot be placed in without changing anything else
/// it says, or a file or copy that cannot be written.
pub(crate) fn set(root: &Path, place: Place, key: &str, value_text: &str) -> anyhow::Result<()> {
    if let Some(switch) = switch_key(key) {
        return set_switch(switch, value_text);
    }
    let path = file_of(root, place)?;
    let backup = path.with_extension(BACKUP_EXTENSION);
    refuse_unless_file(&path)?;
    refuse_unless_file(&backup)?;
    let before = read_text_file(&path, MAX_SETTINGS_BYTES)?;
    let origin = path.display().to_string();
    let file = ConfigLayer {
        origin: &origin,
        text: before.as_deref().unwrap_or_default(),
    };
    let value = setting_value(value_text);
    let text = with_setting(&file, key, &value, None)?;
    replace_unchanged(&path, before.as_deref().map(str::as_bytes), text.as_bytes())?;
    output::line(&format!("set {key} = {value} in {}", path.display()))?;
    let Some(before) = before else {
        return Ok(());
    };
    if fs::symlink_metadata(&backup).is_ok() {
        return Ok(());
    }
    replace_unchanged(&backup, None, before.as_bytes()).with_context(|| {
        format!(
            "{} was changed, but the copy of what it held before could not be kept",
            path.display()
        )
    })?;
    output::line(&format!(
        "kept what {} held before as {}",
        path.display(),
        backup.display()
    ))
}

/// Turns `switch` on or off as `value_text`, `true` or `false`, says, by writing or removing its
/// file in the user's folder, whichever file was named: a switch is the user's alone.
///
/// # Errors
///
/// A value other than `true` or `false`, no home folder, or a file that cannot be written or
/// removed.
fn set_switch(switch: &SwitchKey, value_text: &str) -> anyhow::Result<()> {
    let is_on = match value_text.trim() {
        "true" => true,
        "false" => false,
        _ => bail!("`{}` takes true or false", switch.name),
    };
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let file = switch.file(&user);
    crate::adoption::switch(&file, is_on)?;
    output::line(&format!(
        "set {} = {is_on} ({} {})",
        switch.name,
        if is_on { "wrote" } else { "removed" },
        file.display()
    ))
}

/// The configuration file of `place` for the repository at `root`, in a folder that exists and,
/// for the repository's, is its own `.cortex` folder.
///
/// # Errors
///
/// No home folder, a folder that does not exist, or a repository folder that is a link or leads
/// elsewhere.
fn file_of(root: &Path, place: Place) -> anyhow::Result<PathBuf> {
    let path = place
        .path(root)
        .context("no home folder to find the user's configuration in")?;
    let folder = path
        .parent()
        .with_context(|| format!("{} is in no folder", path.display()))?;
    match fs::symlink_metadata(folder) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => bail!(
            "{} does not exist: Cortex has not been set up here, so there is no configuration to \
             change (run `cortex init` first)",
            folder.display()
        ),
        Err(error) => return Err(error).with_context(|| format!("finding {}", folder.display())),
    }
    match place {
        // The folder exists, so this refuses a link or a folder leading elsewhere, and makes
        // nothing.
        Place::Repository => Ok(own_folder(root, ".cortex")?.join("config.toml")),
        Place::User if folder.is_dir() => Ok(path),
        Place::User => bail!("{} is not a folder", folder.display()),
    }
}

/// Refuses `path` when something other than a file is there, a link included: Cortex writes only
/// a file it found as a file, or makes one where there is nothing.
///
/// # Errors
///
/// Something other than a file is at `path`, or it cannot be looked at.
fn refuse_unless_file(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => bail!(
            "{} is not a plain file (a link or a folder): Cortex does not write through it",
            path.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("looking at {}", path.display())),
    }
}
