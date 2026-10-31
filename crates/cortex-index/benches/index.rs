//! Indexing benchmarks over the synthetic 10,000-file repository: the first index, and the index
//! of one changed file's path, as the daemon runs it on every change, for four kinds of edit in
//! turn. `cargo xtask verify --full` reads their samples and checks them against the budgets.

mod synthetic;

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cortex_core::RepoPath;
use cortex_core::{ModeSetting, RepoId};
use cortex_index::{
    DiscoverOptions, Grammars, IndexOptions, IndexReport, index_paths, index_repository,
};
use cortex_store::{Writer, WriterOptions};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};

/// The default per-file time limit, `index.parse_timeout_ms`.
const PARSE_TIMEOUT: Duration = Duration::from_secs(1);

/// How long the watcher waits for a path to be quiet before it reports the change.
const WATCHER_QUIET: Duration = Duration::from_millis(200);

/// The default size limit, `index.max_file_bytes`.
const MAX_FILE_BYTES: u64 = 1_048_576;

/// The options of an index on every core, as the daemon runs it.
fn options() -> IndexOptions<'static> {
    IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: MAX_FILE_BYTES,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: PARSE_TIMEOUT,
        threads: std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN),
        now_ms: now_ms(),
        mode: ModeSetting::Auto,
    }
}

/// Indexes the repository at `root` with `writer`, as the daemon does first.
fn index(root: &Path, grammars: &Arc<Grammars>, writer: &mut Writer) -> IndexReport {
    // A benchmark that cannot index has nothing to measure.
    #[allow(clippy::expect_used)]
    index_repository(root, grammars, writer, &options()).expect("indexing the synthetic repository")
}

/// Indexes the files at `paths` of the repository at `root` with `writer`, as the daemon does
/// when they change.
fn update(root: &Path, grammars: &Arc<Grammars>, writer: &mut Writer, paths: &[RepoPath]) {
    // A benchmark that cannot index has nothing to measure.
    #[allow(clippy::expect_used)]
    index_paths(root, grammars, writer, &options(), paths).expect("indexing a changed file");
}

/// Opens the writer of the repository at `root`.
fn writer(root: &Path) -> Writer {
    // A benchmark that cannot open its store has nothing to measure.
    #[allow(clippy::expect_used)]
    Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(root),
        mmap_bytes: 268_435_456,
        now_ms: now_ms(),
    })
    .expect("opening the benchmark's store")
}

/// The time now in UTC milliseconds.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The first index of the repository, from no store at all; and an index after one file changed.
fn indexing(criterion: &mut Criterion) {
    // A benchmark without its repository or grammars has nothing to measure.
    #[allow(clippy::expect_used)]
    let root = synthetic::repository().expect("writing the synthetic repository");
    #[allow(clippy::expect_used)]
    let grammars = Arc::new(Grammars::load().expect("loading the grammars"));
    let store = root.join(".cortex");
    let mut group = criterion.benchmark_group("index");
    group.sampling_mode(SamplingMode::Flat).sample_size(10);
    group.bench_function("initial_10k", |bencher| {
        bencher.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                // The store is the benchmark's own, made by the previous iteration.
                #[allow(clippy::expect_used)]
                if store.exists() {
                    std::fs::remove_dir_all(&store).expect("removing the benchmark's store");
                }
                let mut writer = writer(&root);
                let start = Instant::now();
                index(&root, &grammars, &mut writer);
                total += start.elapsed();
            }
            total
        });
    });
    // Ten samples of each kind of edit.
    group.sample_size(40);
    let mut changes = synthetic::Changes::default();
    #[allow(clippy::expect_used)]
    changes.write(&root).expect("changing a file");
    let mut writer = writer(&root);
    index(&root, &grammars, &mut writer);
    #[allow(clippy::expect_used)]
    let changed = [RepoPath::new(synthetic::CHANGED_FILE).expect("a valid path")];
    let mut sample = 0;
    group.bench_function("one_file_changed_10k", |bencher| {
        bencher.iter_custom(|iterations| {
            // One kind of edit per sample, the next kind in the next sample: a slow kind makes
            // whole samples slow instead of averaging into the others'.
            let edit = synthetic::Edit::of_sample(sample);
            sample += 1;
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                #[allow(clippy::expect_used)]
                changes.make(&root, edit).expect("changing a file");
                // The daemon sees a change once the path has been quiet for the watcher's 200 ms.
                // Read at once instead, a file just written is often scanned by the system's
                // malware protection first, which no index of a watched change waits for.
                std::thread::sleep(WATCHER_QUIET);
                let start = Instant::now();
                update(&root, &grammars, &mut writer, &changed);
                total += start.elapsed();
            }
            total
        });
    });
    group.finish();
}

criterion_group!(benches, indexing);
criterion_main!(benches);
