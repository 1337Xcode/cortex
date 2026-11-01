//! `cortex uninstall`: undoes what `cortex init` did to a repository and stops its daemon. Each
//! file init changed gets back what it held before, from init's backup, when it still holds what
//! init wrote, and is removed when init made it; a file changed since keeps the change, and only
//! what init added comes out of it (see [`restore`]). Init's manifests are read from the user's
//! folder alone (see [`runs`]): a manifest inside a repository may have come with the clone. The
//! files go first, so that Claude Code, which its documentation says reloads settings files when
//! they change, stops running the hooks; then the repository's daemon, whatever its version, is
//! asked to stop. Nothing is said until all of that is done, so that an output closed early never
//! stops it partway. Running it again changes nothing.

mod restore;
mod runs;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use cortex_core::ipc::Role;
use cortex_core::{RepoId, UserFolder};
use cortex_daemon::is_running;
use cortex_hook::{ClientError, stop_any};

use crate::{output, repository, upgrade};

/// How long uninstall gives the daemon to stop, from the first request: it drains its inbox and
/// gives its connections a second, as when a newer build replaces it.
const STOP_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex uninstall`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// What uninstall says once it is done: lines for stdout, and for stderr the changes it could not
/// undo and the notes about what it passed over.
#[derive(Debug, Default)]
struct Report {
    /// What it did, one line each.
    said: Vec<String>,
    /// Why each change it could not undo is left.
    failures: Vec<String>,
    /// What it passed over, and why.
    notes: Vec<String>,
}

/// Undoes init's changes to the repository `args` name and stops its daemon, then says what it
/// did.
///
/// # Errors
///
/// No home folder, a folder that cannot be found, a change that could not be undone (each named on
/// stderr, the others undone all the same), a daemon that does not stop, or an output that cannot
/// be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let program = crate::program::this_program()?;
    let mut report = Report {
        said: vec![format!("repository: {}", root.display())],
        ..Report::default()
    };
    let runs = runs::of_repository(&user, &root, &mut report.notes)?;
    if runs.is_empty() {
        report.said.push(format!(
            "nothing to restore: no manifest of init's in {} names this repository",
            user.path().join("backups").display()
        ));
    }
    undo_all(&runs, &program, &mut report);
    let stopped = stopped(&root, &user);
    if let Ok(line) = &stopped {
        report.said.push(line.clone());
    }
    if root.join(".cortex").is_dir() {
        report.said.push(format!(
            "note: {} stays, with the index and init's backups",
            root.join(".cortex").display()
        ));
    }
    // A daemon that did not stop matters more than an output that could not be written.
    let written = said(&report);
    stopped?;
    written?;
    if !report.failures.is_empty() {
        bail!(
            "{} of init's changes could not be undone; each is named above",
            report.failures.len()
        );
    }
    Ok(())
}

/// Undoes every change of `runs`, newest first and each run's last change first, `program` being
/// this binary, recording in `report` what it did and what it could not do.
fn undo_all(runs: &[runs::Run], program: &str, report: &mut Report) {
    for run in runs {
        for change in run.manifest.files.iter().rev() {
            match restore::undo(run, change, program) {
                Ok(Some(line)) => report.said.push(line),
                Ok(None) => {}
                Err(error) => report.failures.push(format!("error: {error:#}")),
            }
        }
    }
}

/// Writes what `report` holds: its lines to stdout, then its notes and failures to stderr.
///
/// # Errors
///
/// An output cannot be written.
fn said(report: &Report) -> anyhow::Result<()> {
    for line in &report.said {
        output::line(line)?;
    }
    for line in report.notes.iter().chain(&report.failures) {
        output::note(line)?;
    }
    Ok(())
}

/// Asks the daemon of the repository at `root`, whose user folder is `user`, to stop, whatever its
/// version, waits until it has let go of the repository, and says so; says when none runs.
///
/// # Errors
///
/// The daemon cannot be asked, does not answer or does not stop within [`STOP_WITHIN`], or whether
/// one runs cannot be found.
fn stopped(root: &Path, user: &UserFolder) -> anyhow::Result<String> {
    let repo_id = RepoId::of(root);
    let files = user.run_files(&repo_id);
    let until = Instant::now() + STOP_WITHIN;
    loop {
        match stop_any(&files, Role::Cli, until) {
            Ok(version) => {
                upgrade::wait_until_stopped(user, &repo_id, until)
                    .context("waiting for the daemon to stop")?;
                return Ok(format!("daemon: stopped (Cortex {version})"));
            }
            // Nothing listens, or the time ran out: either way a daemon that let go meanwhile is
            // none.
            Err(ClientError::NotRunning | ClientError::TimedOut)
                if !is_running(user, &repo_id).context("finding whether the daemon runs")? =>
            {
                return Ok("daemon: not running".to_owned());
            }
            // One that holds the repository but does not listen yet is starting.
            Err(ClientError::NotRunning) if Instant::now() < until => {
                std::thread::sleep(upgrade::LOOK_EVERY);
            }
            Err(ClientError::NotRunning | ClientError::TimedOut) => bail!(
                "the daemon holds the repository but did not answer within {} s",
                STOP_WITHIN.as_secs()
            ),
            Err(error) => return Err(error).context("asking the daemon to stop"),
        }
    }
}
