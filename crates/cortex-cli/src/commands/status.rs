//! `cortex status`: whether the repository's daemon runs (asked of the daemon itself), whether its
//! index is ready, the scale mode, and how many files are stored, parsed and waiting to be parsed.
//! The store is only read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{FileFlags, ModeSetting, RepoId, UserFolder};
use cortex_daemon::settings;
use cortex_hook::{Answer, ClientError, ask};
use cortex_index::waits_to_be_parsed;
use cortex_store::{MetaKey, Queries, Reader, StoreError, is_writer_running};

use crate::{output, repository};

/// How long the daemon has to answer. It answers status from memory, so a daemon that takes longer
/// is stuck, not busy.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// The arguments of `cortex status`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Prints the repository's status, one fact per line, or that it has no index.
///
/// # Errors
///
/// A root that is not a folder, an unusable configuration, a store that cannot be read, or
/// stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let loaded = settings::load(&root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    for line in daemon_lines(&root) {
        output::line(&line)?;
    }
    let folder = root.join(".cortex");
    let reader = match Reader::open(&folder, 0) {
        Ok(reader) => reader,
        Err(StoreError::Missing { .. }) => {
            return output::line(&format!(
                "no index in {}; `cortex daemon` builds one",
                root.display()
            ));
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading the store in {}", folder.display()));
        }
    };
    for line in status_lines(&folder, reader.queries(), loaded.config.mode)? {
        output::line(&line)?;
    }
    Ok(())
}

/// The lines saying whether the daemon of the repository at `root` runs and what waits for it,
/// from its own answer.
fn daemon_lines(root: &Path) -> Vec<String> {
    let Some(user) = UserFolder::of_current_user() else {
        return vec!["daemon: unknown, no home folder to find it from".to_owned()];
    };
    let files = user.run_files(&RepoId::of(root));
    let deadline = Instant::now() + ANSWER_WITHIN;
    let line = match ask(&files, Role::Cli, RequestBody::Status, deadline) {
        Ok(Answer {
            body: ResponseBody::Status(report),
            ..
        }) => {
            return vec![
                format!(
                    "daemon: running (process {}, Cortex {}, up {} s)",
                    report.pid,
                    report.cortex_version,
                    report.uptime_ms / 1000
                ),
                format!(
                    "queue: {} events and {} changes waiting; {} events dropped, {} batches \
                     not recorded",
                    report.events_waiting,
                    report.changes_waiting,
                    report.events_dropped,
                    report.failed_batches
                ),
            ];
        }
        Ok(_) => "daemon: running, but it did not answer with its status".to_owned(),
        Err(ClientError::NotRunning) => "daemon: not running".to_owned(),
        Err(ClientError::TimedOut) => "daemon: not answering".to_owned(),
        Err(ClientError::Mismatch(theirs)) => format!(
            "daemon: running Cortex {}, which speaks another protocol",
            theirs.cortex_version
        ),
        Err(other) => format!("daemon: unreachable ({other})"),
    };
    vec![line]
}

/// The status of the store in `folder` that `queries` reads, with the mode `setting`.
fn status_lines(
    folder: &Path,
    queries: Queries<'_>,
    setting: ModeSetting,
) -> anyhow::Result<Vec<String>> {
    let index = if queries.meta(MetaKey::CortexVersion)?.is_some() {
        "ready"
    } else if is_writer_running(folder)? {
        "not ready, an index is under way"
    } else {
        "not ready, the last index did not finish; the next one reads every file again"
    };
    let mut mode = queries
        .meta(MetaKey::ScaleMode)?
        .unwrap_or_else(|| "not counted yet".to_owned());
    if let ModeSetting::Pinned(pinned) = setting {
        mode.push_str(&format!(
            "; pinned to {} in the configuration",
            pinned.name()
        ));
    }
    let rows = queries.file_rows()?;
    let parsed = rows
        .iter()
        .filter(|row| row.file.flags.contains(FileFlags::PARSED))
        .count();
    let mut files = format!("{} stored, {parsed} parsed", rows.len());
    let waiting = rows
        .iter()
        .filter(|row| waits_to_be_parsed(&row.file))
        .count();
    if waiting > 0 {
        files.push_str(&format!(", {waiting} waiting to be parsed"));
    }
    Ok(vec![
        format!("index: {index}"),
        format!("mode: {mode}"),
        format!("files: {files}"),
    ])
}
