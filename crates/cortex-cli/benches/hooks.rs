//! Hook latency benchmarks: the real `cortex hook` binary, end to end from the process's start to
//! its exit, for each event the budgets name, on recorded Claude Code payloads, against a daemon
//! running on a medium repository (the index benchmark's 10,000 synthetic files, committed to git)
//! once its graph is built, so that a session's start gets the primer and the prompt a work order,
//! and an edit that changes a function's signature its warnings, as checked before they are
//! measured (every measured edit does the same work; its warnings, told once a task, are new only
//! the first time); the cold start, a hook given no input, which ends before
//! it asks the daemon anything; and each event again from a repository whose daemon is down, so
//! that the hook tries it, starts one and exits. Then retrieval alone: the prompt's work order
//! asked of the running daemon directly, with no hook process to start, each from a session of its
//! own. `cargo xtask verify --full` reads their samples and checks the budgets. Every sample is one
//! hook process or one request, so the percentiles are of single runs.

#[path = "../tests/common/mod.rs"]
mod common;

#[allow(
    dead_code,
    reason = "the generator's edits serve the index benchmark; these benchmarks only write it"
)]
#[path = "../../cortex-index/benches/synthetic/mod.rs"]
mod synthetic;

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::git::{commit, git};
use common::{Daemon, graph_of, isolate, payloads, stop_late_daemon};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, Event, HookKind, Identifier, RepoId, RunFiles, Ulid, UserFolder, canonical_root,
    create_private_folder, open_private_file,
};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, SamplingMode, criterion_group, criterion_main};

/// Hook processes per benchmark: enough for a 95th percentile that is not the slowest run.
const SAMPLES: usize = 100;

/// The Claude Code version whose recorded payloads the hooks read.
const RECORDED: &str = "2.1.312";

/// The prompt the benchmarks ask: it names a function of `pkg00/m000.py` and a class of the file
/// it imports from, `pkg02/m003.py`, so the prompt's words and an import agree on one file, as the
/// gate asks before it lets a work order through.
const PROMPT: &str = "helper_0_0_1 in pkg00/m000.py returns the wrong total from Model_2_3; fix it";

/// How long the daemon of the medium repository has for its first index, its mining and its first
/// graph: the first index alone took 16 s at the 95th percentile on Windows at the last gate.
const GRAPH_WITHIN: Duration = Duration::from_secs(300);

/// The file the recorded read and edit name, with its recorded contents before the edit.
const OPS_PY: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
    def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a // b\n";

/// The header of `add` before the measured edit, and after: the edit changes its signature.
const ADD_HEADERS: (&str, &str) = ("def add(a, b):", "def add(a, b, c=0):");

/// `main.py`, which calls `add` on line 3.
const MAIN_PY: &str = "from calc.ops import add\n\nprint(add(1, 2))\n";

/// `tests/test_ops.py`, which imports `calc/ops.py` and calls `add` on line 7.
const TEST_OPS_PY: &str = "import unittest\n\nfrom calc.ops import add\n\n\n\
    class OpsTest(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n";

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

/// A repository holding the recorded payloads' file, a caller of it and its tests, in a temporary
/// folder.
fn repository() -> tempfile::TempDir {
    // A benchmark without its repository has nothing to measure.
    #[allow(clippy::expect_used)]
    let repo = tempfile::tempdir().expect("a repository folder");
    for (path, text) in [
        ("calc/__init__.py", ""),
        ("calc/ops.py", OPS_PY),
        ("main.py", MAIN_PY),
        ("tests/__init__.py", ""),
        ("tests/test_ops.py", TEST_OPS_PY),
    ] {
        let path = repo.path().join(path);
        if let Some(folder) = path.parent() {
            #[allow(clippy::expect_used)]
            std::fs::create_dir_all(folder).expect("a folder of the repository");
        }
        #[allow(clippy::expect_used)]
        std::fs::write(&path, text).expect("a file of the repository");
    }
    repo
}

