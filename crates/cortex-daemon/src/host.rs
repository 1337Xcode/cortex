//! The daemon's work loop: the one writer, the hook events, the watcher's changes and the
//! miner's findings its inbox holds, the edits the watcher saw while a hook may still claim them,
//! what the tasks that end teach and the usage the stops' transcripts report, the huge mode crawl
//! between them, and the idle stop. While the writer indexes or stores a mining, the work lets it
//! in between its write transactions, and the events waiting and the edits due are recorded
//! there; the tasks they end and the transcripts they name wait until the writer is free. After
//! every index the facts are reviewed, so a fact whose code the index found changed turns
//! suspect.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, bail};
use cortex_core::ipc::UpkeepReport;
use cortex_core::{Config, Event, HookKind, RepoPath, unix_millis};
use cortex_git::mine;
use cortex_index::{
    Crawl, DiscoverOptions, Grammars, IndexOptions, IndexReport, OnChanged, index_paths,
    index_repository,
};
use cortex_learn::{Recorder, TaskState, review_facts};
use cortex_store::{ChangeKind, PAUSE_EVERY, Pause, StoreError, SymbolChange, WriteTx, Writer};

use crate::asked::{AT_WORK_WITHIN_MS, AskJob, AtWork};
use crate::given::{Given, Notices};
use crate::graph::Snapshot;
use crate::inbox::{Changed, Inbox, MiningJob, Work};
use crate::meter::Transcript;
use crate::pending::Pending;
use crate::positions;
use crate::signatures::Signatures;
use crate::upkeep::{Ended, LOG_LIMIT_BYTES, Upkeep, UpkeepClock, UpkeepSettings};

/// How often the crawler takes a batch of `index.crawl_files_per_second` files.
const CRAWL_PERIOD: Duration = Duration::from_secs(1);

/// How often the writer ends the tasks of sessions gone quiet: often enough that a handoff is
/// left within a minute of its session counting as done, rarely enough to cost nothing.
const QUIET_SWEEP_PERIOD: Duration = Duration::from_secs(60);

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
    /// The tasks that ended and wait to be learned from, and the transcripts that wait to be read.
    pending: Pending,
    /// Where hook events, the watcher's changes and a stop arrive.
    inbox: &'a Inbox,
    /// The graph retrieval spreads over, told when a write changes what it holds.
    snapshot: &'a Snapshot,
    /// Where the signature changes an index finds are kept a short while, for an edit's warnings.
    signatures: &'a Signatures,
    /// Where upkeep is between runs.
    upkeep: Upkeep,
    /// When upkeep is due.
    upkeep_clock: UpkeepClock,
}

