//! `cortex uninstall`: undoes what `cortex init` did to a repository and stops its daemon. Each
//! file init changed gets back what it held before, from init's backup, when it still holds what
//! init wrote, and is removed when init made it; a file changed since keeps the change, and only
//! what init added comes out of it (see [`restore`]). Init's manifests are read from the user's
//! folder alone (see [`runs`]): a manifest inside a repository may have come with the clone.
//! `--scope user` also undoes init's changes to the user's own files, which serve every
//! repository. The files go first, so that Claude Code, which its documentation says reloads
//! settings files when they change, stops running the hooks; then the repository's daemon,
//! whatever its version, is asked to stop; then `--purge` deletes what Cortex keeps of the
//! repository (see [`purge`]), only once every change is undone and the daemon has stopped, so
//! that no backup a retry needs is lost. Nothing is said until all of that is done, so that an
//! output closed early never stops it partway. Running it again changes nothing.
//!
//! `--global` undoes `cortex init --global`: every change init made to the user's own files is
//! undone, whatever repository or none it was made for, and adoption is turned off. Repositories
//! Cortex set up on its own keep their `.cortex/`, which `cortex uninstall` in each takes out.

mod purge;
mod restore;
pub(crate) mod runs;
mod users;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use cortex_core::ipc::Role;
use cortex_core::{RepoId, UserFolder};
use cortex_daemon::is_running;
use cortex_hook::{ClientError, stop_any};

use super::init::{Scope, switch_off};
use crate::backup::{Changed, Kind, replace_unchanged};
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
    /// Whose files to undo: the repository's, or also the user's own, which serve every
    /// repository.
    #[arg(long, value_enum, default_value_t = Scope::Project)]
    scope: Scope,
    /// Also delete the repository's `.cortex/` but `.cortex/shared/`, and its manifests and daemon
    /// log in the user's folder.
    #[arg(long)]
    purge: bool,
    /// Undo `cortex init --global`: every change to the user's own files, and adoption.
    #[arg(long, conflicts_with_all = ["root", "scope", "purge"])]
    global: bool,
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

/// Undoes init's changes to the repository `args` name, and to the user's files at the user's
/// scope, stops the repository's daemon and, asked to, purges what Cortex keeps of it; then says
/// what it did.
///
/// # Errors
///
/// No home folder, a folder that cannot be found, a change that could not be undone (each named on
/// stderr, the others undone all the same), a daemon that does not stop, a purge that fails, or
/// an output that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    if args.global {
        return run_global();
    }
    let root = repository::root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let program = crate::program::this_program()?;
    let mut report = Report {
        said: vec![format!("repository: {}", root.display())],
        ..Report::default()
    };
    let repository_runs = runs::of_repository(&user, &root, &mut report.notes)?;
    let user_runs = match args.scope {
        Scope::Project => Vec::new(),
        Scope::User => runs::of_user(&user, &mut report.notes)?,
    };
    if repository_runs.is_empty() && user_runs.is_empty() {
        report.said.push(format!(
            "nothing to restore: no manifest of init's in {} names this repository",
            user.path().join("backups").display()
        ));
    }
    undo_all(&repository_runs, &program, &mut report);
    undo_all(&user_runs, &program, &mut report);
    // Forgotten before the daemon stops, so that no hook serves the repository again meanwhile.
    if let Err(error) = cortex_core::forget_set_up(&user, &root) {
        report.failures.push(format!(
            "error: could not forget that the repository is set up here: {error}"
        ));
    }
    if user.adopt_path().is_file() {
        report.notes.push(
            "note: adoption is on, so Cortex sets this repository up again the next time an agent \
             works in it; `cortex off` keeps it off here"
                .to_owned(),
        );
    }
    let stopped = stopped(&root, &user);
    if let Ok(line) = &stopped {
        report.said.push(line.clone());
    }
    let purged = purge_or_keep(
        (args.purge, stopped.is_ok()),
        (&root, &user),
        &repository_runs,
        &mut report,
    );
    if args.scope == Scope::Project {
        report.said.extend(users::users_hooks(&user, &program));
    }
    // A daemon that did not stop, or a purge that failed, matters more than an output that could
    // not be written.
    let written = said(&report);
    stopped?;
    purged?;
    written?;
    if !report.failures.is_empty() {
        bail!(
            "{} of init's changes could not be undone; each is named above",
            report.failures.len()
        );
    }
    Ok(())
}

