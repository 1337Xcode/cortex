//! `cortex features`: what Cortex gives agents, each feature's state in this repository, the key
//! that switches it and the measurement that decides whether it should be on. No effect is shown:
//! a feature's effect is shown only once it has been measured here against held-out tasks, and no
//! repository's receipts yet hold enough of them. `cortex features on|off <name>` sets the
//! feature's key through `cortex config set`.

use std::path::PathBuf;

use anyhow::bail;
use cortex_core::{FEATURES, feature};
use cortex_daemon::settings::{self, Place};

use crate::commands::config;
use crate::{output, repository};

/// What `cortex features` says of a feature's effect in this repository.
const NOT_MEASURED: &str = "not measured here yet";

/// The arguments of `cortex features`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Switch a feature instead of listing them.
    #[command(subcommand)]
    action: Option<Action>,
    /// The repository's root folder; the current folder when left out.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

/// What `cortex features` can change.
#[derive(Debug, clap::Subcommand)]
enum Action {
    /// Turn a feature on in the repository's configuration, or the user's with --user.
    On(Switch),
    /// Turn a feature off in the repository's configuration, or the user's with --user.
    Off(Switch),
}

/// The arguments of `cortex features on` and `off`.
#[derive(Debug, clap::Args)]
struct Switch {
    /// The feature's name, as `cortex features` lists it.
    name: String,
    /// Write the user's own ~/.cortex/config.toml instead of the repository's file.
    #[arg(long)]
    user: bool,
}

/// Lists the features, or switches one.
///
/// # Errors
///
/// A root that is not a folder, a configuration that cannot be read or used, a feature that does
/// not exist or that no key switches, a file that cannot be written, or stdout that cannot be
/// written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let (switch, is_on) = match &args.action {
        None => return list(&root),
        Some(Action::On(switch)) => (switch, true),
        Some(Action::Off(switch)) => (switch, false),
    };
    let Some(chosen) = feature(&switch.name) else {
        let names: Vec<&str> = FEATURES.iter().map(|each| each.name).collect();
        bail!(
            "unknown feature `{}`; the features are {}",
            switch.name,
            names.join(", ")
        );
    };
    let Some(key) = chosen.key else {
        bail!("`{}` has no key of its own to switch it", chosen.name);
    };
    let place = if switch.user {
        Place::User
    } else {
        Place::Repository
    };
    config::set(&root, place, key, if is_on { "true" } else { "false" })
}

/// Prints each feature: its name, whether the settings of the repository at `root` let it
/// reach a task and where it stands, then what it gives, what decides it and its measured effect
/// here.
///
/// # Errors
///
/// A configuration that cannot be read or used, or output that cannot be written.
fn list(root: &std::path::Path) -> anyhow::Result<()> {
    let loaded = settings::load(root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    for each in FEATURES {
        let is_on = if each.is_on(&loaded.config) {
            "on"
        } else {
            "off"
        };
        let key = each
            .key
            .map_or("no key".to_owned(), |key| format!("key {key}"));
        output::line(&format!(
            "{}: {is_on} (state {}, {key})",
            each.name,
            each.state.name()
        ))?;
        output::line(&format!("  {}", each.description))?;
        output::line(&format!("  decided by: {}", each.measure))?;
        output::line(&format!("  effect here: {NOT_MEASURED}"))?;
    }
    Ok(())
}