/// A medium repository: [`repository`]'s file and the synthetic 10,000 files, committed to git
/// with `home` as the user's home folder, then its tasks never held out, so every prompt can get
/// a work order.
fn medium_repository(home: &Path) -> tempfile::TempDir {
    let repo = repository();
    let root = repo.path();
    // A benchmark without its repository has nothing to measure.
    #[allow(clippy::expect_used)]
    synthetic::write_into(root).expect("the synthetic files");
    #[allow(clippy::expect_used)]
    git(root, home, &["init", "-q"]).expect("git init");
    #[allow(clippy::expect_used)]
    commit(root, home, "the repository", &["."]).expect("the first commit");
    #[allow(clippy::expect_used)]
    std::fs::create_dir(root.join(".cortex")).expect("the .cortex folder");
    #[allow(clippy::expect_used)]
    std::fs::write(root.join(".cortex/config.toml"), "[holdout]\nshare = 0\n")
        .expect("the repository's configuration");
    repo
}

/// The recorded payload `fixture`, its paths under the recorded root moved to `repo`, the prompt
/// [`PROMPT`] for a prompt's, and for the edit's, the edit of `add`'s header ([`ADD_HEADERS`]).
#[allow(
    clippy::indexing_slicing,
    reason = "the payload edited here is a JSON object, where indexing a missing key adds it"
)]
fn payload(fixture: &str, repo: &Path) -> Vec<u8> {
    // A benchmark without its payload has nothing to measure.
    #[allow(clippy::expect_used)]
    let root = canonical_root(repo).expect("the repository's canonical root");
    #[allow(clippy::expect_used)]
    let recorded = payloads::payload(RECORDED, fixture, &root).expect("a recorded payload");
    #[allow(clippy::expect_used)]
    let mut value: serde_json::Value =
        serde_json::from_slice(&recorded).expect("a recorded payload is JSON");
    if let Some(prompt) = value.get_mut("prompt") {
        *prompt = PROMPT.into();
    }
    if fixture == "post-tool-use-edit.json" {
        let (old, new) = ADD_HEADERS;
        value["tool_input"]["old_string"] = old.into();
        value["tool_input"]["new_string"] = new.into();
        let response = &mut value["tool_response"];
        response["oldString"] = old.into();
        response["newString"] = new.into();
        response["originalFile"] = OPS_PY.into();
        response["structuredPatch"] = serde_json::json!([{
            "oldStart": 4,
            "oldLines": 1,
            "newStart": 4,
            "newLines": 1,
            "lines": [format!("-{old}"), format!("+{new}")],
        }]);
    }
    #[allow(clippy::expect_used)]
    serde_json::to_vec(&value).expect("the payload as JSON")
}

/// Runs `cortex hook --agent claude --event <event>` on `payload`, from `repo`, with `home` as the
/// home folder, and returns how long the process took from its start to its exit.
fn run_hook(repo: &Path, home: &Path, event: &str, payload: &[u8]) -> Duration {
    let mut command = hook_command(repo, home, event);
    command.stdout(Stdio::null());
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

/// The `cortex hook --agent claude --event <event>` command, run from `repo` with `home` as the
/// home folder, its input piped and its errors dropped.
fn hook_command(repo: &Path, home: &Path, event: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["hook", "--agent", "claude", "--event", event])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stderr(Stdio::null());
    isolate(&mut command, home);
    command
}

