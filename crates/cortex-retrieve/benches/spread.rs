//! Spreading activation on synthetic graphs of 10,000, 100,000 and 1,000,000 nodes, the sizes a
//! retrieval meets from a small repository to past the huge mode's bound; and the huge mode's two
//! stages on the largest, over the graph's view by files prepared beforehand, as the daemon prepares
//! it when it rebuilds a huge repository's graph. `cargo xtask verify --full` reads their samples
//! and checks each against the retrieval budget, of which spreading is one step: 120 ms at the 95th
//! percentile.
//!
//! Each graph is built once, the same on every run: files of five symbols each, every symbol
//! referring to two symbols of other files, every file changing with three others in history and
//! worked on with two others by agents, placed by a fixed sequence of pseudo-random numbers. A
//! spread starts from five symbols spread over the graph, as a prompt naming five things does.

use std::hint::black_box;
use std::time::Duration;

use cortex_core::{EdgeType, NodeKind, TypeWeights};
use cortex_retrieve::{Graph, LinkInput, NodeInput, Standing, Weighing, spread, two_stage};
use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};

/// The nodes of each graph, files and symbols together.
const SIZES: [u32; 3] = [10_000, 100_000, 1_000_000];

/// The symbols of each file: a file and its symbols take six node numbers.
const SYMBOLS_PER_FILE: u32 = 5;

/// The restart probability retrieval spreads with.
const TELEPORT: f64 = 0.5;

/// The symbols a spread starts from.
const SEEDS: u32 = 5;

/// Spreads per benchmark: enough for a 95th percentile that is not the slowest run.
const SAMPLES: u32 = 100;

/// A fixed sequence of pseudo-random numbers (`SplitMix64`), so every run builds the same graph.
struct Sequence(u64);

impl Sequence {
    /// The next number below `bound`.
    fn below(&mut self, bound: u32) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        u32::try_from(mixed % u64::from(bound.max(1))).unwrap_or(0)
    }
}

/// The graph of `nodes` nodes, as the module documentation describes it.
fn graph(nodes: u32) -> Graph<u32> {
    let per_file = SYMBOLS_PER_FILE + 1;
    let files = nodes / per_file;
    let mut sequence = Sequence(u64::from(nodes));
    let mut inputs = Vec::new();
    let mut links = Vec::new();
    let link = |src, dst, kind, weight| LinkInput {
        src,
        dst,
        kind,
        weight,
        last_seq: 0,
    };
    for file in 0..files {
        let file_node = file * per_file;
        inputs.push(NodeInput {
            id: file_node,
            kind: NodeKind::File,
            file: None,
            standing: Standing::default(),
        });
        for symbol in 1..=SYMBOLS_PER_FILE {
            let id = file_node + symbol;
            inputs.push(NodeInput {
                id,
                kind: NodeKind::Symbol,
                file: Some(file_node),
                standing: Standing::default(),
            });
            for _ in 0..2 {
                let other = sequence.below(files) * per_file + 1 + sequence.below(SYMBOLS_PER_FILE);
                links.push(link(id, other, EdgeType::Structural, 0.9));
            }
        }
        for _ in 0..3 {
            let partner = sequence.below(files) * per_file;
            let confidence = 0.2 + f64::from(sequence.below(80)) / 100.0;
            links.push(link(file_node, partner, EdgeType::CoChange, confidence));
        }
        for _ in 0..2 {
            let partner = sequence.below(files) * per_file;
            links.push(link(file_node, partner, EdgeType::CoAccess, 1.0));
        }
    }
    let weighing = Weighing {
        type_weights: TypeWeights::default(),
        latest_seq: 0,
        link_decay: 0.995,
    };
    // A benchmark without its graph has nothing to measure.
    #[allow(clippy::expect_used)]
    Graph::build(&inputs, &links, &weighing).expect("a graph within 32-bit numbers")
}

/// The seeds of a spread over a graph of `nodes` nodes: [`SEEDS`] symbols evenly apart, each an
/// equal share. A graph's nodes are numbered in the order of their ids, which are the numbers
/// [`graph`] gives them, so a symbol's id is its number.
fn seeds(nodes: u32) -> Vec<(u32, f64)> {
    let per_file = SYMBOLS_PER_FILE + 1;
    let files = nodes / per_file;
    (0..SEEDS)
        .map(|at| ((files / SEEDS) * at * per_file + 1, 1.0 / f64::from(SEEDS)))
        .collect()
}

/// Spreading at each size, and the two stages on the largest, each sample one spreading: a tenth
/// of a millisecond of measuring time per sample is less than any spreading here takes, so
/// criterion runs each sample once (and warns that the samples take longer than the time it was
/// given).
fn spreading(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spread");
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(usize::try_from(SAMPLES).unwrap_or(100))
        .measurement_time(Duration::from_micros(100) * SAMPLES)
        .warm_up_time(Duration::from_millis(500));
    let mut largest = None;
    for nodes in SIZES {
        let built = graph(nodes);
        let start = seeds(nodes);
        let activation = spread(&built, &start, TELEPORT, None);
        assert!(
            activation.kept.len() > usize::try_from(SEEDS).unwrap_or(0),
            "a spread over {nodes} nodes reached only its seeds"
        );
        group.bench_with_input(
            BenchmarkId::new("nodes", nodes),
            &start,
            |bencher, start| {
                bencher.iter(|| black_box(spread(&built, start, TELEPORT, None)));
            },
        );
        largest = Some((nodes, built, start));
    }
    if let Some((nodes, built, start)) = largest {
        // A benchmark whose graph cannot be viewed by files has nothing to measure.
        #[allow(clippy::expect_used)]
        built.prepare_by_files().expect("the view by files");
        group.bench_with_input(
            BenchmarkId::new("two_stage_nodes", nodes),
            &start,
            |bencher, start| {
                bencher.iter(|| black_box(two_stage(&built, start, TELEPORT, None)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, spreading);
criterion_main!(benches);
