//! Upkeep over a large store, run as the daemon runs it on its writer, again after each wait when
//! the write-ahead log holds too much, until it is done: the time of those runs, the waits left
//! out, and their longest batch, which is the longest a hook event waiting behind upkeep waits for
//! it. `cargo xtask verify --full` reads their samples and checks them against the budgets.
//!
//! The store is a busy repository's after 100 days of use, past every limit upkeep keeps: 20,000
//! files; 200,000 links of use, half faded out; interest on every file, half faded out; 1,000,000
//! events spread over the 100 days, a tenth older than 90 days; 60,000 tasks with their episodes,
//! a third of unknown outcome, so 10,000 past the cap; 10,000 positions, half that can no longer
//! hold; 1,000 facts, half retired 99 days ago. It is built once; each run works on a copy.

use std::cell::Cell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cortex_coord::Timing;
use cortex_core::{
    Agent, Config, ContentHash, CoordinationConfig, EdgeType, EventKind, EventMeta, FactKind,
    FactSource, FactStatus, FileFlags, Identifier, InterestTier, PositionMode, RepoId, RepoPath,
    TaskOutcome, Ulid,
};
use cortex_daemon::{Ended, LOG_LIMIT_BYTES, PACE_WAIT, Swept, Upkeep, UpkeepSettings};
use cortex_store::{
    EdgeRecord, EpisodeRecord, EventRecord, FactRecord, FileIndex, FileRecord, InterestRow, NodeId,
    Pitfalls, PositionRecord, SessionRecord, TaskRecord, TransitionRow, WriteTx, Writer,
    WriterOptions,
};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};

/// Any failure ends the benchmark: it has nothing to measure.
type BenchResult<T = ()> = Result<T, Box<dyn Error>>;

/// A day in milliseconds.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// When upkeep runs: day 100 of the store's use.
const NOW_MS: i64 = 100 * DAY_MS;

/// Files, each a node.
const FILES: usize = 20_000;

/// Links of use from each file, to the files after it: the even ones strong, the odd ones faded.
const LINKS_PER_FILE: usize = 10;

/// Events, one every 8.64 seconds over the 100 days.
const EVENTS: i64 = 1_000_000;

/// Events written in one transaction while the store is built.
const EVENTS_PER_TRANSACTION: i64 = 100_000;

/// Tasks, one every 144 seconds over the 100 days, each with its episode.
const TASKS: i64 = 60_000;

/// Sessions the tasks, events and positions belong to.
const SESSIONS: usize = 100;

/// Positions each session holds, on as many files.
const POSITIONS_PER_SESSION: usize = 100;

/// Facts, one on each of the first files.
const FACTS: usize = 1_000;

/// Runs measured per benchmark.
const SAMPLES: usize = 10;

/// How long a copy of the store is left alone before a run on it is timed.
const SETTLE: Duration = Duration::from_secs(5);

/// What a pass of upkeep removes, counted from the store's shape: every odd link of the 10 from each file
/// (100,000); every odd way of the 4 from each file to the files read after it (40,000); the interest of every even file (10,000); every even fact, retired on day 1, more
/// than 90 days before day 100 (500); the events of the first 10 days, one every 8.64 s (100,000);
/// no injection (the store holds none);
/// the episodes past 50,000 of the 60,000, all of unknown outcome (10,000); the positions of
/// day 1, every even one of each session's 100 (5,000); no handoff's failure and no handoff (the
/// store holds none); no blame (the store
/// holds none) and no file warmed (no history worker runs); and 2,000 free pages, the most upkeep
/// gives back each time through its jobs, of the more than 2,400 the events alone leave (100,000
/// rows of over 100 bytes each, in 4 KiB pages).
const SWEPT: [u64; 13] = [
    100_000, 40_000, 10_000, 500, 100_000, 0, 10_000, 5_000, 0, 0, 0, 0, 2_000,
];

/// How many ways each file has to the files tasks read after it.
const TRANSITIONS_PER_FILE: usize = 4;

/// Upkeep's runs on a copy of the store, until it is done: how long they took, and their longest
/// batch.
struct Run {
    /// From the first batch to the end of the last of each run, the waits between runs left out.
    took: Duration,
    /// The longest time between two of the runner's checks for waiting work, the last batch's
    /// end included.
    longest_batch: Duration,
}

