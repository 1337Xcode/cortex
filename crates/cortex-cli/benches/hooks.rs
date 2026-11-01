//! Hook latency benchmarks: the real `cortex hook` binary, end to end from the process's start to
//! its exit, for each event the budgets name, on recorded Claude Code payloads, against a daemon
//! running on a small repository; the cold start, a hook given no input, which ends before it asks
//! the daemon anything; and each event again from a repository whose daemon is down, so that the
//! hook tries it, starts one and exits. `cargo xtask verify --full` reads their samples and checks
//! the budgets. Every sample is one hook process, so the percentiles are of single runs.

#[path = "../tests/common/mod.rs"]
mod common;

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Daemon, isolate, payloads, stop_late_daemon};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    RepoId, RunFiles, UserFolder, canonical_root, create_private_folder, open_private_file,
};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, SamplingMode, criterion_group, criterion_main};

/// Hook processes per benchmark: enough for a 95th percentile that is not the slowest run.
const SAMPLES: usize = 100;

/// The Claude Code version whose recorded payloads the hooks read.
const RECORDED: &str = "2.1.312";

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

/// A repository holding the recorded payloads' file, in a temporary folder.
fn repository() -> tempfile::TempDir {
    // A benchmark without its repository has nothing to measure.
    #[allow(clippy::expect_used)]
    let repo = tempfile::tempdir().expect("a repository folder");
    let calc = repo.path().join("calc");
    #[allow(clippy::expect_used)]
    std::fs::create_dir_all(&calc).expect("the calc folder");
    #[allow(clippy::expect_used)]
    std::fs::write(calc.join("ops.py"), OPS_PY).expect("calc/ops.py");
    repo
}

/// The recorded payload `fixture`, its paths under the recorded root moved to `repo`.
fn payload(fixture: &str, repo: &Path) -> Vec<u8> {
    // A benchmark without its payload has nothing to measure.
    #[allow(clippy::expect_used)]
    let root = canonical_root(repo).expect("the repository's canonical root");
    #[allow(clippy::expect_used)]
    payloads::payload(RECORDED, fixture, &root).expect("a recorded payload")
}

/// Runs `cortex hook --agent claude --event <event>` on `payload`, from `repo`, with `home` as the
/// home folder, and returns how long the process took from its start to its exit.
fn run_hook(repo: &Path, home: &Path, event: &str, payload: &[u8]) -> Duration {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["hook", "--agent", "claude", "--event", event])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    isolate(&mut command, home);
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
        let repo = repository();
        // A benchmark without its home folder or daemon has nothing to measure.
        #[allow(clippy::expect_used)]
        let home = tempfile::tempdir().expect("a home folder");
        #[allow(clippy::expect_used)]
        let daemon = Daemon::start(repo.path(), home.path()).expect("starting the daemon");
        #[allow(clippy::expect_used)]
        let ready = daemon.first_line().expect("the daemon's ready line");
        assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
        Self { repo, home, daemon }
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

/// A repository marked for Cortex whose daemon is down and cannot come up: the benchmark holds its
/// run lock, as a daemon still starting does, so each daemon a hook starts finds the repository
/// held and exits.
struct Down {
    /// The held run lock, released when dropped.
    held: Option<File>,
    /// Its run files, the start's marker among them.
    files: RunFiles,
    /// The repository.
    repo: tempfile::TempDir,
}

impl Drop for Down {
    /// Lets go of the run lock, then stops a daemon the last hook may have started and that took
    /// the lock once it was free, before the repository goes.
    fn drop(&mut self) {
        drop(self.held.take());
        stop_late_daemon(&self.files);
    }
}

impl Down {
    /// A repository holding the recorded payloads' file and a `.cortex` folder, its run lock held
    /// in the user's folder under `home`.
    fn hold(home: &Path) -> Self {
        let repo = repository();
        let user = UserFolder::in_home(home);
        // A benchmark without its repository held has nothing to measure.
        #[allow(clippy::expect_used)]
        std::fs::create_dir(repo.path().join(".cortex")).expect("the .cortex folder");
        #[allow(clippy::expect_used)]
        let root = canonical_root(repo.path()).expect("the repository's canonical root");
        let files = user.run_files(&RepoId::of(&root));
        #[allow(clippy::expect_used)]
        create_private_folder(&user.run_folder()).expect("the run folder");
        #[allow(clippy::expect_used)]
        let held = open_private_file(&files.lock_path()).expect("the run lock");
        #[allow(clippy::expect_used)]
        held.try_lock().expect("holding the run lock");
        Self {
            held: Some(held),
            files,
            repo,
        }
    }

    /// Runs the hook for `event` on `payload` with `home` as the home folder, as [`run_hook`]
    /// does, with no start of a daemon in the last ten seconds, so that the hook makes one; and
    /// checks that it did.
    fn run(&self, home: &Path, event: &str, payload: &[u8]) -> Duration {
        if let Err(error) = std::fs::remove_file(self.files.start_path()) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::NotFound,
                "removing the start's marker: {error}"
            );
        }
        let took = run_hook(self.repo.path(), home, event, payload);
        assert!(
            self.files.start_path().exists(),
            "the hook for {event} made no start"
        );
        took
    }
}

/// Configures `group` for one hook process per sample: a tenth of a millisecond of measuring time
/// per sample is less than any process takes to start, so criterion runs each sample once (and
/// warns that the samples take longer than the time it was given).
fn single_runs(group: &mut BenchmarkGroup<'_, WallTime>) {
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(SAMPLES)
        .measurement_time(Duration::from_micros(100) * u32::try_from(SAMPLES).unwrap_or(100))
        .warm_up_time(Duration::from_secs(1));
}

/// Each event's hook against the running daemon and with the daemon down, and the cold start.
fn hooks(criterion: &mut Criterion) {
    let bench = Bench::start();
    let (repo, home) = (bench.repo.path(), bench.home.path());
    let mut group = criterion.benchmark_group("hooks");
    single_runs(&mut group);
    for (function, event, fixture) in EVENTS {
        let payload = payload(fixture, repo);
        group.bench_function(function, |bencher| {
            bencher.iter_custom(|runs| {
                (0..runs)
                    .map(|_| run_hook(repo, home, event, &payload))
                    .sum()
            });
        });
    }
    group.bench_function("cold_start", |bencher| {
        bencher.iter_custom(|runs| {
            (0..runs)
                .map(|_| run_hook(repo, home, "SessionStart", b""))
                .sum()
        });
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
    let down = Down::hold(home);
    let mut group = criterion.benchmark_group("hooks_daemon_down");
    single_runs(&mut group);
    for (function, event, fixture) in EVENTS {
        let payload = payload(fixture, down.repo.path());
        group.bench_function(function, |bencher| {
            bencher.iter_custom(|runs| (0..runs).map(|_| down.run(home, event, &payload)).sum());
        });
    }
    group.finish();
    drop(down);
    drop(bench.daemon);
}

criterion_group!(benches, hooks);
criterion_main!(benches);
