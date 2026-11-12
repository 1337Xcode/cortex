//! The experience hint's worst case: what a work order pays, after the gate has opened, to look
//! for the hint when 200 past task records (as many as are read) are all like the prompt and none
//! can be told, because every file they changed is gone from disk. Each look reads the 200 records'
//! changes, finds each like the prompt, reads each whole record back, asks the store for its
//! files' paths and opens each path, finds none, then searches the change cards, of which there
//! are none. Measured is `experience_hint` alone, from its first query to its answer of no hint,
//! with the graph and the similar-task seeds already worked out, as the work order has them; the
//! rest of the retrieval is the hook benchmarks' to measure.
//!
//! The store is built once: three files of `src/orders/`, written to disk and indexed, then
//! deleted from disk; a cluster of past tasks whose first words are the prompt's; and 200 tasks,
//! each changing the three files and succeeding, ended before the task asking began.

#[path = "../tests/common/mod.rs"]
mod common;

use std::hint::black_box;
use std::time::Duration;

use common::{Repo, TestResult};
use cortex_core::{FileFlags, RepoPath, TaskOutcome, Ulid};
use cortex_retrieve::{
    ExperienceAsk, GraphSettings, MAX_EPISODES_READ, PromptTerms, experience_hint, read_graph,
    similar,
};
use cortex_store::{ChangedFile, EpisodeRecord, NodeId, Pitfalls};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};

/// Looks per benchmark: enough for a 95th percentile that is not the slowest look.
const SAMPLES: usize = 100;

/// The prompt, whose words are the cluster's first.
const PROMPT: &str = "orders service validate";

/// When the task asking began, in UTC milliseconds: after every record was written.
const START_MS: i64 = 1_000_000_000;

/// The files every record changed, gone from disk once indexed.
const PATHS: [&str; 3] = [
    "src/orders/service.py",
    "src/orders/validate.py",
    "src/orders/model.py",
];

/// The store the module documentation describes.
fn store() -> TestResult<Repo> {
    let mut repo = Repo::new()?;
    let mut nodes: Vec<NodeId> = Vec::new();
    for path in PATHS {
        nodes.extend(repo.file(path, "x = 1\n", &[], FileFlags::empty())?.first());
        std::fs::remove_file(RepoPath::new(path)?.to_native(repo.root.path()))?;
    }
    repo.clusters(&[(
        &["orders", "service", "validate", "model", "create"],
        nodes.as_slice(),
        200,
    )])?;
    let count = u32::try_from(MAX_EPISODES_READ)?;
    repo.tasks(count)?;
    let mut transaction = repo.writer.transaction()?;
    for at in 1..=count {
        transaction.replace_episode(&EpisodeRecord {
            task_id: Ulid::from_seed(u64::from(at), b"task"),
            localize: Vec::new(),
            change: nodes
                .iter()
                .map(|node| ChangedFile {
                    node: *node,
                    lines: 1,
                    symbols: Vec::new(),
                })
                .collect(),
            verify: Vec::new(),
            pitfalls: Pitfalls::default(),
            outcome: TaskOutcome::Success,
            created_at_ms: i64::from(at),
        })?;
    }
    transaction.commit()?;
    Ok(repo)
}

/// The benchmark: one look for the hint per sample, over the store built once.
fn experience(criterion: &mut Criterion) {
    // A benchmark whose store cannot be built has nothing to measure.
    #[allow(clippy::expect_used)]
    let repo = store().expect("the store");
    #[allow(clippy::expect_used)]
    let reader = repo.reader().expect("a reader");
    #[allow(clippy::expect_used)]
    let graph = read_graph(reader.queries(), &GraphSettings::default()).expect("the graph");
    let terms = PromptTerms::of(PROMPT);
    #[allow(clippy::expect_used)]
    let found = similar(reader.queries(), &graph, &terms).expect("the similar-task seeds");
    assert!(!found.files.is_empty(), "the cluster is like the prompt");
    let ask = ExperienceAsk {
        before_ms: START_MS,
        similar: &found,
        terms: &terms,
        root: repo.root.path(),
        deadline: None,
    };
    #[allow(clippy::expect_used)]
    let told = experience_hint(reader.queries(), &graph, &ask).expect("a look");
    assert_eq!(told, None, "no record can be told");
    let mut group = criterion.benchmark_group("experience");
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(SAMPLES)
        .measurement_time(Duration::from_millis(50) * u32::try_from(SAMPLES).unwrap_or(100))
        .warm_up_time(Duration::from_millis(500));
    group.bench_function("unrenderable_records_200", |bencher| {
        bencher.iter(|| black_box(experience_hint(reader.queries(), &graph, &ask)));
    });
    group.finish();
}

criterion_group!(benches, experience);
criterion_main!(benches);