/// The session numbered `at`.
fn session(at: usize) -> BenchResult<Identifier> {
    Ok(Identifier::new(&format!("session-{at}"))?)
}

/// The item of `items` at `at`, counting round from the first again past the last.
fn wrapped<T: Copy>(items: &[T], at: usize) -> BenchResult<T> {
    items
        .get(at % items.len().max(1))
        .copied()
        .ok_or_else(|| "nothing to pick from".into())
}

/// Stores the files and returns their nodes.
fn add_files(transaction: &mut WriteTx<'_>) -> BenchResult<Vec<NodeId>> {
    let mut nodes = Vec::with_capacity(FILES);
    for at in 0..FILES {
        let path = format!("src/module_{at}.rs");
        let change = transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new(&path)?,
                language: None,
                size_bytes: 100,
                line_count: Some(10),
                content_hash: Some(ContentHash::of(path.as_bytes())),
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        nodes.push(change.file_node);
    }
    Ok(nodes)
}

/// Stores the sessions, then the tasks with their episodes, every third of unknown outcome:
/// the tasks, and the last task's number.
fn add_tasks(transaction: &mut WriteTx<'_>) -> BenchResult<(Vec<Ulid>, u64)> {
    for at in 0..SESSIONS {
        transaction.record_session(&SessionRecord {
            session_id: session(at)?,
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 0,
        })?;
    }
    let mut tasks = Vec::new();
    let mut latest = 0;
    for at in 0..TASKS {
        let started_at_ms = at * (100 * DAY_MS / TASKS);
        let task_id = Ulid::from_seed(u64::try_from(started_at_ms)?, b"task");
        latest = transaction.start_task(&TaskRecord {
            task_id,
            session_id: session(usize::try_from(at)? % SESSIONS)?,
            started_at_ms,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: None,
        })?;
        transaction.replace_episode(&EpisodeRecord {
            task_id,
            localize: Vec::new(),
            change: Vec::new(),
            verify: Vec::new(),
            pitfalls: Pitfalls::default(),
            outcome: if at % 3 == 0 {
                TaskOutcome::Unknown
            } else {
                TaskOutcome::Success
            },
            created_at_ms: started_at_ms,
        })?;
        tasks.push(task_id);
    }
    Ok((tasks, latest))
}

/// Stores the links of use, the ways from file to file and the interest, as of task `latest`, so
/// that none fades further.
fn add_links(transaction: &mut WriteTx<'_>, nodes: &[NodeId], latest: u64) -> BenchResult {
    let link_seq = i64::try_from(latest)?;
    let mut links = Vec::with_capacity(FILES * LINKS_PER_FILE);
    let mut interest = Vec::with_capacity(FILES);
    for (at, node) in nodes.iter().enumerate() {
        for step in 1..=LINKS_PER_FILE {
            let weight = if step % 2 == 0 { 0.5 } else { 0.004 };
            links.push((
                *node,
                EdgeRecord {
                    dst: wrapped(nodes, at + step)?,
                    weight,
                    count: 1,
                    last_seq: link_seq,
                    is_directed: false,
                },
            ));
        }
        interest.push(InterestRow {
            node: *node,
            score: if at % 2 == 0 { 0.004 } else { 1.0 },
            last_seq: latest,
            tier: InterestTier::Interesting,
        });
    }
    transaction.set_edges(EdgeType::CoAccess, &links)?;
    transaction.replace_interest(&interest)?;
    let mut ways = Vec::with_capacity(FILES * TRANSITIONS_PER_FILE);
    for (at, node) in nodes.iter().enumerate() {
        for step in 1..=TRANSITIONS_PER_FILE {
            ways.push(TransitionRow {
                src: *node,
                dst: wrapped(nodes, at + step)?,
                weight: if step % 2 == 0 { 2.0 } else { 0.004 },
                count: 1,
                last_seq: latest,
            });
        }
    }
    transaction.set_transitions(&ways)?;
    Ok(())
}

