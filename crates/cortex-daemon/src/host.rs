//! The daemon's work loop: the one writer, the hook events and the watcher's changes its inbox
//! holds, the edits the watcher saw while a hook may still claim them, the huge mode crawl between
//! them, and the idle stop. While the writer indexes, the index lets it in between its write
//! transactions, and the events waiting and the edits due are recorded there.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use cortex_core::{Config, Event, RepoPath};
use cortex_index::{
    Crawl, DiscoverOptions, Grammars, IndexOptions, IndexReport, PAUSE_EVERY, Pause, index_paths,
    index_repository,
};
use cortex_learn::{Recorder, WatcherEdits};
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
    /// The edits the watcher saw, held until a hook can no longer claim them.
    edits: WatcherEdits,
    /// Where hook events, the watcher's changes and a stop arrive.
    inbox: &'a Inbox,
}

impl<'a> Host<'a> {
    /// A host over `writer` for the repository at `root`, with the settings in effect and the
    /// walk's options, recording events with `recorder` and taking its work from `inbox`, no file
    /// waiting yet.
    pub(crate) fn new(
        root: &'a Path,
        grammars: Arc<Grammars>,
        (config, discover): (&'a Config, DiscoverOptions<'a>),
        (writer, recorder, inbox): (Writer, Recorder, &'a Inbox),
    ) -> Self {
        Self {
            root,
            grammars,
            config,
            discover,
            writer,
            crawl: Crawl::default(),
            recorder,
            edits: WatcherEdits::default(),
            inbox,
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
        let report = self
            .indexing(threads, |root, grammars, writer, options| {
                index_repository(root, grammars, writer, options)
            })
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

    /// Records the events and indexes after the changes the inbox brings, records the edits the
    /// watcher saw once they are due, crawls while nothing comes, and returns when asked to stop,
    /// or once nothing waits to be parsed or recorded and neither a request nor a change came for
    /// `daemon.idle_shutdown_minutes`. A batch that cannot be recorded is counted and left; the
    /// daemon goes on.
    ///
    /// # Errors
    ///
    /// A failed index or parse, or a watcher that stopped.
    pub(crate) fn serve(mut self) -> anyhow::Result<()> {
        let inbox = self.inbox;
        let excerpt = self.config.store_prompt_excerpt;
        let idle = Duration::from_secs(self.config.idle_shutdown_minutes.saturating_mul(60));
        let mut last_activity = Instant::now();
        let mut next_crawl = Instant::now();
        loop {
            let parts = (&mut self.recorder, &mut self.edits);
            record_due_edits(&mut self.writer, parts, inbox, excerpt);
            last_activity = last_activity.max(inbox.last_request().unwrap_or(last_activity));
            // An idle time too long to reach is never reached: the inbox is waited on for ever.
            let idle_at = last_activity.checked_add(idle);
            match inbox.wait(self.deadline(next_crawl, idle_at)) {
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
                        Changed::Paths(paths, seen_ms) => self.update(&paths, seen_ms)?,
                    }
                    last_activity = Instant::now();
                }
                Work::Timeout if self.waiting() > 0 && Instant::now() >= next_crawl => {
                    next_crawl = Instant::now() + CRAWL_PERIOD;
                    self.crawl_batch()?;
                }
                Work::Timeout => {
                    // A request that came during the wait does not wake the writer: it counts now.
                    let requested = inbox.last_request().unwrap_or(last_activity);
                    let idle_at = last_activity.max(requested).checked_add(idle);
                    let is_idle = self.waiting() == 0 && self.edits.next_due_ms().is_none();
                    if is_idle && idle_at.is_some_and(|at| Instant::now() >= at) {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// When the work loop looks again if nothing arrives: at the next crawl while files wait to be
    /// parsed, else at `idle_at`, and sooner when a held edit falls due.
    fn deadline(&self, next_crawl: Instant, idle_at: Option<Instant>) -> Option<Instant> {
        let work = if self.waiting() > 0 {
            Some(next_crawl)
        } else {
            idle_at
        };
        let edit_due = self.edits.next_due_ms().map(|due_ms| {
            let wait_ms = u64::try_from(due_ms.saturating_sub(now_ms())).unwrap_or(0);
            Instant::now() + Duration::from_millis(wait_ms)
        });
        work.into_iter().chain(edit_due).min()
    }

    /// Records `events` in one transaction. When any of them cannot be recorded, none is: the
    /// transaction is rolled back and the recorder, which may remember what was rolled back,
    /// starts afresh.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub(crate) fn record(&mut self, events: &[Event]) -> Result<(), StoreError> {
        let excerpt = self.config.store_prompt_excerpt;
        record_batch(&mut self.writer, &mut self.recorder, excerpt, events)
    }

    /// Indexes the files at `paths`, which the watcher saw change at `seen_ms`, on every core, and
    /// holds the edits it found until a hook can no longer claim them.
    fn update(&mut self, paths: &[RepoPath], seen_ms: i64) -> anyhow::Result<()> {
        let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        let report = self
            .indexing(threads, |root, grammars, writer, options| {
                index_paths(root, grammars, writer, options, paths)
            })
            .context("indexing changed files")?;
        self.edits.hold(report.edited, seen_ms);
        Ok(())
    }

    /// Parses the next `index.crawl_files_per_second` waiting files on one thread.
    fn crawl_batch(&mut self) -> anyhow::Result<()> {
        let count = usize::try_from(self.config.crawl_files_per_second).unwrap_or(usize::MAX);
        let batch = self.crawl.next(count);
        self.indexing(NonZeroUsize::MIN, |root, grammars, writer, options| {
            index_paths(root, grammars, writer, options, &batch)
        })
        .context("parsing waiting files")?;
        Ok(())
    }

    /// Runs `index` with the options of an index on `threads` threads, timed now, and a pause that
    /// records the hook events waiting whenever the index lets the writer in. A batch the pause
    /// cannot record is counted, as one the work loop cannot.
    fn indexing<T>(
        &mut self,
        threads: NonZeroUsize,
        index: impl FnOnce(&Path, &Arc<Grammars>, &mut Writer, &IndexOptions<'_>) -> T,
    ) -> T {
        let options = self.options(threads);
        let (inbox, excerpt) = (self.inbox, self.config.store_prompt_excerpt);
        let parts = RefCell::new((&mut self.recorder, &mut self.edits));
        let pause = |writer: &mut Writer| {
            // INVARIANT: single-writer. Events are recorded through the writer the index lends,
            // between its transactions, on the writer's own thread. The index never calls the
            // pause from within it, so the recorder is always free here.
            if let Ok(mut parts) = parts.try_borrow_mut() {
                let (recorder, edits) = &mut *parts;
                record_waiting(writer, recorder, inbox, excerpt);
                record_due_edits(writer, (recorder, edits), inbox, excerpt);
            }
        };
        let options = IndexOptions {
            pause: Some(Pause::new(&pause, PAUSE_EVERY)),
            ..options
        };
        index(self.root, &self.grammars, &mut self.writer, &options)
    }

    /// The options of an index or a parse on `threads` threads, timed now, with no pause.
    fn options(&self, threads: NonZeroUsize) -> IndexOptions<'a> {
        IndexOptions {
            discover: self.discover,
            parse_timeout: Duration::from_millis(self.config.parse_timeout_ms),
            threads,
            now_ms: now_ms(),
            mode: self.config.mode,
            pause: None,
        }
    }
}

/// Loads every grammar on a thread of its own, recording the hook events that wait meanwhile every
/// [`PAUSE_EVERY`]: a debug build takes seconds to load them, and the events must not wait as long.
/// When the grammars cannot be had, the events waiting are recorded before the error is returned.
///
/// # Errors
///
/// A grammar that cannot be loaded, or a loader that stopped without a word.
pub(crate) fn load_grammars(
    writer: &mut Writer,
    recorder: &mut Recorder,
    inbox: &Inbox,
    excerpt: bool,
) -> anyhow::Result<Arc<Grammars>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || drop(sender.send(Grammars::load())));
    let loaded = loop {
        match receiver.recv_timeout(PAUSE_EVERY) {
            Ok(Ok(grammars)) => return Ok(Arc::new(grammars)),
            Ok(Err(error)) => break Err(error).context("loading the language grammars"),
            Err(RecvTimeoutError::Timeout) => record_waiting(writer, recorder, inbox, excerpt),
            Err(RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("loading the language grammars stopped"));
            }
        }
    };
    record_waiting(writer, recorder, inbox, excerpt);
    loaded
}

/// Records the hook events waiting in `inbox` with `recorder`, all or none; a batch that cannot be
/// recorded is counted.
pub(crate) fn record_waiting(
    writer: &mut Writer,
    recorder: &mut Recorder,
    inbox: &Inbox,
    excerpt: bool,
) {
    let events = inbox.take_events();
    if !events.is_empty() && record_batch(writer, recorder, excerpt, &events).is_err() {
        inbox.count_failed_batch();
    }
}

/// Records the edits the watcher saw that are due by now, all or none, after recording the hook
/// events waiting in `inbox`, so that a hook's report of the same edit claims it first. A batch
/// that cannot be recorded is counted, and the recorder starts afresh, as after events.
fn record_due_edits(
    writer: &mut Writer,
    (recorder, edits): (&mut Recorder, &mut WatcherEdits),
    inbox: &Inbox,
    excerpt: bool,
) {
    let due = edits.take_due(now_ms());
    if due.is_empty() {
        return;
    }
    record_waiting(writer, recorder, inbox, excerpt);
    let recorded = writer.transaction().and_then(|mut transaction| {
        for (path, seen_ms) in &due {
            recorder.record_watcher_edit(&mut transaction, path, *seen_ms)?;
        }
        transaction.commit()
    });
    if recorded.is_err() {
        *recorder = Recorder::new(excerpt);
        inbox.count_failed_batch();
    }
}

/// Records `events` with `recorder` in one transaction of `writer`. When any of them cannot be
/// recorded, none is: the transaction is rolled back and the recorder, which may remember what was
/// rolled back, starts afresh, keeping prompt excerpts as `excerpt` says.
///
/// # Errors
///
/// The store's error.
fn record_batch(
    writer: &mut Writer,
    recorder: &mut Recorder,
    excerpt: bool,
    events: &[Event],
) -> Result<(), StoreError> {
    let recorded = writer.transaction().and_then(|mut transaction| {
        for event in events {
            recorder.record(&mut transaction, event)?;
        }
        transaction.commit()
    });
    if recorded.is_err() {
        *recorder = Recorder::new(excerpt);
    }
    recorded
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

    use cortex_core::{Agent, EventKind, HookKind, Identifier, RepoId, Ulid};
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

    /// A writer for a store in `root`.
    fn writer(root: &Path) -> Result<Writer, Box<dyn std::error::Error>> {
        Ok(Writer::open(&WriterOptions {
            folder: &root.join(".cortex"),
            repo_id: &RepoId::of(root),
            mmap_bytes: 0,
            now_ms: 1,
        })?)
    }

    /// The walk's options under `config`.
    fn discover(config: &Config) -> DiscoverOptions<'_> {
        DiscoverOptions {
            max_file_bytes: config.max_file_bytes,
            extra_ignores: &[],
            global_gitignore: None,
        }
    }

    /// The kinds of the events stored, in order.
    fn stored_kinds(host: &Host<'_>) -> Result<Vec<EventKind>, Box<dyn std::error::Error>> {
        Ok(host
            .writer
            .queries()
            .events_after(0, 10)?
            .into_iter()
            .map(|row| row.record.kind)
            .collect())
    }

    /// Writes 600 Python modules of one function each under `root`: three batches of files.
    fn write_modules(root: &Path) -> TestResult {
        for number in 0..600 {
            let source = format!("def f{number}():\n    pass\n");
            std::fs::write(root.join(format!("m{number}.py")), source)?;
        }
        Ok(())
    }

    /// Hook events that wait when the writer starts an index of 600 files are recorded during it,
    /// when the index lets the writer in after its walk, however quick the index: when it ends
    /// nothing waits, and the store holds them.
    #[test]
    fn events_are_recorded_while_indexing() -> TestResult {
        let root = tempfile::tempdir()?;
        write_modules(root.path())?;
        let config = Config::default();
        let grammars = Arc::new(Grammars::load()?);
        let inbox = Inbox::default();
        let parts = (writer(root.path())?, Recorder::new(false), &inbox);
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        let report = host.index(&[])?;
        assert_eq!(report.files, 600);
        assert_eq!(inbox.counts().events_waiting, 0);
        assert_eq!(
            stored_kinds(&host)?,
            [EventKind::SessionStart, EventKind::Prompt]
        );
        Ok(())
    }

    /// A batch the index's pause cannot record, its second prompt repeating the first prompt's
    /// event, is counted and left, and the index goes on: all 600 files are written and nothing of
    /// the batch is stored. The pause started the host's own recorder afresh, so the next event
    /// registers its session again and is recorded.
    #[test]
    fn a_batch_the_pause_cannot_record_is_counted() -> TestResult {
        let root = tempfile::tempdir()?;
        write_modules(root.path())?;
        let config = Config::default();
        let grammars = Arc::new(Grammars::load()?);
        let inbox = Inbox::default();
        let parts = (writer(root.path())?, Recorder::new(false), &inbox);
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        let prompt = event(HookKind::Prompt, 10, b"same")?;
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(prompt.clone());
        inbox.push_event(prompt);
        let report = host.index(&[])?;
        assert_eq!(report.files, 600);
        let counts = inbox.counts();
        assert_eq!((counts.events_waiting, counts.failed_batches), (0, 1));
        assert_eq!(host.writer.queries().events_after(0, 10)?, []);
        host.record(&[event(HookKind::Prompt, 20, b"next")?])?;
        let session = Identifier::new("s1")?;
        assert!(host.writer.queries().session(&session)?.is_some());
        assert_eq!(stored_kinds(&host)?, [EventKind::Prompt]);
        Ok(())
    }

    /// A batch whose second prompt repeats the first prompt's event (so its task id is taken)
    /// cannot be recorded: none of it is, not even the session it started. The host then starts
    /// its recorder afresh, so the next batch registers the session again and is recorded whole.
    #[test]
    fn a_batch_that_fails_leaves_nothing_behind() -> TestResult {
        let root = tempfile::tempdir()?;
        let config = Config::default();
        let grammars = Arc::new(Grammars::load()?);
        let inbox = Inbox::default();
        let parts = (writer(root.path())?, Recorder::new(false), &inbox);
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
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
        assert_eq!(stored_kinds(&host)?, [EventKind::Prompt]);
        Ok(())
    }
}