impl<'a> Host<'a> {
    /// A host over `writer` for the repository at `root`, with the settings in effect and the
    /// walk's options, recording events with `recorder`, learning from the tasks they end and
    /// reading the transcripts they name with `pending`, taking its work from `inbox`, telling
    /// `snapshot` when a write changes the graph and keeping in `signatures` the signature changes
    /// its indexes find, no file waiting yet.
    pub(crate) fn new(
        root: &'a Path,
        grammars: Arc<Grammars>,
        (config, discover): (&'a Config, DiscoverOptions<'a>),
        (writer, recorder, pending, (inbox, snapshot, signatures)): (
            Writer,
            Recorder,
            Pending,
            (&'a Inbox, &'a Snapshot, &'a Signatures),
        ),
    ) -> Self {
        // Upkeep carries on where an earlier daemon left it; a store that cannot say where starts
        // it at its first step, which redoes nothing a pass would not.
        let upkeep = Upkeep::resumed(writer.queries()).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "upkeep's place could not be read: it starts over");
            Upkeep::default()
        });
        Self {
            root,
            grammars,
            config,
            discover,
            writer,
            crawl: Crawl::default(),
            recorder,
            pending,
            inbox,
            snapshot,
            signatures,
            upkeep,
            upkeep_clock: UpkeepClock::new(Instant::now()),
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
    /// watcher saw once they are due, learns from the tasks that ended and reads the transcripts
    /// the stops named, crawls while nothing comes,
    /// and returns when asked to stop, or once nothing waits to be parsed or recorded and neither a
    /// request nor a change came for `daemon.idle_shutdown_minutes`. A batch that cannot be
    /// recorded is counted, with its error, and left; the daemon goes on. However it ends, the
    /// inbox then takes no more events, the events it took and the edits still held are recorded,
    /// the tasks that ended are learned from and the transcripts named are read: an event a hook
    /// was answered for is not lost to a stop.
    ///
    /// # Errors
    ///
    /// A failed index or parse, or a watcher that stopped.
    pub(crate) fn serve(mut self) -> anyhow::Result<()> {
        let served = self.work();
        self.close();
        served
    }

    /// Closes the inbox to events and records what it took, and the edits the watcher saw that
    /// are still held, then learns from the tasks waiting and reads the transcripts waiting: what
    /// the daemon does last, however it stops.
    pub(crate) fn close(&mut self) {
        let recorded = close(&mut self.writer, &mut self.recorder, self.inbox);
        self.pending.wait(recorded);
        // Every edit still held is recorded too: a hook has had its chance to claim most of the
        // hold, and a stop must not lose them.
        let recorded = record_due_edits(&mut self.writer, &mut self.recorder, self.inbox, i64::MAX);
        self.pending.wait(recorded);
        self.settle();
    }

    /// Learns from the tasks that ended and wait and reads the transcripts that wait, recording
    /// the events that wait between them. What a task teaches changes the graph.
    fn settle(&mut self) {
        let is_learned = self.pending.settle(
            (&mut self.writer, &mut self.recorder),
            self.inbox,
            self.config,
        );
        if is_learned {
            self.snapshot.changed();
        }
    }

    /// The work of [`Host::serve`] until it is asked to stop, is idle long enough or fails.
    fn work(&mut self) -> anyhow::Result<()> {
        let inbox = self.inbox;
        let idle = Duration::from_secs(self.config.idle_shutdown_minutes.saturating_mul(60));
        let mut last_activity = Instant::now();
        let mut next_crawl = Instant::now();
        // At once, then every period: a daemon that stopped idle missed sessions going quiet.
        let mut next_sweep = Instant::now();
        loop {
            let recorded = record_due_edits(&mut self.writer, &mut self.recorder, inbox, now_ms());
            self.pending.wait(recorded);
            if Instant::now() >= next_sweep {
                next_sweep = Instant::now() + QUIET_SWEEP_PERIOD;
                self.end_quiet_tasks();
            }
            self.settle();
            let is_free = self.pending.is_idle() && self.waiting() == 0;
            if is_free
                && self
                    .upkeep_clock
                    .due_at(Instant::now())
                    .is_some_and(|at| Instant::now() >= at)
            {
                self.run_upkeep();
            }
            last_activity = last_activity.max(inbox.last_request().unwrap_or(last_activity));
            // An idle time too long to reach is never reached: the inbox is waited on for ever.
            let idle_at = last_activity.checked_add(idle);
            match inbox.wait(self.deadline(next_crawl, idle_at, next_sweep)) {
                Work::Stop => {
                    tracing::info!("stopping: asked to");
                    return Ok(());
                }
                Work::WatcherStopped => bail!("the watcher stopped"),
                Work::Events(events) => {
                    if let Err(error) = self.record(&events) {
                        count_failed(inbox, "hook events", &error);
                    }
                    last_activity = Instant::now();
                    let has_ended = events
                        .iter()
                        .any(|event| event.hook == HookKind::SessionEnd);
                    self.upkeep_clock.heard(last_activity, has_ended);
                }
                Work::Given(given) => record_given(&mut self.writer, inbox, &given),
                Work::Changed(changed) => {
                    match changed {
                        Changed::Everything(touched) => drop(self.index(&touched)?),
                        Changed::Paths(changes) => self.update(&changes)?,
                    }
                    last_activity = Instant::now();
                }
                // Mining is the daemon's own work, not a user's: it does not put off the idle stop.
                Work::Mining(job) => self.store_mining(*job),
                Work::Ask(job) => {
                    self.carry_out_ask(*job);
                    last_activity = Instant::now();
                }
                Work::Upkeep => self.upkeep_clock.ask(),
                Work::Timeout if self.waiting() > 0 && Instant::now() >= next_crawl => {
                    next_crawl = Instant::now() + CRAWL_PERIOD;
                    self.crawl_batch()?;
                }
                Work::Timeout => {
                    // A request that came during the wait does not wake the writer: it counts now.
                    let requested = inbox.last_request().unwrap_or(last_activity);
                    let idle_at = last_activity.max(requested).checked_add(idle);
                    let is_idle =
                        self.waiting() == 0 && self.recorder.next_watcher_edit_due_ms().is_none();
                    if is_idle && idle_at.is_some_and(|at| Instant::now() >= at) {
                        let idle_minutes = self.config.idle_shutdown_minutes;
                        tracing::info!(idle_minutes, "stopping: idle");
                        return Ok(());
                    }
                }
            }
        }
    }

    /// When the work loop looks again if nothing arrives: at once while ended tasks wait to be
    /// learned from or transcripts to be read, at the next crawl while files wait to be parsed,
    /// else at `idle_at`, and sooner when a held edit falls due or the next sweep for quiet
    /// sessions, `next_sweep`, comes.
    fn deadline(
        &self,
        next_crawl: Instant,
        idle_at: Option<Instant>,
        next_sweep: Instant,
    ) -> Option<Instant> {
        if !self.pending.is_idle() {
            return Some(Instant::now());
        }
        let work = if self.waiting() > 0 {
            Some(next_crawl)
        } else {
            idle_at
        };
        let edit_due = self.recorder.next_watcher_edit_due_ms().map(|due_ms| {
            let wait_ms = u64::try_from(due_ms.saturating_sub(now_ms())).unwrap_or(0);
            Instant::now() + Duration::from_millis(wait_ms)
        });
        // Upkeep waits for the files waiting to be parsed: it is not due while they are.
        let upkeep_due = self
            .upkeep_clock
            .due_at(Instant::now())
            .filter(|_| self.waiting() == 0);
        work.into_iter()
            .chain(edit_due)
            .chain(upkeep_due)
            .chain([next_sweep])
            .min()
    }

    /// Runs upkeep from where it last stopped, within its budget, stopping when other work waits
    /// for the writer; what it removed is logged, and the graph told when links or interest went.
    /// A failure is counted, and the next quiet spell runs it again.
    fn run_upkeep(&mut self) {
        let settings = UpkeepSettings {
            link_decay: self.config.link_decay,
            interest_decay: self.config.interest_decay,
            longest_hold_ms: self.inbox.position_timing().longest_ms(),
            log_limit_bytes: LOG_LIMIT_BYTES,
        };
        let inbox = self.inbox;
        let asked_before = inbox.last_request();
        let should_stop = || inbox.should_upkeep_stop(asked_before);
        let deadline = self.upkeep_clock.run_deadline(Instant::now());
        let ran = self.upkeep.run(
            &mut self.writer,
            (now_ms(), &settings),
            (deadline, &should_stop),
        );
        match ran {
            Ok((swept, ended)) => {
                tracing::info!(
                    links = swept.links,
                    transitions = swept.transitions,
                    interest = swept.interest,
                    facts = swept.facts,
                    events = swept.events,
                    injections = swept.injections,
                    episodes = swept.episodes,
                    positions = swept.positions,
                    pages = swept.pages,
                    ended = ?ended,
                    "upkeep ran"
                );
                if swept.links > 0 || swept.interest > 0 {
                    self.snapshot.changed();
                }
                inbox.note_upkeep(UpkeepReport {
                    at_ms: now_ms(),
                    links: swept.links,
                    interest: swept.interest,
                    facts: swept.facts,
                    events: swept.events,
                    episodes: swept.episodes,
                    positions: swept.positions,
                    pages: swept.pages,
                    is_done: ended == Ended::Done,
                });
                self.upkeep_clock.ran(ended, Instant::now());
            }
            Err(error) => {
                count_failed(inbox, "upkeep", &error);
                self.upkeep_clock.ran(Ended::OutOfTime, Instant::now());
            }
        }
    }

    /// Ends the tasks of sessions quiet for `coordination.handoff_idle_minutes`, so that what they
    /// left is handed on, and leaves them to be learned from. A failure is counted, and the next
    /// sweep tries again.
    fn end_quiet_tasks(&mut self) {
        let quiet_ms = self.inbox.position_timing().handoff_idle_ms;
        let since_ms = now_ms().saturating_sub(quiet_ms);
        let recorder = &mut self.recorder;
        let ended = self.writer.transaction().and_then(|mut transaction| {
            let ended = recorder.end_quiet_tasks(&mut transaction, since_ms)?;
            transaction.commit()?;
            Ok(ended)
        });
        match ended {
            Ok(ended) => self.pending.wait(Recorded {
                ended,
                transcripts: Vec::new(),
            }),
            Err(error) => {
                self.recorder.forget_stored();
                count_failed(self.inbox, "ending the tasks of quiet sessions", &error);
            }
        }
    }

    /// Records `events` in one transaction, the tasks they end left to wait to be learned from
    /// and the transcripts they name to be read. When any of them cannot be recorded, none is:
    /// the transaction is rolled back and the recorder forgets what it took as stored.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub(crate) fn record(&mut self, events: &[Event]) -> Result<(), StoreError> {
        let recorded = record_batch(&mut self.writer, &mut self.recorder, events, self.inbox)?;
        self.pending.wait(recorded);
        Ok(())
    }

    /// Indexes the files the watcher saw change, each with when a change first named it, on every
    /// core, and has the recorder hold the edits found, with what each file held before, until a
    /// hook can no longer claim them. An edit keeps the time its file, or the nearest folder above
    /// it, was named; one the changes do not name, found by an index of the whole repository the
    /// update fell back to, is taken as seen now.
    fn update(&mut self, changes: &[(RepoPath, i64)]) -> anyhow::Result<()> {
        let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        let paths: Vec<RepoPath> = changes.iter().map(|(path, _)| path.clone()).collect();
        let report = self
            .indexing(threads, |root, grammars, writer, options| {
                index_paths(root, grammars, writer, options, &paths)
            })
            .context("indexing changed files")?;
        let named: BTreeMap<RepoPath, i64> = changes.iter().cloned().collect();
        let now = now_ms();
        for edit in report.edited {
            let path = edit.path;
            let seen_ms = std::iter::successors(Some(path.clone()), RepoPath::parent)
                .find_map(|place| named.get(&place).copied())
                .unwrap_or(now);
            self.recorder
                .hold_watcher_edits([(path, edit.before)], seen_ms);
        }
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

    /// Runs `index` with the options of an index on `threads` threads, timed now, and the pause
    /// of [`Host::with_pause`], the recorder told while it runs so that a hook's edit recorded in a
    /// pause takes nothing from a row the index wrote as what its file held before. However it
    /// went, the graph is rebuilt from what it wrote. Each stretch of its writes leaves the
    /// signature changes it found with the signatures kept for an edit's warnings before it is
    /// committed, so that warnings that find a file's new rows know what changed in it.
    fn indexing<E>(
        &mut self,
        threads: NonZeroUsize,
        index: impl FnOnce(
            &Path,
            &Arc<Grammars>,
            &mut Writer,
            &IndexOptions<'_>,
        ) -> Result<IndexReport, E>,
    ) -> Result<IndexReport, E> {
        let options = self.options(threads);
        let (root, grammars) = (self.root, Arc::clone(&self.grammars));
        let signatures = self.signatures;
        let keep = |changes: &[SymbolChange]| {
            let changed = changes
                .iter()
                .filter(|change| change.kind == ChangeKind::Signature)
                .map(|change| change.symbol);
            signatures.note(changed, now_ms());
        };
        self.recorder.index_started(options.now_ms);
        let indexed = self.with_pause(|writer, pause| {
            let options = IndexOptions {
                pause: Some(pause),
                on_changed: Some(OnChanged(&keep)),
                ..options
            };
            index(root, &grammars, writer, &options)
        });
        self.recorder.index_done();
        self.review_facts();
        self.snapshot.changed();
        indexed
    }

    /// Makes suspect, in a transaction of its own, the facts whose code changed or is gone, and
    /// retires the facts suspect long enough. A review that fails is logged; the next index
    /// reviews again, and a fact is checked against its code again whenever it is rendered.
    fn review_facts(&mut self) {
        let reviewed = self.writer.transaction().and_then(|mut transaction| {
            let reviewed = review_facts(&mut transaction, now_ms())?;
            transaction.commit()?;
            Ok(reviewed)
        });
        match reviewed {
            Ok(reviewed) => tracing::debug!(
                suspected = reviewed.suspected.len(),
                retired = reviewed.retired.len(),
                "facts reviewed"
            ),
            Err(error) => tracing::warn!(error = %error, "facts could not be reviewed"),
        }
    }

    /// Stores what the miner found in turns, with the pause of [`Host::with_pause`], and tells the
    /// miner how that went. A mining that cannot be stored leaves no mark, so the next is whole.
    /// The graph is told when the write begins and ends, so that it takes co-change links only
    /// from a complete mining.
    fn store_mining(&mut self, job: MiningJob) {
        let writing = self.snapshot.mining_write();
        let stored = self
            .with_pause(|writer, pause| mine::write(writer, &job.mining, now_ms(), Some(pause)));
        drop(writing);
        if let Err(error) = &stored {
            tracing::warn!(error = %error, "a mining could not be stored");
        }
        job.answer(stored.map_err(|error| error.to_string()));
    }

    /// Carries out a request of a person or an agent in a transaction of its own, answered once
    /// it is committed. A store error is the answer, and counted.
    fn carry_out_ask(&mut self, job: AskJob) {
        let now = now_ms();
        let at_work = AtWork {
            latest: self.recorder.session_at_work(now, AT_WORK_WITHIN_MS),
            sole: self.recorder.sole_session_at_work(now, AT_WORK_WITHIN_MS),
        };
        let timing = self.inbox.position_timing();
        if let Err(error) = job.carry_out(&mut self.writer, (now, &at_work, &timing)) {
            count_failed(self.inbox, "a request asked of the writer", &error);
        }
    }

    /// Runs `work` with the writer and a pause that records the hook events waiting, and the edits
    /// due, whenever the work lets the writer in between its transactions; the tasks they end and
    /// the transcripts they name wait until the work is done. A batch the pause cannot record is
    /// counted, as one the work loop cannot.
    fn with_pause<T>(&mut self, work: impl FnOnce(&mut Writer, Pause<'_>) -> T) -> T {
        let inbox = self.inbox;
        let recorder = RefCell::new(&mut self.recorder);
        let pending = RefCell::new(&mut self.pending);
        let pause = |writer: &mut Writer| {
            // INVARIANT: single-writer. Events are recorded through the writer the work lends,
            // between its transactions, on the writer's own thread. The work never calls the
            // pause from within it, so the recorder and what is pending are always free here.
            if let (Ok(mut recorder), Ok(mut pending)) =
                (recorder.try_borrow_mut(), pending.try_borrow_mut())
            {
                let recorded = record_waiting(writer, &mut recorder, inbox);
                pending.wait(recorded);
                let recorded = record_due_edits(writer, &mut recorder, inbox, now_ms());
                pending.wait(recorded);
            }
        };
        work(&mut self.writer, Pause::new(&pause, PAUSE_EVERY))
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
            on_changed: None,
        }
    }
}