/// What the hook for `event` prints on `payload`, run as [`run_hook`] runs it.
fn printed(repo: &Path, home: &Path, event: &str, payload: &[u8]) -> String {
    let mut command = hook_command(repo, home, event);
    command.stdout(Stdio::piped());
    // A hook that cannot be started or fed has nothing to show.
    #[allow(clippy::expect_used)]
    let mut child = command.spawn().expect("starting the hook");
    if let Some(mut stdin) = child.stdin.take() {
        #[allow(clippy::expect_used)]
        stdin.write_all(payload).expect("writing the payload");
    }
    #[allow(clippy::expect_used)]
    let output = child.wait_with_output().expect("waiting for the hook");
    String::from_utf8_lossy(&output.stdout).into_owned()
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
    /// The medium repository, a home folder, and a daemon on them that has built its first
    /// graph.
    fn start() -> Self {
        // A benchmark without its home folder or daemon has nothing to measure.
        #[allow(clippy::expect_used)]
        let home = tempfile::tempdir().expect("a home folder");
        let repo = medium_repository(home.path());
        #[allow(clippy::expect_used)]
        let daemon = Daemon::start(repo.path(), home.path()).expect("starting the daemon");
        #[allow(clippy::expect_used)]
        let ready = daemon.first_line().expect("the daemon's ready line");
        assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
        let deadline = Instant::now() + GRAPH_WITHIN;
        loop {
            #[allow(clippy::expect_used)]
            let graph = graph_of(repo.path(), home.path()).expect("the daemon's status");
            if graph.is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "no graph within {GRAPH_WITHIN:?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        Self { repo, home, daemon }
    }

    /// The run files of the daemon.
    fn files(&self) -> RunFiles {
        let user = UserFolder::in_home(self.home.path());
        // A benchmark that cannot find its daemon has nothing to measure.
        #[allow(clippy::expect_used)]
        let root = canonical_root(self.repo.path()).expect("the repository's canonical root");
        user.run_files(&RepoId::of(&root))
    }

    /// How many requests of each hook the daemon has handled, from its status.
    fn handled(&self) -> Vec<(String, u64)> {
        let files = self.files();
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

/// The work order the daemon at `files`, of the repository at `root`, gives [`PROMPT`] asked from
/// session `session`: the request's time from its start to the answer, and whether the answer held
/// a work order. The request waits as long as the daemon takes, so a slow retrieval is measured
/// whole rather than cut off.
fn retrieve(files: &RunFiles, root: &Path, session: u64) -> (Duration, bool) {
    // A benchmark that cannot make its request has nothing to measure.
    #[allow(clippy::expect_used)]
    let root = canonical_root(root).expect("the repository's canonical root");
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        });
    #[allow(clippy::expect_used)]
    let session_id = Identifier::new(&format!("bench-{session}")).expect("a session id");
    let event = Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms).unwrap_or(0), &session.to_be_bytes()),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id,
        parent_session_id: None,
        cwd: root.clone(),
        repo_root: root,
        hook: HookKind::Prompt,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: Some(PROMPT.to_owned()),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    };
    let body = RequestBody::Retrieve {
        event: Box::new(event),
        answer_by_ms: None,
    };
    let start = Instant::now();
    #[allow(clippy::expect_used)]
    let answer = cortex_hook::ask(files, Role::Hook, body, start + Duration::from_secs(10))
        .expect("the daemon's answer");
    let took = start.elapsed();
    let has_work_order = matches!(answer.body, ResponseBody::Reply { text: Some(_) });
    (took, has_work_order)
}

/// Each event's hook against the running daemon and with the daemon down, and the cold start.
fn hooks(criterion: &mut Criterion) {
    let bench = Bench::start();
    let (repo, home) = (bench.repo.path(), bench.home.path());
    // What is measured is the answer a user gets: a session's start its primer, and the prompt a
    // work order. A hook that printed nothing here would be timing silence.
    for (event, fixture) in [
        ("SessionStart", "session-start-startup.json"),
        ("UserPromptSubmit", "user-prompt-submit.json"),
    ] {
        let said = printed(repo, home, event, &payload(fixture, repo));
        assert!(
            !said.trim().is_empty(),
            "the hook for {event} printed nothing"
        );
    }
    // The edit, made once the prompt's task is open, is told of `add`'s callers.
    let (old, new) = ADD_HEADERS;
    #[allow(clippy::expect_used)]
    std::fs::write(repo.join("calc/ops.py"), OPS_PY.replace(old, new)).expect("the edit");
    let said = printed(
        repo,
        home,
        "PostToolUse",
        &payload("post-tool-use-edit.json", repo),
    );
    assert!(
        said.contains("add's signature changed"),
        "the edit was told: {said}"
    );
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
    // Retrieval alone, as the daemon answers a prompt: each request from a session of its own, so
    // each is a session's first prompt, and each answered with a work order, checked as it runs.
    let files = bench.files();
    let mut sessions = 0..;
    let mut group = criterion.benchmark_group("retrieval");
    single_runs(&mut group);
    group.bench_function("medium_10k", |bencher| {
        bencher.iter_custom(|runs| {
            (0..runs)
                .map(|_| {
                    let session = sessions.next().unwrap_or(u64::MAX);
                    let (took, has_work_order) = retrieve(&files, repo, session);
                    assert!(
                        has_work_order,
                        "the prompt of session {session} got no work order"
                    );
                    took
                })
                .sum()
        });
    });
    group.finish();
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
