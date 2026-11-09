//! The repository's daemon once `cortex init` has written its files: found running, or started (one
//! older than this build replaced), then waited for until it says its first index is done, for at
//! most 5 minutes, with how many files the store holds said on stderr every 2 seconds while the
//! store can be read. An index that takes longer goes on in the background, and init says so; a
//! daemon that stops before its index is done fails init, which names the daemon's log.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{RepoId, UserFolder};
use cortex_daemon::{is_running, settings};
use cortex_hook::{Answer, ClientError, ask};
use cortex_store::Reader;

use crate::commands::status::index_lines;
use crate::{output, upgrade};

/// How long a running daemon has to answer its status: it answers from memory, so a daemon that
/// takes longer is stuck, not busy.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// How long init waits for the first index: a mid-size repository's takes well under a minute.
const INDEX_WITHIN: Duration = Duration::from_secs(300);

/// How often init asks the daemon how far its index is.
const LOOK_EVERY: Duration = Duration::from_millis(200);

/// How often init says how many files the store holds.
const SAY_EVERY: Duration = Duration::from_secs(2);

/// How many times in a row a daemon that still holds the repository may fail to answer before init
/// gives up on it: one that is stopping lets go of the repository within moments.
const FAILURES_ALLOWED: usize = 5;

/// Finds or starts the daemon of the repository whose canonical root is `root`, says which, waits
/// for its first index, and says how far the index is. A daemon of a newer Cortex that speaks
/// another protocol is left alone, unasked, and said to be so.
///
/// # Errors
///
/// No home folder, a daemon that cannot be started, does not answer or stops before its first
/// index is done, a configuration that does not load, or a store that cannot be read.
pub(super) fn started_and_indexed(root: &Path) -> anyhow::Result<()> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let deadline = Instant::now() + ANSWER_WITHIN;
    let asked = upgrade::ask_started(root, &user, &RequestBody::Status, deadline);
    if let Some(older) = &asked.replaced {
        output::line(&format!("daemon: replaced a daemon of Cortex {older}"))?;
    }
    match asked.answer {
        Ok(Answer {
            body: ResponseBody::Status(report),
            ..
        }) => output::line(&format!(
            "daemon: running (process {}, Cortex {})",
            report.pid, report.cortex_version
        ))?,
        Ok(_) => bail!("the daemon did not answer with its status"),
        Err(ClientError::Mismatch(theirs)) => {
            return output::line(&format!(
                "daemon: running Cortex {}, which speaks another protocol; init does not wait for \
                 its index",
                theirs.cortex_version
            ));
        }
        Err(error) => return Err(error).context("starting the repository's daemon"),
    }
    waited(root, &user)?;
    let config = settings::load(root)?.config;
    // The status's first three lines say what the index holds (or its one line, why there is
    // none); the rest (learning, the gate, warnings) says nothing yet on a repository just set
    // up, and `cortex status` shows it.
    for line in index_lines(root, &config)?.into_iter().take(3) {
        output::line(&line)?;
    }
    Ok(())
}

/// Waits until the daemon of the repository at `root`, the user's folder being `user`, says its
/// first index is done, at most [`INDEX_WITHIN`], saying every [`SAY_EVERY`] how many files the
/// store holds, and says when it gives up waiting. A failed answer from a daemon that no longer
/// holds the repository means it stopped; one that still holds it gets [`FAILURES_ALLOWED`] tries.
///
/// # Errors
///
/// The daemon stops, or fails to answer that many times in a row, or stdout or stderr cannot be
/// written.
fn waited(root: &Path, user: &UserFolder) -> anyhow::Result<()> {
    let repo_id = RepoId::of(root);
    let files = user.run_files(&repo_id);
    let started = Instant::now();
    let mut said = started;
    let mut failures = 0;
    loop {
        let deadline = Instant::now() + ANSWER_WITHIN;
        match ask(&files, Role::Cli, RequestBody::Status, deadline) {
            Ok(Answer {
                body: ResponseBody::Status(report),
                ..
            }) if report.is_indexed => return Ok(()),
            Ok(Answer {
                body: ResponseBody::Status(_),
                ..
            }) => failures = 0,
            Ok(_) => bail!("the daemon did not answer with its status"),
            Err(error) => {
                if !is_running(user, &repo_id).context("finding whether the daemon runs")? {
                    bail!(
                        "the daemon stopped before its first index was done; its log says why: {}",
                        user.logs_folder()
                            .join(format!("daemon-{repo_id}.log"))
                            .display()
                    );
                }
                failures += 1;
                if failures >= FAILURES_ALLOWED {
                    return Err(error).context("asking the daemon how far its index is");
                }
            }
        }
        if started.elapsed() >= INDEX_WITHIN {
            return output::line(&format!(
                "index: still under way after {} min; it goes on in the background, and `cortex \
                 status` says when it is ready",
                INDEX_WITHIN.as_secs() / 60
            ));
        }
        if said.elapsed() >= SAY_EVERY {
            if let Some(stored) = stored(&root.join(".cortex")) {
                output::note(&format!("indexing: {stored} files stored"))?;
            }
            said = Instant::now();
        }
        std::thread::sleep(LOOK_EVERY);
    }
}

/// How many files the store in `folder` holds, or `None` while it cannot be read, as while the
/// daemon makes it: the progress line is then left out, never guessed.
fn stored(folder: &Path) -> Option<u64> {
    let reader = Reader::open(folder, 0).ok()?;
    reader.queries().counts().ok().map(|counts| counts.files)
}