/// Stores the positions and the facts, every even one of each retired on day 1.
fn add_positions_and_facts(transaction: &mut WriteTx<'_>, nodes: &[NodeId]) -> BenchResult {
    for at in 0..SESSIONS {
        for place in 0..POSITIONS_PER_SESSION {
            let last_ms = if place % 2 == 0 { DAY_MS } else { NOW_MS - 1 };
            transaction.keep_position(&PositionRecord {
                session_id: session(at)?,
                node: wrapped(nodes, at * POSITIONS_PER_SESSION + place)?,
                mode: PositionMode::Read,
                since_ms: last_ms,
                last_ms,
                worktree: None,
                branch: None,
            })?;
        }
    }
    for (at, node) in nodes.iter().take(FACTS).enumerate() {
        let id = transaction.add_fact(&FactRecord {
            text: format!("Errors of module {at} are typed, never strings."),
            kind: FactKind::Convention,
            source: FactSource::User,
            status: FactStatus::Active,
            created_at_ms: DAY_MS,
            task: None,
            anchors: transaction.queries().anchor_hashes(&[*node])?,
        })?;
        if at % 2 == 0 {
            transaction.set_fact_status(id, FactStatus::Retired, DAY_MS)?;
        }
    }
    Ok(())
}

/// Stores the events, each of the task running then and on one of the files.
fn add_events(writer: &mut Writer, nodes: &[NodeId], tasks: &[Ulid]) -> BenchResult {
    let spacing_ms = 100 * DAY_MS / EVENTS;
    for first in (0..EVENTS).step_by(usize::try_from(EVENTS_PER_TRANSACTION)?) {
        let mut transaction = writer.transaction()?;
        for at in first..first + EVENTS_PER_TRANSACTION {
            let index = usize::try_from(at)?;
            transaction.append_event(&EventRecord {
                task_id: Some(wrapped(
                    tasks,
                    index * tasks.len() / usize::try_from(EVENTS)?,
                )?),
                session_id: Some(session(index % SESSIONS)?),
                ts_ms: at * spacing_ms,
                kind: EventKind::Read,
                node_id: Some(wrapped(nodes, index)?),
                path: None,
                range: None,
                meta: EventMeta::default(),
            })?;
        }
        transaction.commit()?;
    }
    Ok(())
}

/// Opens the store in `folder`, which belongs to `repo`.
fn open(folder: &Path, repo: &RepoId) -> BenchResult<Writer> {
    Ok(Writer::open(&WriterOptions {
        folder,
        repo_id: repo,
        mmap_bytes: 268_435_456,
        now_ms: NOW_MS,
    })?)
}

/// Builds the large store in `folder`.
fn build(folder: &Path, repo: &RepoId) -> BenchResult {
    let mut writer = open(folder, repo)?;
    let mut transaction = writer.transaction()?;
    let nodes = add_files(&mut transaction)?;
    let (tasks, latest) = add_tasks(&mut transaction)?;
    add_links(&mut transaction, &nodes, latest)?;
    add_positions_and_facts(&mut transaction, &nodes)?;
    transaction.commit()?;
    add_events(&mut writer, &nodes, &tasks)
}

/// Copies the files of the store in `from` into a new folder `to`.
fn copy_store(from: &Path, to: &Path) -> BenchResult {
    if to.exists() {
        std::fs::remove_dir_all(to)?;
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let copied = to.join(entry.file_name());
            std::fs::copy(entry.path(), &copied)?;
            // The copy waits in the system's write cache until it is written out; left there, the
            // run's first sync would write all of it, which a store kept for days never waits for.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&copied)?
                .sync_all()?;
        }
    }
    Ok(())
}

/// What `swept` removed, in the order of [`SWEPT`].
fn counts(swept: Swept) -> [u64; 13] {
    let Swept {
        links,
        transitions,
        interest,
        facts,
        events,
        injections,
        episodes,
        positions,
        handoff_failures,
        handoffs,
        blames,
        warmed,
        pages,
    } = swept;
    [
        links,
        transitions,
        interest,
        facts,
        events,
        injections,
        episodes,
        positions,
        handoff_failures,
        handoffs,
        blames,
        warmed,
        pages,
    ]
}

/// One run of `upkeep` with `writer` and `settings`, timed: what it removed, how it ended, and how
/// long it took with its longest batch.
fn run_timed(
    upkeep: &mut Upkeep,
    writer: &mut Writer,
    settings: &UpkeepSettings,
) -> BenchResult<(Swept, Ended, Run)> {
    let start = Instant::now();
    let last_check = Cell::new(start);
    let longest = Cell::new(Duration::ZERO);
    let check = || {
        let now = Instant::now();
        longest.set(longest.get().max(now - last_check.get()));
        last_check.set(now);
        false
    };
    let far = start + Duration::from_secs(3_600);
    let (swept, ended) = upkeep.run(writer, (NOW_MS, settings), (far, &check))?;
    let end = Instant::now();
    longest.set(longest.get().max(end - last_check.get()));
    let run = Run {
        took: end - start,
        longest_batch: longest.get(),
    };
    Ok((swept, ended, run))
}

