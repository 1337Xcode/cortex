//! `cortex daemon`: the long-lived process that keeps a repository's index, and its only writer.
//!
//! It takes the store's writer, so a second daemon on the same repository is refused, and indexes
//! what changed since the last index. Then it watches the repository and indexes again after each
//! change. In huge mode it parses the files it took inventory of, those that changed first, at
//! `index.crawl_files_per_second` on one thread. It stops after `daemon.idle_shutdown_minutes`
//! without a file change, once nothing waits to be parsed. One line on stderr says when the
//! first index is done and the watcher runs.

mod host;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use anyhow::Context;
use cortex_core::RepoId;
use cortex_index::{Change, DiscoverOptions, Grammars, IndexReport, watch};
use cortex_store::{Writer, WriterOptions};

use self::host::Host;
use crate::{output, repository, settings};

/// How many changes may wait for the writer. A change that finds the queue full is dropped and the
/// queue marked as overflowed, so that the changes drained next are indexed as a change to
/// everything, the dropped change's files included.
const QUEUE_CHANGES: usize = 10_000;

/// The arguments of `cortex daemon`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Runs the daemon for the repository until it has been idle long enough.
///
/// # Errors
///
/// A root that is not a folder, an unusable configuration, a store that cannot be opened (another
/// daemon holds it), a watcher that cannot start or stops, and a failed index.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let loaded = settings::load(&root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    let config = loaded.config;
    let grammars = Arc::new(Grammars::load().context("loading the language grammars")?);
    let folder = root.join(".cortex");
    let writer = Writer::open(&WriterOptions {
        folder: &folder,
        repo_id: &RepoId::of(&root),
        mmap_bytes: config.mmap_bytes,
        now_ms: host::now_ms(),
    })
    .with_context(|| format!("opening the store in {}", folder.display()))?;
    let (sender, queue) = mpsc::sync_channel::<Change>(QUEUE_CHANGES);
    let discover = DiscoverOptions {
        max_file_bytes: config.max_file_bytes,
        extra_ignores: &config.extra_ignores,
        global_gitignore: None,
    };
    let overflowed = Arc::new(AtomicBool::new(false));
    let marks = Arc::clone(&overflowed);
    // A full queue drops the change and marks the overflow (see QUEUE_CHANGES); a closed one means
    // the daemon is stopping, when nothing is left to index.
    let watcher = watch(&root, &discover, move |change| {
        if sender.try_send(change).is_err() {
            marks.store(true, Ordering::Release);
        }
    })
    .context("watching the repository")?;
    let mut host = Host::new(&root, grammars, &config, writer);
    let report = host.index(&[])?;
    output::note(&ready_line(&report, host.waiting()))?;
    host.serve(&queue, &overflowed)?;
    drop(watcher);
    Ok(())
}

/// The line that says the first index is done: what it wrote, the mode, and in huge mode how many
/// files wait to be parsed.
fn ready_line(report: &IndexReport, waiting: usize) -> String {
    let mut line = format!(
        "daemon ready: mode {}; files: {} written, {} unchanged",
        report.mode.name(),
        report.files,
        report.unchanged
    );
    if waiting > 0 {
        line.push_str(&format!(", {waiting} waiting to be parsed"));
    }
    line
}