/// Loads every grammar on a thread of its own, recording the hook events that wait meanwhile every
/// [`PAUSE_EVERY`]: a debug build takes seconds to load them, and the events must not wait as long.
/// The tasks they end and the transcripts they name wait in `pending`. When the grammars cannot be
/// had, the events waiting are recorded before the error is returned.
///
/// # Errors
///
/// A grammar that cannot be loaded, or a loader that stopped without a word.
pub(crate) fn load_grammars(
    (writer, recorder): (&mut Writer, &mut Recorder),
    pending: &mut Pending,
    inbox: &Inbox,
) -> anyhow::Result<Arc<Grammars>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || drop(sender.send(Grammars::load())));
    let loaded = loop {
        match receiver.recv_timeout(PAUSE_EVERY) {
            Ok(Ok(grammars)) => return Ok(Arc::new(grammars)),
            Ok(Err(error)) => break Err(error).context("loading the language grammars"),
            Err(RecvTimeoutError::Timeout) => pending.wait(record_waiting(writer, recorder, inbox)),
            Err(RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("loading the language grammars stopped"));
            }
        }
    };
    pending.wait(record_waiting(writer, recorder, inbox));
    loaded
}

/// Closes `inbox` to events and records with `recorder` the events it took: what the daemon does
/// last once its writer is open, however it stops, so that no event a hook was answered for is
/// lost. What they left to do.
pub(crate) fn close(writer: &mut Writer, recorder: &mut Recorder, inbox: &Inbox) -> Recorded {
    inbox.close();
    record_waiting(writer, recorder, inbox)
}

