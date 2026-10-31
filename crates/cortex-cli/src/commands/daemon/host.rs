//! The daemon's work loop: the one writer, the changes the watcher queues for it, the huge mode
//! crawl between changes, and the idle stop.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use cortex_core::{Config, RepoPath};
use cortex_index::{
    Change, Crawl, DiscoverOptions, Grammars, IndexOptions, IndexReport, index_repository,
    parse_files,
};
use cortex_store::Writer;

/// How often the crawler takes a batch of `index.crawl_files_per_second` files.
const CRAWL_PERIOD: Duration = Duration::from_secs(1);

/// The daemon's state between changes: the writer and the files waiting to be parsed.
pub(super) struct Host<'a> {
    /// The repository's root folder.
    root: &'a Path,
    /// Every language's grammar, loaded once.
    grammars: Arc<Grammars>,
    /// The settings in effect.
    config: &'a Config,
    /// The store's one writer.
    writer: Writer,
    /// The files huge mode took inventory of, in the order they are parsed.
    crawl: Crawl,
}

impl<'a> Host<'a> {
    /// A host over `writer` for the repository at `root`, nothing waiting yet.
    pub(super) fn new(
        root: &'a Path,
        grammars: Arc<Grammars>,
        config: &'a Config,
        writer: Writer,
    ) -> Self {
        Self {
            root,
            grammars,
            config,
            writer,
            crawl: Crawl::default(),
        }
    }

    /// Indexes what changed since the last index on every core, then lists the files waiting to
    /// be parsed, those among `touched` first.
    ///
    /// # Errors
    ///
    /// A failed index, or a store error while listing.
    pub(super) fn index(&mut self, touched: &[RepoPath]) -> anyhow::Result<IndexReport> {
        let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        let options = self.options(threads);
        let report = index_repository(self.root, &self.grammars, &mut self.writer, &options)
            .context("indexing the repository")?;
        self.crawl = Crawl::of_store(self.writer.queries())
            .context("listing the files waiting to be parsed")?;
        self.crawl.request(touched);
        Ok(report)
    }

    /// How many files wait to be parsed.
    pub(super) fn waiting(&self) -> usize {
        self.crawl.waiting()
    }

    /// Indexes after each change `queue` brings, crawls while no change comes, and returns once
    /// nothing waits to be parsed and no change came for `daemon.idle_shutdown_minutes`.
    ///
    /// # Errors
    ///
    /// A failed index or parse, or a watcher that stopped.
    pub(super) fn serve(mut self, queue: &Receiver<Change>) -> anyhow::Result<()> {
        let idle = Duration::from_secs(self.config.idle_shutdown_minutes.saturating_mul(60));
        let mut last_change = Instant::now();
        let mut next_crawl = Instant::now();
        loop {
            let deadline = if self.waiting() > 0 {
                Some(next_crawl)
            } else {
                last_change.checked_add(idle)
            };
            let received = match deadline {
                Some(deadline) => {
                    queue.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                }
                // An idle time too long to reach is never reached.
                None => queue.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(change) => {
                    self.index(&touched(change, queue))?;
                    last_change = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) if self.waiting() > 0 => {
                    next_crawl = Instant::now() + CRAWL_PERIOD;
                    self.crawl_batch()?;
                }
                Err(RecvTimeoutError::Timeout) => return Ok(()),
                Err(RecvTimeoutError::Disconnected) => bail!("the watcher stopped"),
            }
        }
    }

    /// Parses the next `index.crawl_files_per_second` waiting files on one thread.
    fn crawl_batch(&mut self) -> anyhow::Result<()> {
        let count = usize::try_from(self.config.crawl_files_per_second).unwrap_or(usize::MAX);
        let batch = self.crawl.next(count);
        let options = self.options(NonZeroUsize::MIN);
        parse_files(
            self.root,
            &self.grammars,
            &mut self.writer,
            &options,
            &batch,
        )
        .context("parsing waiting files")?;
        Ok(())
    }

    /// The options of an index or a parse on `threads` threads, timed now.
    fn options(&self, threads: NonZeroUsize) -> IndexOptions<'a> {
        IndexOptions {
            discover: DiscoverOptions {
                max_file_bytes: self.config.max_file_bytes,
                extra_ignores: &self.config.extra_ignores,
                global_gitignore: None,
            },
            parse_timeout: Duration::from_millis(self.config.parse_timeout_ms),
            threads,
            now_ms: now_ms(),
            mode: self.config.mode,
        }
    }
}

/// The paths `first` and every change already queued behind it name, in path order. A change to
/// everything names none: the index reads the whole tree in any case.
fn touched(first: Change, queue: &Receiver<Change>) -> Vec<RepoPath> {
    let mut touched = BTreeSet::new();
    for change in std::iter::once(first).chain(queue.try_iter()) {
        if let Change::Paths(paths) = change {
            touched.extend(paths);
        }
    }
    touched.into_iter().collect()
}

/// The time now in UTC milliseconds; a clock set before 1970 reads as 0.
pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}
