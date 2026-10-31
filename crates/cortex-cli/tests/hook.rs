//! `cortex hook` end to end: captured Claude Code payloads through the real binary to a real
//! daemon, and every way a call can go wrong. Each call ends with exit code 0 and nothing on stdout
//! or stderr, as an agent requires of a hook.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::{Daemon, TestResult, isolate, wait_for};
use cortex_core::ipc::{
    Mismatch, Protocol, Request, RequestBody, Response, ResponseBody, read_frame, write_frame,
};
use cortex_core::{
    Endpoint, EventKind, HookKind, Identifier, LineRange, RepoId, RunFiles, UserFolder,
    canonical_root,
};
use cortex_store::Reader;
use interprocess::local_socket::traits::{Listener as _, Stream as _};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, Name, Stream, ToFsName, ToNsName,
};

/// One stored event row: its kind, its repository path and its lines.
type Row = (EventKind, Option<String>, Option<LineRange>);

/// The main session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The folder of the captured payloads.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude/2.1.312")
}

/// `windows`, a path the fixtures write under `C:\repo`, moved under `root`.
fn under(root: &Path, windows: &str) -> Option<String> {
    let inside = windows.strip_prefix(r"C:\repo")?;
    let moved = inside
        .split('\\')
        .filter(|name| !name.is_empty())
        .fold(root.to_path_buf(), |path, name| path.join(name));
    Some(moved.to_string_lossy().into_owned())
}

/// The captured payload `name` with its working folder and tool paths moved under `root`.
fn payload(name: &str, root: &Path) -> TestResult<Vec<u8>> {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures().join(name))?)?;
    let rewrite = |field: Option<&mut serde_json::Value>| {
        if let Some(serde_json::Value::String(path)) = field
            && let Some(moved) = under(root, path)
        {
            *path = moved;
        }
    };
    rewrite(value.get_mut("cwd"));
    if let Some(input) = value.get_mut("tool_input") {
        rewrite(input.get_mut("file_path"));
        rewrite(input.get_mut("path"));
    }
    Ok(serde_json::to_vec(&value)?)
}

/// The payload's event name.
fn event_name(payload: &[u8]) -> TestResult<String> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;
    Ok(value["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned())
}

/// How a hook call went: its output and how long it took.
struct Call {
    /// What the process left behind.
    output: Output,
    /// From its start to its exit.
    took: Duration,
    /// Whether the whole input could be written: not when the hook had stopped reading and closed
    /// its end first.
    is_input_taken: bool,
}

impl Call {
    /// Whether the call ended as an agent requires: exit code 0, nothing printed.
    fn is_silent_success(&self) -> bool {
        self.output.status.code() == Some(0)
            && self.output.stdout.is_empty()
            && self.output.stderr.is_empty()
    }
}

/// Runs `cortex hook` with `args`, writing `input` to its standard input after `delay` from a
/// thread of its own (a hook that stops reading may close the pipe first), with `home` as the
/// user's home folder.
fn hook(args: &[&str], input: Vec<u8>, delay: Duration, home: &Path) -> TestResult<Call> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .arg("hook")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut command, home);
    let started = Instant::now();
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin is not piped")?;
    let writer = thread::spawn(move || {
        thread::sleep(delay);
        // The hook may have stopped reading and closed its end: that is its right.
        stdin.write_all(&input).is_ok()
    });
    let output = child.wait_with_output()?;
    let took = started.elapsed();
    let is_input_taken = writer.join().map_err(|_| "the writer panicked")?;
    Ok(Call {
        output,
        took,
        is_input_taken,
    })
}

/// Sends the captured payload `name` from a repository at `root` through `cortex hook`.
fn send(name: &str, root: &Path, home: &Path) -> TestResult<Call> {
    let payload = payload(name, root)?;
    let event = event_name(&payload)?;
    hook(
        &["--agent", "claude", "--event", &event],
        payload,
        Duration::ZERO,
        home,
    )
}

/// A home folder holding a repository, as most are, with the file the captured payloads read and
/// edit (14 lines, as in the capture). Inside a home folder, the hook looks for a repository no
/// higher than the home folder, so no folder above the temporary one is taken for a repository.
fn repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join("calc"))?;
    let lines: String = (1..=14)
        .map(|number| {
            format!(
                "x{number} = {number}
"
            )
        })
        .collect();
    std::fs::write(repo.join("calc/ops.py"), lines)?;
    Ok((home, repo))
}