/// Records the hook events waiting in `inbox` with `recorder`, all or none, and returns what they
/// left to do; a batch that cannot be recorded is counted, with its error, and leaves nothing.
pub(crate) fn record_waiting(
    writer: &mut Writer,
    recorder: &mut Recorder,
    inbox: &Inbox,
) -> Recorded {
    let (events, given) = inbox.take_waiting();
    let recorded = if events.is_empty() {
        Recorded::default()
    } else {
        record_batch(writer, recorder, &events, inbox).unwrap_or_else(|error| {
            count_failed(inbox, "hook events", &error);
            Recorded::default()
        })
    };
    // After the events: what was given answered an event recorded by now, with its task and its
    // session.
    record_given(writer, inbox, &given);
    recorded
}

/// Records `given`, what agents were given, with `writer`, in one transaction; a row the store
/// refuses (its event's batch not recorded, so its task missing) is counted, with its error, and
/// the rest kept.
fn record_given(writer: &mut Writer, inbox: &Inbox, given: &[Given]) {
    if given.is_empty() {
        return;
    }
    let recorded = writer.transaction().and_then(|mut transaction| {
        for record in given {
            match record {
                Given::Injection(injection) => {
                    if let Err(error) = transaction.record_injection(injection) {
                        count_failed(inbox, "an injection", &error);
                    }
                }
                Given::Warnings(rows, text) => {
                    if let Err(error) = transaction.record_injection(text) {
                        count_failed(inbox, "an injection", &error);
                    }
                    for row in rows {
                        if let Err(error) = transaction.record_warning(row) {
                            count_failed(inbox, "a warning", &error);
                        }
                    }
                }
                Given::Notices(notices) => record_notices(&mut transaction, inbox, notices),
                Given::Handoff(handoff, by, text) => {
                    if let Err(error) = transaction.record_injection(text) {
                        count_failed(inbox, "an injection", &error);
                    }
                    if let Err(error) = transaction.close_handoff(*handoff, by) {
                        count_failed(inbox, "a handoff given", &error);
                    }
                }
                Given::Facts(task, facts) => {
                    if let Err(error) = transaction.give_facts(*task, facts) {
                        count_failed(inbox, "the facts given", &error);
                    }
                }
            }
        }
        transaction.commit()
    });
    if let Err(error) = recorded {
        count_failed(inbox, "what was given", &error);
    }
}

/// Records in `transaction` what an answer did with coordination notices: the waiting ones it gave
/// marked delivered, the lines it told kept as delivered, the text they came to as an injection,
/// and each notice it queued for another session, unless that session was sent or given one about
/// the same lately. A row the store refuses is counted, with its error, and the rest kept.
fn record_notices(transaction: &mut WriteTx<'_>, inbox: &Inbox, notices: &Notices) {
    for id in &notices.delivered {
        if let Err(error) = transaction.deliver_notice(*id, notices.at_ms) {
            count_failed(inbox, "a notice delivered", &error);
        }
    }
    for told in &notices.told {
        if let Err(error) = transaction.record_told(told) {
            count_failed(inbox, "a notice told", &error);
        }
    }
    if let Some(text) = &notices.text
        && let Err(error) = transaction.record_injection(text)
    {
        count_failed(inbox, "an injection", &error);
    }
    for queued in &notices.queued {
        let unless_since_ms = queued.at_ms.saturating_sub(notices.repeat_after_ms);
        if let Err(error) = transaction.queue_notice(queued, unless_since_ms) {
            count_failed(inbox, "a notice queued", &error);
        }
    }
}

/// Records the edits the watcher saw that are due by `now_ms`, all or none, after recording the
/// hook events waiting in `inbox`, while the edits are still held: a hook's report of the same edit
/// claims it first, and takes from it what the file held before. A batch that cannot be recorded
/// is counted, with its error, and the recorder forgets what it took as stored, as after events.
/// What the hook events left to do: an edit the watcher saw leaves nothing.
fn record_due_edits(
    writer: &mut Writer,
    recorder: &mut Recorder,
    inbox: &Inbox,
    now_ms: i64,
) -> Recorded {
    if recorder
        .next_watcher_edit_due_ms()
        .is_none_or(|due_ms| due_ms > now_ms)
    {
        return Recorded::default();
    }
    let recorded = record_waiting(writer, recorder, inbox);
    let due = recorder.take_due_watcher_edits(now_ms);
    let written = writer.transaction().and_then(|mut transaction| {
        for edit in &due {
            recorder.record_watcher_edit(&mut transaction, edit)?;
        }
        transaction.commit()
    });
    if let Err(error) = written {
        recorder.forget_stored();
        count_failed(inbox, "the edits the watcher saw", &error);
    }
    recorded
}

/// Counts in `inbox` a batch of `what` the writer could not record, the store's `error` kept as
/// the reason and logged. The store's errors name what it was doing and the file, row or setting
/// involved, and carry no stored text but paths.
pub(crate) fn count_failed(inbox: &Inbox, what: &str, error: &StoreError) {
    tracing::warn!(what, error = %error, "a batch could not be recorded");
    inbox.count_failed_batch(&format!("recording {what} failed: {error}"), now_ms());
}

/// Records `events` with `recorder` in one transaction of `writer`, with the positions they give
/// their sessions, kept by the timing `inbox` holds, and returns what they left to do: the tasks
/// they ended and the transcripts they named, each in order. When any of them cannot be recorded,
/// none is: the transaction is rolled back, the recorder forgets what it took as stored, which may
/// include what was rolled back, no task has ended and no transcript waits. A position that cannot
/// be kept is counted in `inbox`, and the events are recorded all the same.
///
/// # Errors
///
/// The store's error.
fn record_batch(
    writer: &mut Writer,
    recorder: &mut Recorder,
    events: &[Event],
    inbox: &Inbox,
) -> Result<Recorded, StoreError> {
    let timing = inbox.position_timing();
    let mut ended = Vec::new();
    let recorded = writer.transaction().and_then(|mut transaction| {
        for event in events {
            ended.extend(recorder.record(&mut transaction, event)?);
            // A position serves coordination, not the event's record: one that cannot be kept is
            // counted, and the batch goes on.
            if let Err(error) = positions::keep(&mut transaction, event, &timing) {
                count_failed(inbox, "a session's position", &error);
            }
        }
        transaction.commit()
    });
    match recorded {
        Ok(()) => Ok(Recorded {
            ended,
            transcripts: events.iter().filter_map(Transcript::named_by).collect(),
        }),
        Err(error) => {
            recorder.forget_stored();
            Err(error)
        }
    }
}

