//! `cortex pause` and `cortex resume`: stop what Cortex says to an agent in a repository, and
//! start saying it again. What the agents do there is still recorded and still learned from while
//! a pause holds, so a pause costs nothing but the help itself; `cortex off` is the heavier thing,
//! which stops the hooks and the daemon.
//!
//! The pause is a file in the repository's `.cortex`, for the reason the off marker is one: only
//! the daemon writes the store, so a command that wrote this there would need a request of its
//! own. A pause with no minutes holds until `cortex resume`.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Context as _;
use cortex_core::{PAUSED_FILE, Pause, open_private_file, pause_of, unix_millis};

use super::init::own_folder;
use super::off::OFF_GITIGNORE;
use crate::{output, repository};

/// The arguments of `cortex pause`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// How many minutes to pause for; until `cortex resume` when left out.
    #[arg(long)]
    minutes: Option<u32>,
    /// The repository's root folder; the repository, or the git working tree, the current folder
    /// is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// The arguments of `cortex resume`.
#[derive(Debug, clap::Args)]
pub(crate) struct ResumeArgs {
    /// The repository's root folder; the repository, or the git working tree, the current folder
    /// is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Pauses the repository `args` names and says until when.
///
/// # Errors
///
/// A folder that cannot be found, a `.cortex` that cannot be made or written, minutes that reach
/// past what a moment can hold, or an output that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::switched_root(args.root.as_deref())?;
    let is_new = !root.join(".cortex").exists();
    let cortex = own_folder(&root, ".cortex")?;
    if is_new {
        let ignore = cortex.join(".gitignore");
        fs::write(&ignore, OFF_GITIGNORE)
            .with_context(|| format!("writing {}", ignore.display()))?;
    }
    let pause = pause_for(args.minutes, unix_millis(SystemTime::now()))?;
    let path = cortex.join(PAUSED_FILE);
    let mut file =
        open_private_file(&path).with_context(|| format!("writing {}", path.display()))?;
    file.set_len(0)
        .and_then(|()| writeln!(file, "{}", pause.written()))
        .with_context(|| format!("writing {}", path.display()))?;
    output::line(&format!(
        "cortex: paused in {}: {}. What agents do here is still recorded; `cortex resume` turns \
         the help back on",
        root.display(),
        said(pause, args.minutes)
    ))
}

/// Takes the pause off the repository `args` names and says so, whether or not one was held.
///
/// # Errors
///
/// A folder that cannot be found, a marker that cannot be removed, or an output that cannot be
/// written.
pub(crate) fn resume(args: &ResumeArgs) -> anyhow::Result<()> {
    let root = repository::switched_root(args.root.as_deref())?;
    let path = root.join(".cortex").join(PAUSED_FILE);
    let was_paused = pause_of(&root).is_some();
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("removing {}", path.display()));
        }
    }
    output::line(&if was_paused {
        format!("cortex: on again in {}", root.display())
    } else {
        format!("cortex: {} was not paused", root.display())
    })
}

/// The pause `minutes` asks for as of `now_ms`: until it is lifted when none is asked for.
///
/// # Errors
///
/// Minutes that reach past what a moment can hold, which no real pause does.
fn pause_for(minutes: Option<u32>, now_ms: i64) -> anyhow::Result<Pause> {
    let Some(minutes) = minutes else {
        return Ok(Pause::UntilLifted);
    };
    let span = i64::from(minutes)
        .checked_mul(60_000)
        .and_then(|span| now_ms.checked_add(span))
        .context("that many minutes reaches past the end of time")?;
    Ok(Pause::Until(span))
}

/// How the pause reads in the line the command prints.
fn said(pause: Pause, minutes: Option<u32>) -> String {
    match (pause, minutes) {
        (Pause::Until(_), Some(1)) => "for 1 minute".to_owned(),
        (Pause::Until(_), Some(minutes)) => format!("for {minutes} minutes"),
        _ => "until it is lifted".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No minutes is a pause until it is lifted; minutes are added to the moment it is asked for,
    /// so 5 minutes from a moment 1,000 ms after the epoch ends at 301,000.
    #[test]
    fn minutes_become_the_moment_a_pause_ends() -> anyhow::Result<()> {
        assert_eq!(pause_for(None, 1_000)?, Pause::UntilLifted);
        assert_eq!(pause_for(Some(5), 1_000)?, Pause::Until(301_000));
        assert_eq!(pause_for(Some(0), 1_000)?, Pause::Until(1_000));
        Ok(())
    }

    /// Minutes that would reach past what a moment can hold are refused rather than wrapped.
    #[test]
    fn a_pause_past_the_end_of_time_is_refused() {
        assert!(pause_for(Some(u32::MAX), i64::MAX - 1).is_err());
    }

    /// One minute is said in the singular, as every other count is.
    #[test]
    fn the_line_counts_minutes_in_the_singular() {
        assert_eq!(said(Pause::Until(0), Some(1)), "for 1 minute");
        assert_eq!(said(Pause::Until(0), Some(2)), "for 2 minutes");
        assert_eq!(said(Pause::UntilLifted, None), "until it is lifted");
    }
}
