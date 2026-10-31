//! The daemon: the long-lived process that keeps a repository's index, and its only writer.
//!
//! It takes the store's writer, so a second daemon on the same repository is refused, and indexes
//! what changed since the last index. Then it watches the repository and indexes again after each
//! change. In huge mode it parses the files it took inventory of, those that changed first, at
//! `index.crawl_files_per_second` on one thread. It stops after `daemon.idle_shutdown_minutes`
//! without a file change, once nothing waits to be parsed. It says when the first index is done
//! and the watcher runs.
//!
//! Errors are `anyhow` errors with context, as a binary's are: this crate is the daemon process's
//! top level, and its errors are reported, never matched.

mod global_ignore;
mod host;
mod instance;
pub mod settings;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use anyhow::Context;
use cortex_core::{RepoId, UserFolder};
use cortex_index::{Change, DiscoverOptions, Grammars, IndexReport, watch};
use cortex_store::{Writer, WriterOptions};

use self::host::Host;
use self::instance::Instance;

/// How many changes may wait for the writer. A change that finds the queue full is dropped and the
/// queue marked as overflowed, so that the changes drained next are indexed as a change to
/// everything, the dropped change's files included.
const QUEUE_CHANGES: usize = 10_000;

/// Runs the daemon for the repository whose canonical root is `root` until it has been idle long
/// enough. Each line a person running it in the foreground should see (configuration warnings,
/// the ready line) goes to `note`.
///
/// # Errors
///
/// No home folder, another daemon serving the repository, an unusable configuration, a store
/// that cannot be opened, a watcher that cannot start or stops, a failed index, and a line `note`
/// could not write.
pub fn run(root: &Path, note: &mut dyn FnMut(&str) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let user = UserFolder::of_current_user().context("finding the user's home folder")?;
    let repo_id = RepoId::of(root);
    let _instance = Instance::acquire(&user, &repo_id)?;
    let loaded = settings::load(root)?;
    for warning in &loaded.warnings {
        note(&format!("warning: {warning}"))?;
    }
    let config = loaded.config;
    let grammars = Arc::new(Grammars::load().context("loading the language grammars")?);
    let folder = root.join(".cortex");
    // INVARIANT: single-writer (the store's writer is opened here only, and its lock refuses a
    // second one)
    let writer = Writer::open(&WriterOptions {
        folder: &folder,
        repo_id: &repo_id,
        mmap_bytes: config.mmap_bytes,
        now_ms: host::now_ms(),
    })
    .with_context(|| format!("opening the store in {}", folder.display()))?;
    let (sender, queue) = mpsc::sync_channel::<Change>(QUEUE_CHANGES);
    let global_gitignore = match global_ignore::find(root) {
        Ok(found) => found,
        Err(error) => {
            note(&format!(
                "warning: the global gitignore is not read: {error:#}"
            ))?;
            None
        }
    };
    let discover = DiscoverOptions {
        max_file_bytes: config.max_file_bytes,
        extra_ignores: &config.extra_ignores,
        global_gitignore: global_gitignore.as_deref(),
    };
    let overflowed = Arc::new(AtomicBool::new(false));
    let marks = Arc::clone(&overflowed);
    // A full queue drops the change and marks the overflow (see QUEUE_CHANGES); a closed one means
    // the daemon is stopping, when nothing is left to index.
    let watcher = watch(root, &discover, move |change| {
        if sender.try_send(change).is_err() {
            marks.store(true, Ordering::Release);
        }
    })
    .context("watching the repository")?;
    let mut host = Host::new(root, grammars, (&config, discover), writer);
    let report = host.index(&[])?;
    note(&ready_line(&report, host.waiting()))?;
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