/// What a batch of hook events leaves for the writer once it is recorded.
#[derive(Debug, Default)]
pub(crate) struct Recorded {
    /// The tasks the events ended, to learn from, in order.
    pub(crate) ended: Vec<TaskState>,
    /// The transcripts the events named, to read, in order.
    pub(crate) transcripts: Vec<Transcript>,
}

/// The time now in UTC milliseconds; a clock set before 1970 reads as 0.
pub(crate) fn now_ms() -> i64 {
    unix_millis(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use cortex_core::{
        Agent, ContentHash, EventKind, EventSource, FactKind, FactSource, FactStatus, FileContent,
        HookKind, Identifier, LineRange, RepoId, TaskOutcome, ToolKind, ToolResult, ToolTarget,
        Ulid,
    };
    use cortex_learn::{Asserted, Assertion, Commands, HOLD_MS, assert_fact};
    use cortex_store::{Pacer, WriterOptions};

    use super::*;
    use crate::meter::Meter;

    /// The signature changes the hosts of these tests keep: none of the tests reads them.
    static SIGNATURES: Signatures = Signatures::new();

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
            is_probe: false,
        })
    }

    /// Nothing pending, with a meter whose home folder holds no transcript: the events here name
    /// none.
    fn pending() -> Pending {
        Pending::new(Meter::new(Path::new("no-home"), &|_| None))
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
        let snapshot = Snapshot::default();
        let parts = (
            writer(root.path())?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (&inbox, &snapshot, &SIGNATURES),
        );
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        let report = host.index(&[])?;
        assert_eq!(report.files, 600);
        assert_eq!(inbox.counts().events_waiting, 0);
        assert_eq!(
            stored_kinds(root.path())?,
            [EventKind::SessionStart, EventKind::Prompt]
        );
        Ok(())
    }

    /// A batch the index's pause cannot record, its second prompt repeating the first prompt's
    /// event, is counted and left, and the index goes on: all 600 files are written and nothing of
    /// the batch is stored. Why is kept as a recent error: SQLite's refusal of the second task, and
    /// nothing of the prompt. The host's own recorder forgot what it took as stored, so the next
    /// event registers its session again and is recorded.
    #[test]
    fn a_batch_the_pause_cannot_record_is_counted() -> TestResult {
        let root = tempfile::tempdir()?;
        write_modules(root.path())?;
        let config = Config::default();
        let grammars = Arc::new(Grammars::load()?);
        let inbox = Inbox::default();
        let snapshot = Snapshot::default();
        let parts = (
            writer(root.path())?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (&inbox, &snapshot, &SIGNATURES),
        );
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        let prompt = Event {
            prompt_text: Some("a private prompt".to_owned()),
            ..event(HookKind::Prompt, 10, b"same")?
        };
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(prompt.clone());
        inbox.push_event(prompt);
        let before_ms = now_ms();
        let report = host.index(&[])?;
        let after_ms = now_ms();
        assert_eq!(report.files, 600);
        let counts = inbox.counts();
        assert_eq!((counts.events_waiting, counts.failed_batches), (0, 1));
        assert_eq!(host.writer.queries().events_after(0, 10)?, []);
        let errors = inbox.recent_errors();
        let messages: Vec<&str> = errors.iter().map(|error| error.message.as_str()).collect();
        assert!(
            matches!(messages.as_slice(), [message]
                if message.starts_with("recording hook events failed: ")
                    && message.contains("UNIQUE constraint failed")
                    && !message.contains("private")),
            "{messages:?}"
        );
        let times: Vec<i64> = errors.iter().map(|error| error.at_ms).collect();
        assert!(
            times
                .iter()
                .all(|at_ms| (before_ms..=after_ms).contains(at_ms)),
            "{times:?} not within {before_ms}..={after_ms}"
        );
        host.record(&[event(HookKind::Prompt, 20, b"next")?])?;
        let session = Identifier::new("s1")?;
        assert!(host.writer.queries().session(&session)?.is_some());
        assert_eq!(stored_kinds(root.path())?, [EventKind::Prompt]);
        Ok(())
    }

    /// A batch whose second prompt repeats the first prompt's event (so its task id is taken)
    /// cannot be recorded: none of it is, not even the session it started. The recorder then forgets
    /// what it took as stored, so the next batch registers the session again and is recorded whole.
    #[test]
    fn a_batch_that_fails_leaves_nothing_behind() -> TestResult {
        let root = tempfile::tempdir()?;
        let config = Config::default();
        let grammars = Arc::new(Grammars::load()?);
        let inbox = Inbox::default();
        let snapshot = Snapshot::default();
        let parts = (
            writer(root.path())?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (&inbox, &snapshot, &SIGNATURES),
        );
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
        assert_eq!(stored_kinds(root.path())?, [EventKind::Prompt]);
        Ok(())
    }

    /// A host over a repository at `root` holding `a.py` and `b.py`, indexed, both files then
    /// changed on disk.
    fn changed_repository<'a>(
        root: &'a Path,
        (config, inbox, snapshot): (&'a Config, &'a Inbox, &'a Snapshot),
    ) -> Result<Host<'a>, Box<dyn std::error::Error>> {
        std::fs::write(root.join("a.py"), "a = 1\n")?;
        std::fs::write(root.join("b.py"), "b = 1\n")?;
        let parts = (
            writer(root)?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (inbox, snapshot, &SIGNATURES),
        );
        let grammars = Arc::new(Grammars::load()?);
        let mut host = Host::new(root, grammars, (config, discover(config)), parts);
        host.index(&[])?;
        std::fs::write(root.join("a.py"), "a = 2\n")?;
        std::fs::write(root.join("b.py"), "b = 2\n")?;
        Ok(host)
    }

    /// The watcher's edits the store holds under `root`: each file with when it was seen.
    fn watched(root: &Path) -> Result<Vec<(String, i64)>, Box<dyn std::error::Error>> {
        let reader = cortex_store::Reader::open(&root.join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_after(0, 100)?
            .into_iter()
            .filter(|row| row.record.meta.source == Some(EventSource::Watcher))
            .map(|row| {
                let path = row.record.path.map(|path| path.to_string());
                (path.unwrap_or_default(), row.record.ts_ms)
            })
            .collect())
    }

    /// Each edit keeps the time a change first named its own file: `a.py` seen at 1,000 ms and
    /// `b.py` at 9,000, updated together, are recorded at those times, `b.py`'s not taken back to
    /// 1,000.
    #[test]
    fn each_edit_keeps_the_time_its_file_was_seen() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = changed_repository(root.path(), (&config, &inbox, &snapshot))?;
        host.update(&[
            (RepoPath::new("a.py")?, 1_000),
            (RepoPath::new("b.py")?, 9_000),
        ])?;
        record_due_edits(&mut host.writer, &mut host.recorder, &inbox, i64::MAX);
        assert_eq!(
            watched(root.path())?,
            [("a.py".to_owned(), 1_000), ("b.py".to_owned(), 9_000)]
        );
        Ok(())
    }

    /// The kinds of the events stored under `root`, in order.
    fn stored_kinds(root: &Path) -> Result<Vec<EventKind>, Box<dyn std::error::Error>> {
        let reader = cortex_store::Reader::open(&root.join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_after(0, 10)?
            .into_iter()
            .map(|row| row.record.kind)
            .collect())
    }

    /// A host on a repository of one file, indexed, with `config`, `inbox` and `snapshot`.
    fn indexed_host<'a>(
        root: &'a Path,
        (config, inbox, snapshot): (&'a Config, &'a Inbox, &'a Snapshot),
    ) -> Result<Host<'a>, Box<dyn std::error::Error>> {
        std::fs::write(root.join("a.py"), "a = 1\n")?;
        let parts = (
            writer(root)?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (inbox, snapshot, &SIGNATURES),
        );
        let grammars = Arc::new(Grammars::load()?);
        let mut host = Host::new(root, grammars, (config, discover(config)), parts);
        host.index(&[])?;
        Ok(host)
    }

    /// A fact on `a.py` stays active through an index that finds `a.py` as it was, and turns
    /// suspect at the index that finds it changed.
    #[test]
    fn an_index_that_changes_a_fact_s_file_makes_it_suspect() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        let path = RepoPath::new("a.py")?;
        let mut transaction = host.writer.transaction()?;
        let node = transaction
            .queries()
            .file_node_at(&path)?
            .ok_or("a.py has no node")?;
        let assertion = Assertion {
            text: "a is one.",
            kind: FactKind::Convention,
            source: FactSource::User,
            task: None,
            nodes: &[node],
        };
        let Asserted::Added(fact) = assert_fact(&mut transaction, &assertion, 1)? else {
            return Err("the fact was not added".into());
        };
        transaction.commit()?;
        let status = |host: &Host<'_>| -> Result<FactStatus, Box<dyn std::error::Error>> {
            Ok(host
                .writer
                .queries()
                .fact(fact)?
                .ok_or("no fact")?
                .record
                .status)
        };
        let changed = [path];
        let index_a = |host: &mut Host<'_>| {
            host.indexing(NonZeroUsize::MIN, |root, grammars, writer, options| {
                index_paths(root, grammars, writer, options, &changed)
            })
        };
        index_a(&mut host)?;
        assert_eq!(status(&host)?, FactStatus::Active);
        std::fs::write(root.path().join("a.py"), "a = 2\n")?;
        index_a(&mut host)?;
        assert_eq!(status(&host)?, FactStatus::Suspect);
        Ok(())
    }

    /// The signature changes an index writes are kept before the index ends: an edit's warnings
    /// that read a file's new rows while the index still runs know that `f` changed. `g` changed
    /// its body only, which is no signature change.
    #[test]
    fn an_index_s_signature_changes_are_kept_as_they_are_written() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let (snapshot, signatures) = (Snapshot::default(), Signatures::new());
        std::fs::write(
            root.path().join("a.py"),
            "def f(a):
    return a


def g():
    return 1
",
        )?;
        let parts = (
            writer(root.path())?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (&inbox, &snapshot, &signatures),
        );
        let grammars = Arc::new(Grammars::load()?);
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        host.index(&[])?;
        std::fs::write(
            root.path().join("a.py"),
            "def f(a, b):
    return a


def g():
    return 2
",
        )?;
        let changed = [RepoPath::new("a.py")?];
        let mut kept = Vec::new();
        host.indexing(NonZeroUsize::MIN, |root, grammars, writer, options| {
            let indexed = index_paths(root, grammars, writer, options, &changed);
            let queries = writer.queries();
            let symbols = queries
                .file(&changed[0])
                .ok()
                .flatten()
                .and_then(|row| queries.symbols_in_file(row.id).ok())
                .unwrap_or_default();
            let changed_among = signatures.changed_among(
                &symbols.iter().map(|symbol| symbol.id).collect::<Vec<_>>(),
                (0, now_ms()),
            );
            kept = symbols
                .into_iter()
                .filter(|symbol| changed_among.contains(&symbol.id))
                .map(|symbol| symbol.qual_name.as_str().to_owned())
                .collect();
            indexed
        })?;
        assert_eq!(kept, ["f"]);
        Ok(())
    }

    /// Hook events waiting when the daemon is asked to stop are recorded before it stops, though
    /// the stop came first, and so is one that arrives after the stop was asked for but before the
    /// writer saw it; one that arrives once the writer has stopped is refused, not taken and
    /// lost.
    #[test]
    fn waiting_events_are_recorded_at_a_stop() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        assert!(inbox.push_event(event(HookKind::SessionStart, 5, b"start")?));
        inbox.stop();
        assert!(inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?));
        host.serve()?;
        assert!(!inbox.push_event(event(HookKind::Stop, 15, b"stop")?));
        assert_eq!(
            stored_kinds(root.path())?,
            [EventKind::SessionStart, EventKind::Prompt]
        );
        Ok(())
    }

    /// A daemon that stops because it was idle takes no event after: one that arrived as it
    /// stopped would be answered and never recorded.
    #[test]
    fn an_idle_stop_takes_no_more_events() -> TestResult {
        let root = tempfile::tempdir()?;
        let config = Config {
            idle_shutdown_minutes: 0,
            ..Config::default()
        };
        let inbox = Inbox::default();
        let snapshot = Snapshot::default();
        let host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.serve()?;
        assert!(!inbox.push_event(event(HookKind::Stop, 15, b"stop")?));
        assert_eq!(stored_kinds(root.path())?, []);
        Ok(())
    }

    /// An edit still held when the daemon is asked to stop is recorded before it stops, though its
    /// 2 s are not over.
    #[test]
    fn held_edits_are_recorded_at_a_stop() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = changed_repository(root.path(), (&config, &inbox, &snapshot))?;
        let seen_ms = now_ms();
        host.update(&[(RepoPath::new("a.py")?, seen_ms)])?;
        inbox.stop();
        host.serve()?;
        assert_eq!(watched(root.path())?, [("a.py".to_owned(), seen_ms)]);
        Ok(())
    }

    /// An edit of line 1 of `a.py` by session `s1` at `ts_ms`, in the repository at `root`, as
    /// Claude Code reports one that applied.
    fn edit_of_a(root: &Path, ts_ms: i64) -> Result<Event, Box<dyn std::error::Error>> {
        Ok(Event {
            cwd: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            tool: Some(ToolKind::Edit),
            tool_target: Some(ToolTarget {
                path: Some(RepoPath::new("a.py")?),
                ranges: vec![LineRange::new(1, 1)?],
                cut_after: None,
                pattern: None,
                command: None,
                hits: Vec::new(),
                more_paths: Vec::new(),
                failed_output: None,
            }),
            tool_result: Some(ToolResult {
                exit_code: None,
                is_success: true,
                is_interrupted: false,
                output_bytes: None,
            }),
            prompt_text: None,
            ..event(HookKind::PostTool, ts_ms, b"edit")?
        })
    }

    /// What the store says of a task: its outcome, the words that say why, and whether its record
    /// is written.
    type Judged = (Option<TaskOutcome>, Option<String>, bool);

    /// The one task of session `s1` the host's store holds, as [`Judged`] says it.
    fn the_task(host: &Host<'_>) -> Result<Judged, Box<dyn std::error::Error>> {
        let session = Identifier::new("s1")?;
        let queries = host.writer.queries();
        let task_id = queries
            .events_of_session(&session)?
            .into_iter()
            .find_map(|row| row.record.task_id)
            .ok_or("no task")?;
        let row = queries.task(task_id)?.ok_or("no task row")?;
        let has_record = queries.episode_of_task(task_id)?.is_some();
        Ok((row.outcome, row.outcome_evidence, has_record))
    }

    /// The graph is told of each write that changes it: the first index, and a task learned from
    /// when the host closes; a host that closes with no task to learn from tells it nothing.
    #[test]
    fn the_graph_is_told_of_the_writes_that_change_it() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        assert!(snapshot.take_change());
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        inbox.push_event(event(HookKind::Stop, 30, b"stop")?);
        host.close();
        assert!(snapshot.take_change());

        let other = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(other.path(), (&config, &inbox, &snapshot))?;
        assert!(snapshot.take_change());
        host.close();
        assert!(!snapshot.take_change());
        Ok(())
    }

    /// A task that ends is learned from before the daemon stops: a prompt and the end of its
    /// answer wait in the inbox, and once the host closes, the task's record is written, which
    /// only learning writes. It changed nothing and ran nothing, so nothing is known of it.
    #[test]
    fn a_task_that_ends_is_learned_from_before_a_stop() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        inbox.push_event(event(HookKind::Stop, 30, b"stop")?);
        host.close();
        assert_eq!(the_task(&host)?, (Some(TaskOutcome::Unknown), None, true));
        Ok(())
    }

    /// The transcript a stop names is read before the daemon stops: the end of an answer naming
    /// the Grep session's transcript waits in the inbox, and once the host closes, the session
    /// has a usage row for each of the transcript's four responses, whose outputs were 100, 95,
    /// 360 and 3 tokens.
    #[test]
    fn a_stop_s_transcript_is_read_before_a_stop() -> TestResult {
        let root = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        let project = home.path().join(".claude").join("projects").join("C--repo");
        std::fs::create_dir_all(&project)?;
        let transcript = project.join("s1.jsonl");
        std::fs::write(
            &transcript,
            include_str!("../../../tests/fixtures/claude/2.1.316/transcripts/grep-session.jsonl"),
        )?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.pending = Pending::new(Meter::new(home.path(), &|_| None));
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(Event {
            transcript_path: Some(transcript),
            ..event(HookKind::Stop, 30, b"stop")?
        });
        host.close();
        let outputs: Vec<u64> = host
            .writer
            .queries()
            .usage_of_session(&Identifier::new("s1")?)?
            .iter()
            .map(|row| row.output)
            .collect();
        assert_eq!(outputs, [100, 95, 360, 3]);
        Ok(())
    }

    /// A hook's edit waiting in the inbox while the watcher's report of the same change falls due
    /// is recorded first, so it claims the report and the watcher's edit is not recorded again:
    /// `a.py` held `a = 1`, was changed to `a = 2` and indexed, and the hook's edit of it waits
    /// while the report's hold runs out. The task keeps what the report says the file held before.
    /// The file is then put back as it was and indexed again, and the task ends: it is reverted,
    /// every file back as it was before it.
    #[test]
    fn a_waiting_hook_edit_claims_what_the_watcher_saw() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.record(&[
            event(HookKind::SessionStart, 1_000, b"start")?,
            event(HookKind::Prompt, 1_001, b"prompt")?,
        ])?;
        std::fs::write(root.path().join("a.py"), "a = 2\n")?;
        host.update(&[(RepoPath::new("a.py")?, 1_100)])?;
        inbox.push_event(edit_of_a(root.path(), 1_200)?);
        let recorded = record_due_edits(
            &mut host.writer,
            &mut host.recorder,
            &inbox,
            1_100 + HOLD_MS,
        );
        assert!(recorded.ended.is_empty());
        assert_eq!(
            watched(root.path())?,
            [],
            "the hook's edit claimed the watcher's"
        );
        std::fs::write(root.path().join("a.py"), "a = 1\n")?;
        host.update(&[(RepoPath::new("a.py")?, now_ms())])?;
        host.record(&[event(HookKind::Stop, now_ms(), b"stop")?])?;
        host.settle();
        assert_eq!(
            the_task(&host)?,
            (
                Some(TaskOutcome::Reverted),
                Some("every edited file is back as it was before the task".to_owned()),
                true
            )
        );
        Ok(())
    }

    /// The tasks of session `s1` the store in `root` holds, and how many of them have their
    /// record, which only learning writes.
    fn tasks_learned(root: &Path) -> Result<(usize, usize), Box<dyn std::error::Error>> {
        let reader = cortex_store::Reader::open(&root.join(".cortex"), 0)?;
        let queries = reader.queries();
        let tasks: BTreeSet<Ulid> = queries
            .events_of_session(&Identifier::new("s1")?)?
            .into_iter()
            .filter_map(|row| row.record.task_id)
            .collect();
        let mut learned = 0;
        for task in &tasks {
            if queries.episode_of_task(*task)?.is_some() {
                learned += 1;
            }
        }
        Ok((tasks.len(), learned))
    }

    /// The writer records the hook events that wait while it learns from the tasks that ended:
    /// two tasks end and wait, the end of the session arrives meanwhile, and once the tasks are
    /// learned from it is stored too, nothing left waiting.
    #[test]
    fn events_waiting_are_recorded_while_tasks_are_learned_from() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.record(&[
            event(HookKind::SessionStart, 5, b"start")?,
            event(HookKind::Prompt, 10, b"prompt")?,
            event(HookKind::Stop, 20, b"stop")?,
            event(HookKind::Prompt, 30, b"prompt again")?,
            event(HookKind::Stop, 40, b"stop again")?,
        ])?;
        assert_eq!(tasks_learned(root.path())?, (2, 0));
        assert!(inbox.push_event(event(HookKind::SessionEnd, 50, b"end")?));
        host.settle();
        assert_eq!(tasks_learned(root.path())?, (2, 2));
        assert_eq!(
            stored_kinds(root.path())?,
            [
                EventKind::SessionStart,
                EventKind::Prompt,
                EventKind::Stop,
                EventKind::Prompt,
                EventKind::Stop,
                EventKind::SessionEnd,
            ]
        );
        Ok(())
    }

    /// A task that ended and waits to be learned from has the work loop look again at once, instead
    /// of when the next crawl or the idle stop is due; with none waiting it waits for those again.
    /// Upkeep due ten minutes into a quiet spell, before the idle stop, is when it looks again too.
    #[test]
    fn a_task_waiting_to_be_learned_from_wakes_the_work_loop() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        // Upkeep has run in this quiet spell: it is not due until the next event.
        host.upkeep_clock.ran(Ended::Done, Instant::now());
        let idle_at = Instant::now() + Duration::from_secs(600);
        let sweep_at = idle_at + Duration::from_secs(60);
        assert_eq!(
            host.deadline(idle_at, Some(idle_at), sweep_at),
            Some(idle_at)
        );
        host.record(&[
            event(HookKind::SessionStart, 5, b"start")?,
            event(HookKind::Prompt, 10, b"prompt")?,
            event(HookKind::Stop, 20, b"stop")?,
        ])?;
        let now = Instant::now();
        assert!(
            host.deadline(idle_at, Some(idle_at), sweep_at)
                .is_some_and(|at| at <= Instant::now() && at >= now)
        );
        host.settle();
        assert_eq!(
            host.deadline(idle_at, Some(idle_at), sweep_at),
            Some(idle_at)
        );
        // A sweep for quiet sessions due before the idle stop is when the loop looks again.
        let sooner = idle_at - Duration::from_secs(300);
        assert_eq!(host.deadline(idle_at, Some(idle_at), sooner), Some(sooner));
        let quiet = Instant::now();
        host.upkeep_clock = UpkeepClock::new(quiet);
        let later = quiet + Duration::from_secs(3_600);
        assert_eq!(
            host.deadline(later, Some(later), later),
            Some(quiet + crate::upkeep::QUIET_BEFORE_UPKEEP)
        );
        Ok(())
    }

    /// A task whose session went quiet without a stop (last heard from at 10 ms, far longer ago
    /// than the 30 quiet minutes) is ended as left at its last sign of work, and waits to be
    /// learned from; a second sweep finds nothing left to end.
    #[test]
    fn a_task_whose_session_went_quiet_is_ended() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.record(&[
            event(HookKind::SessionStart, 5, b"start")?,
            event(HookKind::Prompt, 10, b"prompt")?,
        ])?;
        host.settle();
        assert!(host.pending.is_idle());
        host.end_quiet_tasks();
        assert!(!host.pending.is_idle());
        host.settle();
        let reader = cortex_store::Reader::open(&root.path().join(".cortex"), 0)?;
        let tasks = reader.queries().tasks_of_session(&Identifier::new("s1")?)?;
        let ended: Vec<Option<i64>> = tasks.iter().map(|task| task.ended_at_ms).collect();
        assert_eq!(ended, [Some(10)]);
        host.end_quiet_tasks();
        assert!(host.pending.is_idle());
        Ok(())
    }

    /// Only so many ended tasks wait to be learned from: of 257 tasks that end in one batch, the
    /// first is let go and the 256 after it are learned from.
    #[test]
    fn only_so_many_ended_tasks_wait() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        let mut events = vec![event(HookKind::SessionStart, 5, b"start")?];
        for task in 0..257_i64 {
            let at_ms = 100 + task * 10;
            events.push(event(HookKind::Prompt, at_ms, b"prompt")?);
            events.push(event(HookKind::Stop, at_ms + 5, b"stop")?);
        }
        host.record(&events)?;
        host.settle();
        assert_eq!(tasks_learned(root.path())?, (257, 256));
        let queries = host.writer.queries();
        let first = queries
            .events_of_session(&Identifier::new("s1")?)?
            .into_iter()
            .find_map(|row| row.record.task_id)
            .ok_or("no task")?;
        assert_eq!(queries.episode_of_task(first)?, None);
        Ok(())
    }

    /// A task that ends in a batch an index's pause records waits until the index is done, then
    /// is learned from: the session's start, a prompt and the end of its answer wait while 600
    /// files are indexed, the pause records them, and the task is learned from once the host
    /// learns, not before.
    #[test]
    fn a_task_a_pause_ended_waits_to_be_learned_from() -> TestResult {
        let root = tempfile::tempdir()?;
        write_modules(root.path())?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let parts = (
            writer(root.path())?,
            Recorder::new(false, 0.0, Commands::default()),
            pending(),
            (&inbox, &snapshot, &SIGNATURES),
        );
        let grammars = Arc::new(Grammars::load()?);
        let mut host = Host::new(root.path(), grammars, (&config, discover(&config)), parts);
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        inbox.push_event(event(HookKind::Stop, 20, b"stop")?);
        host.index(&[])?;
        assert_eq!(inbox.counts().events_waiting, 0);
        assert_eq!(tasks_learned(root.path())?, (1, 0));
        host.settle();
        assert_eq!(tasks_learned(root.path())?, (1, 1));
        Ok(())
    }

    /// A task that ends in the events recorded before the edits the watcher saw fall due is learned
    /// from by the work loop: the session's start, a prompt and the end of its answer wait while
    /// an edit is held past its time, and the daemon serves until it is asked to stop.
    #[test]
    fn a_task_ended_before_due_edits_is_learned_from() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        let before = FileContent::Hashed(ContentHash::of(b"a = 1\n"));
        host.recorder
            .hold_watcher_edits([(RepoPath::new("a.py")?, before)], 1_000);
        inbox.push_event(event(HookKind::SessionStart, 5, b"start")?);
        inbox.push_event(event(HookKind::Prompt, 10, b"prompt")?);
        inbox.push_event(event(HookKind::Stop, 20, b"stop")?);
        inbox.stop();
        host.serve()?;
        assert_eq!(tasks_learned(root.path())?, (1, 1));
        Ok(())
    }

    /// A hook's edit recorded in an index's pause, after that index wrote the file's new row, takes
    /// nothing from the row as what the file held before, the row holding the edit's change
    /// already: `a.py` goes from `a = 1` to `a = 2`, the index writes its row and then lets the
    /// writer in while the hook's edit of it waits. The task ends with the file still `a = 2`, so
    /// taking the row would judge it put back; knowing nothing of it, nothing is known of the task.
    #[test]
    fn an_edit_recorded_in_an_index_s_pause_is_not_taken_as_put_back() -> TestResult {
        let root = tempfile::tempdir()?;
        let (config, inbox) = (Config::default(), Inbox::default());
        let snapshot = Snapshot::default();
        let mut host = indexed_host(root.path(), (&config, &inbox, &snapshot))?;
        host.record(&[
            event(HookKind::SessionStart, 1_000, b"start")?,
            event(HookKind::Prompt, 1_001, b"prompt")?,
        ])?;
        std::fs::write(root.path().join("a.py"), "a = 2\n")?;
        let changed = [RepoPath::new("a.py")?];
        let edit = edit_of_a(root.path(), 1_200)?;
        host.indexing(NonZeroUsize::MIN, |root, grammars, writer, options| {
            let quiet = IndexOptions {
                pause: None,
                ..*options
            };
            let indexed = index_paths(root, grammars, writer, &quiet, &changed);
            inbox.push_event(edit);
            Pacer::new(options.pause).let_in(writer);
            indexed
        })?;
        host.record(&[event(HookKind::Stop, now_ms(), b"stop")?])?;
        host.settle();
        assert_eq!(the_task(&host)?, (Some(TaskOutcome::Unknown), None, true));
        Ok(())
    }
}
