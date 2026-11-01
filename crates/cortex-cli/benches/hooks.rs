//! Hook latency benchmarks: the real `cortex hook` binary, end to end from the process's start to
//! its exit, for each event the budgets name, on recorded Claude Code payloads, against a daemon
//! running on a small repository; and the cold start, a hook given no input, which ends before it
//! asks the daemon anything. `cargo xtask verify --full` reads their samples and checks the
//! budgets. Every sample is one hook process, so the percentiles are of single runs.

#[path = "../tests/common/mod.rs"]
mod common;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Daemon, isolate};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{RepoId, UserFolder, canonical_root};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};
use serde_json::Value;

/// Hook processes per benchmark: enough for a 95th percentile that is not the slowest run.
const SAMPLES: usize = 100;

/// The recorded payloads' folder.
const FIXTURES: &str = "../../tests/fixtures/claude/2.1.312";

/// The repository root the recorded payloads name, replaced by the benchmark's own.
const RECORDED_ROOT: &str = r"C:\repo";

/// The file the recorded read and edit name, with its recorded contents before the edit.
const OPS_PY: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
    def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a // b\n";

/// Each benchmark: its name, the event the hook is run for, and the recorded payload it reads.
const EVENTS: [(&str, &str, &str); 5] = [
    (
        "session_start",
        "SessionStart",
        "session-start-startup.json",
    ),
    (
        "user_prompt_submit",
        "UserPromptSubmit",
        "user-prompt-submit.json",
    ),
    ("pre_tool_use_read", "PreToolUse", "pre-tool-use-read.json"),
    (
        "post_tool_use_edit",
        "PostToolUse",
        "post-tool-use-edit.json",
    ),
    ("stop", "Stop", "stop.json"),
];

/// A repository with a running daemon, and the user's home folder both use.
struct Bench {
    /// The repository.
    repo: tempfile::TempDir,
    /// The user's home folder.
    home: tempfile::TempDir,
    /// The daemon, stopped when dropped.
    daemon: Daemon,
}

impl Bench {
    /// A repository holding the recorded payloads' file, a home folder, and a daemon on them that
    /// has finished its first index.
    fn start() -> Self {
        // A benchmark without its repository or daemon has nothing to measure.
        #[allow(clippy::expect_used)]
        let (repo, home) = (
            tempfile::tempdir().expect("a repository folder"),
            tempfile::tempdir().expect("a home folder"),
        );
        let calc = repo.path().join("calc");
        #[allow(clippy::expect_used)]
        std::fs::create_dir_all(&calc).expect("the calc folder");
        #[allow(clippy::expect_used)]
        std::fs::write(calc.join("ops.py"), OPS_PY).expect("calc/ops.py");
        #[allow(clippy::expect_used)]
        let daemon = Daemon::start(repo.path(), home.path()).expect("starting the daemon");
        #[allow(clippy::expect_used)]
        let ready = daemon.first_line().expect("the daemon's ready line");
        assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
        Self { repo, home, daemon }
    }

    /// The recorded payload `fixture`, its paths under the recorded root moved to this repository.
    fn payload(&self, fixture: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURES)
            .join(fixture);
        // A benchmark without its payload has nothing to measure.
        #[allow(clippy::expect_used)]
        let mut payload: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("reading a recorded payload"),
        )
        .expect("parsing a recorded payload");
        #[allow(clippy::expect_used)]
        let root = canonical_root(self.repo.path()).expect("the repository's canonical root");
        move_paths(&mut payload, &root);
        #[allow(clippy::expect_used)]
        serde_json::to_vec(&payload).expect("writing a payload")
    }

    /// Runs `cortex hook --agent claude --event <event>` on `payload`, from the repository, and
    /// returns how long the process took from its start to its exit.
    fn run(&self, event: &str, payload: &[u8]) -> Duration {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(["hook", "--agent", "claude", "--event", event])
            .current_dir(self.repo.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate(&mut command, self.home.path());
        let start = Instant::now();
        // A hook that cannot be started or fed has nothing to measure.
        #[allow(clippy::expect_used)]
        let mut child = command.spawn().expect("starting the hook");
        if let Some(mut stdin) = child.stdin.take() {
            #[allow(clippy::expect_used)]
            stdin.write_all(payload).expect("writing the payload");
        }
        #[allow(clippy::expect_used)]
        let status = child.wait().expect("waiting for the hook");
        let took = start.elapsed();
        assert!(status.success(), "the hook for {event} failed: {status}");
        took
    }

    /// How many requests of each hook the daemon has handled, from its status.
    fn handled(&self) -> Vec<(String, u64)> {
        let user = UserFolder::in_home(self.home.path());
        #[allow(clippy::expect_used)]
        let root = canonical_root(self.repo.path()).expect("the repository's canonical root");
        let files = user.run_files(&RepoId::of(&root));
        let deadline = Instant::now() + Duration::from_secs(10);
        #[allow(clippy::expect_used)]
        let answer = cortex_hook::ask(&files, Role::Cli, RequestBody::Status, deadline)
            .expect("asking the daemon for its status");
        let ResponseBody::Status(report) = answer.body else {
            return Vec::new();
        };
        report
            .latencies
            .into_iter()
            .map(|latency| (latency.hook, latency.requests))
            .collect()
    }
}

/// Moves every path in `value` under [`RECORDED_ROOT`] to the same place under `root`.
fn move_paths(value: &mut Value, root: &Path) {
    match value {
        Value::String(text) => {
            if let Some(rest) = text.strip_prefix(RECORDED_ROOT) {
                let mut path = PathBuf::from(root);
                path.extend(rest.split('\\').filter(|part| !part.is_empty()));
                *text = path.to_string_lossy().into_owned();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| move_paths(item, root)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| move_paths(field, root)),
        _ => {}
    }
}

/// Each event's hook against the running daemon, and the cold start.
fn hooks(criterion: &mut Criterion) {
    let bench = Bench::start();
    let mut group = criterion.benchmark_group("hooks");
    // One run per sample: a tenth of a millisecond of measuring time per sample is less than any
    // process takes to start, so criterion runs each sample once (and warns that the samples take
    // longer than the time it was given).
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(SAMPLES)
        .measurement_time(Duration::from_micros(100) * u32::try_from(SAMPLES).unwrap_or(100))
        .warm_up_time(Duration::from_secs(1));
    for (function, event, fixture) in EVENTS {
        let payload = bench.payload(fixture);
        group.bench_function(function, |bencher| {
            bencher.iter_custom(|runs| (0..runs).map(|_| bench.run(event, &payload)).sum());
        });
    }
    group.bench_function("cold_start", |bencher| {
        bencher.iter_custom(|runs| (0..runs).map(|_| bench.run("SessionStart", b"")).sum());
    });
    group.finish();
    // The hooks reached the daemon: each event's requests, the warm-up's included, are in its
    // handling times. A hook that failed open without asking would leave its event out.
    let handled = bench.handled();
    for hook in ["session_start", "prompt", "pre_tool", "post_tool", "stop"] {
        let requests = handled
            .iter()
            .find(|(name, _)| name == hook)
            .map_or(0, |(_, requests)| *requests);
        assert!(
            requests >= u64::try_from(SAMPLES).unwrap_or(u64::MAX),
            "the daemon handled {requests} {hook} requests: {handled:?}"
        );
    }
    drop(bench.daemon);
}

criterion_group!(benches, hooks);
criterion_main!(benches);