/// A session of the captured payloads, sent in the order Claude Code sent them, is recorded as the
/// rows the recording rules give, counted by hand: the start, the prompt, the read of the whole
/// file (no lines), the edit of line 13 (the one it changed), the shell, the stop and the end; the
/// moment before the read is not stored. Every other captured payload goes through silently too.
#[test]
fn a_session_through_the_binary_is_recorded() -> TestResult {
    let (home, repo) = repository()?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let session = [
        "session-start-startup.json",
        "user-prompt-submit.json",
        "pre-tool-use-read.json",
        "post-tool-use-read.json",
        "post-tool-use-edit.json",
        "post-tool-use-bash.json",
        "stop.json",
        "session-end.json",
    ];
    for name in session {
        let call = send(name, &repo, home.path())?;
        assert!(call.is_silent_success(), "{name}: {:?}", call.output);
    }
    let s1 = Identifier::new(S1)?;
    let rows = || -> TestResult<Vec<Row>> {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_of_session(&s1)?
            .into_iter()
            .map(|row| {
                let path = row.record.path.map(|path| path.as_str().to_owned());
                (row.record.kind, path, row.record.range)
            })
            .collect())
    };
    wait_for(|| Ok(rows()?.len() >= 7))?;
    let ops = Some("calc/ops.py".to_owned());
    assert_eq!(
        rows()?,
        [
            (EventKind::SessionStart, None, None),
            (EventKind::Prompt, None, None),
            (EventKind::Read, ops.clone(), None),
            (EventKind::Edit, ops, Some(LineRange::new(13, 13)?)),
            (EventKind::Shell, None, None),
            (EventKind::Stop, None, None),
            (EventKind::SessionEnd, None, None),
        ]
    );
    let mut others: Vec<PathBuf> = std::fs::read_dir(fixtures())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    others.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    for path in others {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("a fixture name")?;
        if session.contains(&name) {
            continue;
        }
        let call = send(name, &repo, home.path())?;
        assert!(call.is_silent_success(), "{name}: {:?}", call.output);
    }
    Ok(())
}

/// A repository marked for Cortex, its `.cortex` folder made by hand, with no daemon running, and
/// an empty home folder.
fn marked_repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let (home, repo) = repository()?;
    std::fs::create_dir(repo.join(".cortex"))?;
    Ok((home, repo))
}

/// The stop payload from a session in `root`.
fn stop_payload(root: &Path) -> TestResult<Vec<u8>> {
    payload("stop.json", root)
}

/// Payloads that are malformed, too long, empty or too slow, and arguments other than the two
/// flags, `--help` included: every call exits 0 and prints nothing, and the slow payload is not
/// waited for. The log holds the first failure, and none of the payload's text.
#[test]
fn bad_calls_are_silent() -> TestResult {
    let (home, repo) = marked_repository()?;
    let flags = ["--agent", "claude", "--event", "Stop"];
    let mut too_long = br#"{"session_id":"MARKER0PAYLOAD","pad":""#.to_vec();
    too_long.resize(2 << 20, b'x');
    // A JSON text that is no object: the parser's own message would quote it.
    let quoted = br#""MARKER0PAYLOAD""#.to_vec();
    for input in [quoted, b"{{{".to_vec(), too_long, Vec::new()] {
        let call = hook(&flags, input, Duration::ZERO, home.path())?;
        assert!(call.is_silent_success(), "{:?}", call.output);
    }
    let log_path = UserFolder::in_home(home.path())
        .logs_folder()
        .join("hook.log");
    let log = std::fs::read_to_string(&log_path)?;
    assert!(
        log.contains("claude Stop: the payload is not the agent's hook JSON"),
        "{log}"
    );
    assert!(!log.contains("MARKER0PAYLOAD"), "{log}");

    let stop = stop_payload(&repo)?;
    // Five seconds late: the hook, which waits 50 ms for its payload, has exited by then and
    // closed its end, so the payload cannot be written.
    let slow = hook(&flags, stop.clone(), Duration::from_secs(5), home.path())?;
    assert!(slow.is_silent_success(), "{:?}", slow.output);
    assert!(!slow.is_input_taken, "{:?}", slow.took);
    for args in [
        &["--help"][..],
        &[],
        &["--agent", "claude"],
        &["--agent", "claude", "--event", "Stop", "--extra"],
        &["--agent", "nobody", "--event", "Stop"],
        &["--agent", "claude", "--event", "Notification"],
    ] {
        let call = hook(args, stop.clone(), Duration::ZERO, home.path())?;
        assert!(call.is_silent_success(), "{args:?}: {:?}", call.output);
    }
    Ok(())
}

/// A working folder with no `.cortex` folder at or above it is left alone: nothing printed, and
/// nothing written to the user's folder, not even a log line.
#[test]
fn folders_without_cortex_are_left_alone() -> TestResult {
    let (home, repo) = repository()?;
    let call = hook(
        &["--agent", "claude", "--event", "Stop"],
        stop_payload(&repo)?,
        Duration::ZERO,
        home.path(),
    )?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    assert!(!UserFolder::in_home(home.path()).path().exists());
    Ok(())
}

/// What a fake daemon does once a hook connects.
enum Script {
    /// Reads the handshake and the request, then answers nothing until told.
    Silent(mpsc::Receiver<()>),
    /// Answers the handshake with a mismatch.
    Mismatch,
}

