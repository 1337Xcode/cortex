//! The daemon's work loop: the one writer, the hook events and the watcher's changes its inbox
//! holds, the huge mode crawl between them, and the idle stop.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use cortex_core::{Config, Event, RepoPath};
use cortex_index::{
    Crawl, DiscoverOptions, Grammars, IndexOptions, IndexReport, index_paths, index_repository,
};
use cortex_learn::Recorder;
use cortex_store::{StoreError, Writer};

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
    /// What turns hook events into rows.
    recorder: Recorder,
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
            recorder: Recorder::new(config.store_prompt_excerpt),
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

    /// Records the events and indexes after the changes the inbox brings, crawls while nothing
    /// comes, and returns when asked to stop, or once nothing waits to be parsed and nothing came
    /// for `daemon.idle_shutdown_minutes`. A batch of events that cannot be recorded is counted
    /// and left; the daemon goes on.
    ///
    /// # Errors
    ///
    /// A failed index or parse, or a watcher that stopped.
    pub(crate) fn serve(mut self, inbox: &Inbox) -> anyhow::Result<()> {
        let idle = Duration::from_secs(self.config.idle_shutdown_minutes.saturating_mul(60));
        let mut last_activity = Instant::now();
        let mut next_crawl = Instant::now();
        loop {
            // An idle time too long to reach is never reached: the inbox is waited on for ever.
            let deadline = if self.waiting() > 0 {
                Some(next_crawl)
            } else {
                last_activity.checked_add(idle)
            };
            match inbox.wait(deadline) {
                Work::Stop => return Ok(()),
                Work::WatcherStopped => bail!("the watcher stopped"),
                Work::Events(events) => {
                    if self.record(&events).is_err() {
                        inbox.count_failed_batch();
                    }
                    last_activity = Instant::now();
                }
                Work::Changed(changed) => {
                    match changed {
                        Changed::Everything(touched) => drop(self.index(&touched)?),
                        Changed::Paths(paths) => self.update(&paths)?,
                    }
                    last_activity = Instant::now();
                }
                Work::Timeout if self.waiting() > 0 => {
                    next_crawl = Instant::now() + CRAWL_PERIOD;
                    self.crawl_batch()?;
                }
                Work::Timeout => return Ok(()),
            }
        }
    }

    /// Records `events` in one transaction. When any of them cannot be recorded, none is: the
    /// transaction is rolled back and the recorder, which may remember what was rolled back,
    /// starts afresh.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub(crate) fn record(&mut self, events: &[Event]) -> Result<(), StoreError> {
        let recorded = record_batch(&mut self.writer, &mut self.recorder, events);
        if recorded.is_err() {
            self.recorder = Recorder::new(self.config.store_prompt_excerpt);
        }
        recorded
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

/// Records `events` with `recorder` in one transaction of `writer`, all or none.
fn record_batch(
    writer: &mut Writer,
    recorder: &mut Recorder,
    events: &[Event],
) -> Result<(), StoreError> {
    let mut transaction = writer.transaction()?;
    for event in events {
        recorder.record(&mut transaction, event)?;
    }
    transaction.commit()
}

/// The time now in UTC milliseconds; a clock set before 1970 reads as 0.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cortex_core::{Agent, HookKind, Identifier, RepoId, Ulid};
    use cortex_store::WriterOptions;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// An event of `hook` from session `s1` at `ts_ms`, its id made from `seed`.
    fn event(hook: HookKind, ts_ms: i64, seed: &[u8]) -> Result<Event, Box<dyn std::error::Error>> {
        Ok(Event {
            event_id: Ulid::from_seed(u64::try_from(ts_ms)?, seed),
            ts_ms,
            agent: Agent::Claude,
            agent_version: None,
            session_id: Identifier::new("s1")?,
            parent_session_id: None,
            cwd: PathBuf::from("/repo"),
            repo_root: PathBuf::from("/repo"),
            hook,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: Some("go".to_owned()),
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
        })
    }

    /// A batch whose second prompt repeats the first prompt's event (so its task id is taken)
    /// cannot be recorded: none of it is, not even the session it started. The host then starts
    /// its recorder afresh, so the next batch registers the session again and is recorded whole.
    #[test]
    fn a_batch_that_fails_leaves_nothing_behind() -> TestResult {
        let root = tempfile::tempdir()?;
        let writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &RepoId::of(root.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        let config = Config::default();
        let discover = DiscoverOptions {
            max_file_bytes: config.max_file_bytes,
            extra_ignores: &[],
            global_gitignore: None,
        };
        let grammars = Arc::new(Grammars::load()?);
        let mut host = Host::new(root.path(), grammars, (&config, discover), writer);
        let prompt = event(HookKind::Prompt, 10, b"same")?;
        let failed = host.record(&[
            event(HookKind::SessionStart, 5, b"start")?,
            prompt.clone(),
            prompt,
        ]);
        assert!(failed.is_err());
        let session = Identifier::new("s1")?;
        assert_eq!(host.writer.queries().session(&session)?, None);
        assert_eq!(host.writer.queries().events_after(0, 10)?, []);
        host.record(&[event(HookKind::Prompt, 20, b"next")?])?;
        assert!(host.writer.queries().session(&session)?.is_some());
        let kinds: Vec<_> = host
            .writer
            .queries()
            .events_after(0, 10)?
            .into_iter()
            .map(|row| row.record.kind)
            .collect();
        assert_eq!(kinds, [cortex_core::EventKind::Prompt]);
        Ok(())
    }
}