/// Undoes init's changes to the user's own files and turns adoption off, then says what it did.
///
/// # Errors
///
/// No home folder, a change that could not be undone (each named on stderr, the others undone all
/// the same), adoption that cannot be turned off, or an output that cannot be written.
fn run_global() -> anyhow::Result<()> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let program = crate::program::this_program()?;
    let mut report = Report::default();
    let runs = runs::of_user(&user, &mut report.notes)?;
    if runs.is_empty() {
        report.said.push(format!(
            "nothing to restore: no manifest of init's in {} changed your own files",
            user.path().join("backups").display()
        ));
    }
    undo_all(&runs, &program, &mut report);
    let switched = switch_off(&user);
    if let Ok(was_on) = switched {
        report.said.push(
            if was_on {
                "adoption: off; Cortex sets up no repository on its own"
            } else {
                "adoption: was off"
            }
            .to_owned(),
        );
    }
    let written = said(&report);
    switched?;
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
/// this binary, recording in `report` what it did and what it could not do. A run whose every
/// change is undone is marked so in its manifest, so that doctor no longer checks what it wrote.
fn undo_all(runs: &[runs::Run], program: &str, report: &mut Report) {
    for run in runs {
        let failed_before = report.failures.len();
        for change in run.manifest.files.iter().rev() {
            match restore::undo(run, change, program) {
                Ok(Some(line)) => {
                    report.said.push(line);
                    if let Some(after) = after_removal(change)
                        && !report.said.iter().any(|said| said.ends_with(after))
                    {
                        report.said.push(format!("note: {after}"));
                    }
                }
                Ok(None) => {}
                Err(error) => report.failures.push(format!("error: {error:#}")),
            }
        }
        if report.failures.len() == failed_before
            && !run.manifest.undone
            && let Err(error) = marked_undone(run)
        {
            report.notes.push(format!(
                "could not mark {} as undone, so doctor still checks what it wrote: {error:#}",
                run.manifest_path.display()
            ));
        }
    }
}

/// What uninstall says once it took out `change`, a file of Cortex's own of an agent that leaves
/// something for the user to undo then; none for any other change.
fn after_removal(change: &Changed) -> Option<&'static str> {
    let Kind::AgentFile { agent } = &change.kind else {
        return None;
    };
    crate::agents::setup(agent)?.own.as_ref()?.after_removal
}

/// Writes `run`'s manifest again, marked as undone, provided it still holds what was read.
///
/// # Errors
///
/// The manifest cannot be read, written, or changed since it was read.
fn marked_undone(run: &runs::Run) -> anyhow::Result<()> {
    let before = fs::read(&run.manifest_path)
        .with_context(|| format!("reading {}", run.manifest_path.display()))?;
    let mut manifest = run.manifest.clone();
    manifest.undone = true;
    let after = serde_json::to_vec_pretty(&manifest).context("writing the manifest")?;
    replace_unchanged(&run.manifest_path, Some(&before), &after)
}

/// Purges what Cortex keeps of the repository at `root`, whose user folder is `user` and whose
/// init `runs` were undone, when `is_asked`, the daemon `is_stopped`, none holds the repository
/// again (a hook a running agent still has may start one) and nothing in `report` failed;
/// otherwise says why nothing was purged, or, not asked, that `.cortex/` stays. What it did goes
/// into `report`, as far as it got.
///
/// # Errors
///
/// Whether a daemon runs cannot be found, or the purge failed.
fn purge_or_keep(
    (is_asked, is_stopped): (bool, bool),
    (root, user): (&Path, &UserFolder),
    runs: &[runs::Run],
    report: &mut Report,
) -> anyhow::Result<()> {
    let folder = root.join(".cortex");
    if !is_asked {
        if folder.is_dir() {
            report.said.push(format!(
                "note: {} stays, with the index and init's backups; `cortex uninstall --purge` \
                 deletes it",
                folder.display()
            ));
        }
        return Ok(());
    }
    let why_not = if !report.failures.is_empty() {
        Some("it keeps the backups that undoing what is left needs")
    } else if !is_stopped
        || is_running(user, &RepoId::of(root)).context("finding whether the daemon runs")?
    {
        Some("a daemon holds the repository")
    } else {
        None
    };
    match why_not {
        Some(why) => {
            report
                .notes
                .push(format!("nothing purged from {}: {why}", folder.display()));
            Ok(())
        }
        None => purge::purged(root, user, runs, &mut report.said),
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

/// Removes the folder `folder` when nothing is left in it.
///
/// # Errors
///
/// The folder cannot be read or removed.
fn remove_if_empty(folder: &Path) -> anyhow::Result<()> {
    let is_empty = fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .next()
        .is_none();
    if is_empty {
        fs::remove_dir(folder).with_context(|| format!("removing {}", folder.display()))?;
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
pub(crate) fn stopped(root: &Path, user: &UserFolder) -> anyhow::Result<String> {
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