/// Runs upkeep on a copy of the store in `template` made at `copy` until it is done, waiting
/// [`PACE_WAIT`] after each run that paced, as the daemon does; checks it removed what the store's
/// shape says it must.
fn run_once(template: &Path, copy: &Path, repo: &RepoId) -> BenchResult<Run> {
    copy_store(template, copy)?;
    let mut writer = open(copy, repo)?;
    // A file of hundreds of megabytes just written is read through by the system's malware
    // protection and indexing, which compete with the run for the disk; a store kept for days
    // is not. Measured on Windows without this pause, a run's longest batch varied from 80 ms to
    // 4.2 s.
    std::thread::sleep(SETTLE);
    let config = Config::default();
    let settings = UpkeepSettings {
        link_decay: config.link_decay,
        interest_decay: config.interest_decay,
        longest_hold_ms: Timing::of(&CoordinationConfig::default()).longest_ms(),
        log_limit_bytes: LOG_LIMIT_BYTES,
    };
    let mut upkeep = Upkeep::default();
    let mut removed = [0; 13];
    let mut runs = Run {
        took: Duration::ZERO,
        longest_batch: Duration::ZERO,
    };
    loop {
        let (swept, ended, run) = run_timed(&mut upkeep, &mut writer, &settings)?;
        for (total, count) in removed.iter_mut().zip(counts(swept)) {
            *total += count;
        }
        runs.took += run.took;
        runs.longest_batch = runs.longest_batch.max(run.longest_batch);
        match ended {
            Ended::Done => break,
            Ended::Paced => std::thread::sleep(PACE_WAIT),
            other => return Err(format!("a run ended {other:?}").into()),
        }
    }
    if removed != SWEPT {
        return Err(format!("upkeep removed {removed:?}").into());
    }
    Ok(runs)
}

/// The large store and where each pass's copy goes, under a temporary folder kept until the
/// benchmark ends.
struct Stores {
    /// Holds the folders.
    _scratch: tempfile::TempDir,
    /// The store as built.
    template: PathBuf,
    /// Each pass's copy.
    copy: PathBuf,
    /// The repository the store belongs to.
    repo: RepoId,
}

impl Stores {
    /// Builds the large store.
    fn new() -> BenchResult<Self> {
        let scratch = tempfile::tempdir()?;
        let repo = RepoId::of(scratch.path());
        let template = scratch.path().join("template");
        build(&template, &repo)?;
        Ok(Self {
            template,
            copy: scratch.path().join("copy"),
            _scratch: scratch,
            repo,
        })
    }

    /// One pass of upkeep on a fresh copy.
    fn run(&self) -> Run {
        // A benchmark whose run fails or removes the wrong rows has nothing to measure.
        #[allow(clippy::expect_used)]
        run_once(&self.template, &self.copy, &self.repo).expect("running upkeep on the copy")
    }
}

/// A whole pass of upkeep on the large store, and the longest batch of a pass, each sample one
/// pass: a tenth of a millisecond of measuring time per sample is less than any batch takes, so
/// criterion runs each sample once (and warns that the samples take longer than the time it was
/// given); a millisecond of warming up is one pass.
fn upkeep(criterion: &mut Criterion) {
    // A benchmark without its store has nothing to measure.
    #[allow(clippy::expect_used)]
    let stores = Stores::new().expect("building the large store");
    let mut group = criterion.benchmark_group("upkeep");
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(SAMPLES)
        .measurement_time(Duration::from_micros(100) * u32::try_from(SAMPLES).unwrap_or(10))
        .warm_up_time(Duration::from_millis(1));
    group.bench_function("run_large", |bencher| {
        bencher.iter_custom(|iterations| (0..iterations).map(|_| stores.run().took).sum());
    });
    group.bench_function("longest_batch_large", |bencher| {
        bencher.iter_custom(|iterations| (0..iterations).map(|_| stores.run().longest_batch).sum());
    });
    group.finish();
}

criterion_group!(benches, upkeep);
criterion_main!(benches);
