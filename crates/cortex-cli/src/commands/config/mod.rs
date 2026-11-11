//! `cortex config`: every configuration key with its value in effect and where the value came from
//! (`default`, `profile`, `user` for `~/.cortex/config.toml`, `repository` for the repository's
//! `.cortex/config.toml`), in the order of the keys' names, as text or as JSON; and `cortex config
//! set <key> <value>`, which writes one key into the repository's file, or the user's with
//! `--user`. The configuration's own registry reads and shows each key, and checks what is set.
//! A switch (`adopt.enabled`) is no line of a file but a file's presence in the user's folder: it
//! is shown as `user` when on and `default` when off, and set by writing or removing that file,
//! whichever file `--user` names.

mod write;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cortex_core::{SWITCH_KEYS, Setting, SettingSource, UserFolder};
use cortex_daemon::settings::{self, ConfigFile, Place};
use serde::Serialize;

pub(crate) use self::write::set;
use crate::{output, repository};

/// The arguments of `cortex config`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Change a setting instead of listing them.
    #[command(subcommand)]
    action: Option<Action>,
    /// The repository's root folder; the current folder when left out.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    /// Print the settings as JSON.
    #[arg(long)]
    json: bool,
}

/// What `cortex config` can change.
#[derive(Debug, clap::Subcommand)]
enum Action {
    /// Set one key in the repository's .cortex/config.toml, or the user's own with --user,
    /// keeping the file's other lines, and a copy of it before Cortex first changed it.
    Set(SetArgs),
}

/// The arguments of `cortex config set`.
#[derive(Debug, clap::Args)]
struct SetArgs {
    /// The key, written with dots, such as retrieval.max_files.
    key: String,
    /// The value: a number, true or false, a word, or a TOML value such as ["*.log"].
    value: String,
    /// Write the user's own ~/.cortex/config.toml instead of the repository's file.
    #[arg(long)]
    user: bool,
}

/// One setting as `--json` prints it.
#[derive(Debug, Serialize)]
struct Row {
    /// The key, written with dots.
    key: &'static str,
    /// Its value in effect.
    value: serde_json::Value,
    /// Where the value came from: `default`, `profile`, `user` or `repository`.
    source: &'static str,
    /// What the key changes.
    description: &'static str,
    /// Its value as a configuration file would set it, for the text listing.
    #[serde(skip)]
    shown: String,
}

/// Lists the settings in effect, or sets one key.
///
/// # Errors
///
/// A root that is not a folder, a configuration file that cannot be read or used, a key or value
/// the configuration refuses, a file that cannot be written, or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    match &args.action {
        Some(Action::Set(set_args)) => {
            let place = if set_args.user {
                Place::User
            } else {
                Place::Repository
            };
            set(&root, place, &set_args.key, &set_args.value)
        }
        None => list(&root, args.json),
    }
}

/// Prints every setting of the repository at `root`, as text or, when `is_json`, as JSON, after a
/// warning on stderr for each key a file sets that Cortex does not read.
///
/// # Errors
///
/// A configuration file that cannot be read or used, or output that cannot be written.
fn list(root: &Path, is_json: bool) -> anyhow::Result<()> {
    let files = settings::read_files(root)?;
    let loaded = settings::load_files(&files)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    let mut rows = loaded
        .settings()
        .iter()
        .map(|setting| row(setting, &files))
        .collect::<anyhow::Result<Vec<_>>>()?;
    rows.extend(switch_rows()?);
    rows.sort_by_key(|row| row.key);
    if is_json {
        let json = serde_json::to_string_pretty(&rows).context("writing the settings as JSON")?;
        // JSON escapes every control character inside its strings, so its own line breaks are the
        // only ones, and each line is printed as it is.
        for line in json.lines() {
            output::line(line)?;
        }
        return Ok(());
    }
    for row in &rows {
        output::line(&format!("{} = {} ({})", row.key, row.shown, row.source))?;
    }
    Ok(())
}

/// Each switch as `--json` prints it: `true` from the user's folder when its file is there, else
/// `false` by default.
///
/// # Errors
///
/// No home folder.
fn switch_rows() -> anyhow::Result<Vec<Row>> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    Ok(SWITCH_KEYS
        .iter()
        .map(|switch| {
            let is_on = switch.is_on(&user);
            Row {
                key: switch.name,
                value: serde_json::Value::Bool(is_on),
                source: if is_on { "user" } else { "default" },
                description: switch.description,
                shown: is_on.to_string(),
            }
        })
        .collect())
}

/// `setting` as `--json` prints it, its source named from the `files` read.
///
/// # Errors
///
/// The value cannot be written as JSON, or names a file that was not read.
fn row(setting: &Setting, files: &[ConfigFile]) -> anyhow::Result<Row> {
    Ok(Row {
        key: setting.key,
        value: serde_json::to_value(&setting.value)
            .with_context(|| format!("writing the value of `{}` as JSON", setting.key))?,
        source: source_name(setting.source, files)?,
        description: setting.description,
        shown: setting.value.to_string(),
    })
}

/// The word for where a value came from, a file's position being one in `files`.
///
/// # Errors
///
/// The position names no file read, which loading the files never gives.
fn source_name(source: SettingSource, files: &[ConfigFile]) -> anyhow::Result<&'static str> {
    match source {
        SettingSource::Default => Ok("default"),
        SettingSource::Profile => Ok("profile"),
        SettingSource::Layer(position) => files
            .get(position)
            .map(|file| file.place.name())
            .with_context(|| format!("a setting from configuration file {position}, not read")),
    }
}
