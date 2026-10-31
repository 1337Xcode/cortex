//! The daemon's work loop: the one writer, the changes the watcher puts in its inbox, the huge
//! mode crawl between changes, and the idle stop.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use cortex_core::{Config, RepoPath};
use cortex_index::{
    Crawl, DiscoverOptions, Grammars, IndexOptions, IndexReport, index_paths, index_repository,
};
use cortex_store::Writer;

use crate::inbox::{Changed, Inbox, Work};

/// How often the crawler takes a batch of `index.crawl_files_per_second` files.
const CRAWL_PERIOD: Duration = Duration::from_secs(1);

/// The daemon's state between changes: the writer and the files waiting to be parsed.
pub(crate) struct Host<'a> {
    /// The repository's root folder.
    root: &'a Path,
    /// Every language's grammar, loaded once.
    grammars: Arc<Grammars>,
    /// The settings in effect.
    config: &'a Config,
    /// Which files the walk finds: the size limit and the ignore patterns, the user's global
    /// gitignore among them.
    discover: DiscoverOptions<'a>,
    /// The store's one writer.
    writer: Writer,
    /// The files huge mode took inventory of, in the order they are parsed.
    crawl: Crawl,
}

impl<'a> Host<'a> {
    /// A host over `writer` for the repository at `root`, with the settings in effect and the
    /// walk's options, nothing waiting yet.
    pub(crate) fn new(
        root: &'a Path,
        grammars: Arc<Grammars>,
        (config, discover): (&'a Config, DiscoverOptions<'a>),
        writer: Writer,
    ) -> Self {
        Self {
            root,
            grammars,
            config,
            discover,
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
    pub(crate) fn index(&mut self, touched: &[RepoPath]) -> anyhow::Result<IndexReport> {
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
    pub(crate) fn waiting(&self) -> usize {
        self.crawl.waiting()
    }

    /// Indexes after each change the inbox brings, crawls while no change comes, and returns when
    /// asked to stop, or once nothing waits to be parsed and no change came for
    /// `daemon.idle_shutdown_minutes`.
    ///
    /// # Errors
    ///
    /// A failed index or parse, or a watcher that stopped.
    pub(crate) fn serve(mut self, inbox: &Inbox) -> anyhow::Result<()> {
        let idle = Duration::from_secs(self.config.idle_shutdown_minutes.saturating_mul(60));
        let mut last_change = Instant::now();
        let mut next_crawl = Instant::now();
        loop {
            // An idle time too long to reach is never reached: the inbox is waited on for ever.
            let deadline = if self.waiting() > 0 {
                Some(next_crawl)
            } else {
                last_change.checked_add(idle)
            };
            match inbox.wait(deadline) {
                Work::Stop => return Ok(()),
                Work::WatcherStopped => bail!("the watcher stopped"),
                Work::Changed(changed) => {
                    match changed {
                        Changed::Everything(touched) => drop(self.index(&touched)?),
                        Changed::Paths(paths) => self.update(&paths)?,
                    }
                    last_change = Instant::now();
                }
                Work::Timeout if self.waiting() > 0 => {
                    next_crawl = Instant::now() + CRAWL_PERIOD;
                    self.crawl_batch()?;
                }
                Work::Timeout => return Ok(()),
            }
        }
    }

    /// Indexes the files at `paths`, which changed, on every core.
    fn update(&mut self, paths: &[RepoPath]) -> anyhow::Result<()> {
        let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        let options = self.options(threads);
        index_paths(self.root, &self.grammars, &mut self.writer, &options, paths)
            .context("indexing changed files")?;
        Ok(())
    }

    /// Parses the next `index.crawl_files_per_second` waiting files on one thread.
    fn crawl_batch(&mut self) -> anyhow::Result<()> {
        let count = usize::try_from(self.config.crawl_files_per_second).unwrap_or(usize::MAX);
        let batch = self.crawl.next(count);
        let options = self.options(NonZeroUsize::MIN);
        index_paths(
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
            discover: self.discover,
            parse_timeout: Duration::from_millis(self.config.parse_timeout_ms),
            threads,
            now_ms: now_ms(),
            mode: self.config.mode,
        }
    }
}

/// The time now in UTC milliseconds; a clock set before 1970 reads as 0.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}
