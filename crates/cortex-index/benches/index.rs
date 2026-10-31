//! Indexing benchmarks over the synthetic 10,000-file repository: the first index, and an index
//! after one file changed, as the daemon runs it on every change. `cargo xtask verify --full`
//! reads their samples and checks them against the budgets.

mod synthetic;

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cortex_core::{ModeSetting, RepoId};
use cortex_index::{DiscoverOptions, Grammars, IndexOptions, IndexReport, index_repository};
use cortex_store::{Writer, WriterOptions};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};

/// The default per-file time limit, `index.parse_timeout_ms`.
const PARSE_TIMEOUT: Duration = Duration::from_secs(1);

/// The default size limit, `index.max_file_bytes`.
const MAX_FILE_BYTES: u64 = 1_048_576;

/// Indexes the repository at `root` with `writer` on every core, as the daemon does.
fn index(root: &Path, grammars: &Arc<Grammars>, writer: &mut Writer) -> IndexReport {
    let options = IndexOptions {
        discover: DiscoverOptions {
            max_file_bytes: MAX_FILE_BYTES,
            extra_ignores: &[],
            global_gitignore: None,
        },
        parse_timeout: PARSE_TIMEOUT,
        threads: std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN),
        now_ms: now_ms(),
        mode: ModeSetting::Auto,
    };
    // A benchmark that cannot index has nothing to measure.
    #[allow(clippy::expect_used)]
    index_repository(root, grammars, writer, &options).expect("indexing the synthetic repository")
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
    group.sample_size(30);
    let mut writer = writer(&root);
    index(&root, &grammars, &mut writer);
    let mut variant = 0;
    group.bench_function("one_file_changed_10k", |bencher| {
        bencher.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                variant += 1;
                #[allow(clippy::expect_used)]
                synthetic::change_file(&root, variant).expect("changing a file");
                let start = Instant::now();
                index(&root, &grammars, &mut writer);
                total += start.elapsed();
            }
            total
        });
    });
    group.finish();
}

criterion_group!(benches, indexing);
criterion_main!(benches);