/// A fake daemon serving one connection on a thread of its own.
struct FakeDaemon {
    /// Where it listens.
    files: RunFiles,
    /// The thread, which returns the requests it read.
    thread: thread::JoinHandle<Result<Vec<Request>, String>>,
}

impl FakeDaemon {
    /// A fake daemon at the endpoint of the repository at `root`, its run folder and process-id
    /// file made as the real one makes them, serving one connection by `script`.
    fn start(root: &Path, home: &Path, script: Script) -> TestResult<Self> {
        let user = UserFolder::in_home(home);
        std::fs::create_dir_all(user.run_folder())?;
        let files = user.run_files(&RepoId::of(&canonical_root(root)?));
        std::fs::write(files.pid_path(), format!("{}\n", std::process::id()))?;
        let listener: Listener = ListenerOptions::new()
            .name(endpoint_name(&files)?)
            .create_sync()?;
        let thread = thread::spawn(move || serve(&listener, script));
        Ok(Self { files, thread })
    }

    /// The requests the fake daemon read. When no hook connected, a connection of the test's own
    /// ends its wait, so a hook that never asks fails the test instead of hanging it.
    fn requests(self) -> TestResult<Vec<Request>> {
        if !self.thread.is_finished() {
            // The fake daemon may be past accepting already, and then nothing answers: no matter.
            let _ = endpoint_name(&self.files).and_then(Stream::connect);
        }
        Ok(self
            .thread
            .join()
            .map_err(|_| "the fake daemon panicked")??)
    }
}

/// The name of the endpoint of `files`, as the socket library takes it.
fn endpoint_name(files: &RunFiles) -> std::io::Result<Name<'static>> {
    Ok(match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?.into_owned(),
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?.into_owned(),
    })
}

/// Serves one connection at `listener` by `script`, and returns the requests read.
fn serve(listener: &Listener, script: Script) -> Result<Vec<Request>, String> {
    let mut stream = listener.accept().map_err(|error| error.to_string())?;
    let hello: Request = read_frame(&mut stream).map_err(|error| error.to_string())?;
    match script {
        Script::Mismatch => {
            let answer = Response {
                request_id: hello.request_id,
                body: ResponseBody::Mismatch(Mismatch {
                    protocol: Protocol {
                        major: 99,
                        minor: 0,
                    },
                    cortex_version: "99.0.0".to_owned(),
                }),
            };
            write_frame(&mut stream, &answer).map_err(|error| error.to_string())?;
            Ok(vec![hello])
        }
        Script::Silent(release) => {
            let request: Request = read_frame(&mut stream).map_err(|error| error.to_string())?;
            // Released once the hook has exited; a hook that never gives up meets the timeout
            // instead, and the test fails on its time rather than hanging.
            let _ = release.recv_timeout(Duration::from_secs(30));
            Ok(vec![hello, request])
        }
    }
}

/// A daemon that takes the event and never answers is given up at the event's deadline (500 ms
/// for a tool's result, less the 15 ms the hook keeps), not before; the bound after it is loose,
/// since starting a process on a busy machine is slow. One that speaks another protocol gets
/// nothing more. Both calls are silent.
#[test]
fn a_silent_or_foreign_daemon_is_left_silently() -> TestResult {
    let (home, repo) = marked_repository()?;
    // The first start of a freshly built binary can take seconds (a virus scanner reads it), which
    // the timed call below must not pay.
    hook(&[], Vec::new(), Duration::ZERO, home.path())?;
    let (release, released) = mpsc::channel();
    let silent = FakeDaemon::start(&repo, home.path(), Script::Silent(released))?;
    let edit = payload("post-tool-use-edit.json", &repo)?;
    let call = hook(
        &["--agent", "claude", "--event", "PostToolUse"],
        edit,
        Duration::ZERO,
        home.path(),
    )?;
    // The fake daemon may have stopped already; it is let go either way.
    let _ = release.send(());
    assert!(call.is_silent_success(), "{:?}", call.output);
    assert!(call.took >= Duration::from_millis(485), "{:?}", call.took);
    assert!(call.took < Duration::from_secs(20), "{:?}", call.took);
    let requests = silent.requests()?;
    let sent_the_event = matches!(
        &requests[..],
        [_, Request { body: RequestBody::Event { event }, .. }] if event.hook == HookKind::PostTool
    );
    assert!(sent_the_event, "{requests:?}");

    let foreign = FakeDaemon::start(&repo, home.path(), Script::Mismatch)?;
    let call = hook(
        &["--agent", "claude", "--event", "Stop"],
        stop_payload(&repo)?,
        Duration::ZERO,
        home.path(),
    )?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    let requests = foreign.requests()?;
    assert_eq!(requests.len(), 1);
    Ok(())
}
