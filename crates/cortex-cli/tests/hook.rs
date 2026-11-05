//! `cortex hook` end to end: captured Claude Code payloads through the real binary to a real
//! daemon, and every way a call can go wrong. Each call ends with exit code 0 and nothing on
//! stderr, as an agent requires of a hook, and nothing on stdout but the primer at a session's
//! start.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use common::fake_daemon::{FakeDaemon, Script, kinds};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, payloads, wait_for};
use cortex_core::ipc::{Request, RequestBody};
use cortex_core::{EventKind, HookKind, Identifier, LineRange, UserFolder};
use cortex_store::Reader;

/// One stored event row: its kind, its repository path and its lines.
type Row = (EventKind, Option<String>, Option<LineRange>);

/// The main session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

/// The Claude Code version whose captured payloads these tests send.
const CAPTURED: &str = "2.1.312";

/// What a session's start gives Claude Code in the repository of [`repository`] once its daemon's
/// graph is built: the primer of tiny mode, counting its one source file of 14 lines, with no
/// landmarks. None is ever chosen here: one task ends in these tests (the captured session's, at
/// its stop; the events after it belong to none), and it gives the file at most 1.8 of interest (a
/// read of the whole file 0.3, an edit 1.0, tests passing after it 0.5), under the 3.0 a landmark
/// needs.
const PRIMER: &str = "[cortex] Project map · tiny mode · about 1 source file, 14 lines\n\
                      Cortex records what agents do here; with this little code it adds nothing to \
                      prompts.\n";

/// The captured payload `name` with its paths moved under `root`.
fn payload(name: &str, root: &Path) -> TestResult<Vec<u8>> {
    payloads::payload(CAPTURED, name, root)
}

/// Sends the captured payload `name` from a repository at `root` through `cortex hook`, and returns
/// the context it gave Claude Code: none when it printed nothing. The call must end as an agent
/// requires of a hook.
fn send(name: &str, root: &Path, home: &Path) -> TestResult<Option<String>> {
    let payload = payload(name, root)?;
    let event = payloads::event_name(&payload)?;
    let call = hook(
        &["--agent", "claude", "--event", &event],
        payload,
        Duration::ZERO,
        home,
    )?;
    call.context(&event)
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

/// A session of the captured payloads, sent in the order Claude Code sent them once the daemon's
/// graph is built, is recorded as the rows the recording rules give, counted by hand: the start,
/// the prompt, the read of the whole file (no lines), the edit of line 13 (the one it changed), the
/// shell, the stop and the end; the moment before the read is not stored. The start prints the
/// primer, and nothing else prints anything: a tiny repository's prompt gets nothing. Of the other
/// captured payloads, the starts after a resume and a clear print the primer too, the start after a
/// compaction nothing, and the rest nothing.
#[test]
fn a_session_through_the_binary_is_recorded() -> TestResult {
    let (home, repo) = repository()?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
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
        let primer = (name == "session-start-startup.json").then(|| PRIMER.to_owned());
        assert_eq!(send(name, &repo, home.path())?, primer, "{name}");
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
    let mut others: Vec<PathBuf> = std::fs::read_dir(payloads::fixtures(CAPTURED))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    others.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    // The same order on every platform: a folder lists its entries in an order of its own.
    others.sort();
    for path in others {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("a fixture name")?;
        if session.contains(&name) {
            continue;
        }
        let primer = matches!(
            name,
            "session-start-resume.json" | "session-start-clear.json"
        )
        .then(|| PRIMER.to_owned());
        assert_eq!(send(name, &repo, home.path())?, primer, "{name}");
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

/// A daemon that takes the event and never answers is given up at the event's deadline (500 ms
/// for a tool's result, less the 15 ms the hook keeps), not before, and so is one that never
/// answers a prompt's request for its work order (1,000 ms, less the same); the bound after it is
/// loose, since starting a process on a busy machine is slow. One that speaks another protocol
/// gets nothing more. Every call is silent.
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
    // A prompt asks for its work order and is given up at its own deadline, 1,000 ms less the
    // 15 ms the hook keeps: it prints nothing either.
    let (release, released) = mpsc::channel();
    let silent = FakeDaemon::start(&repo, home.path(), Script::Silent(released))?;
    let call = hook(
        &["--agent", "claude", "--event", "UserPromptSubmit"],
        payload("user-prompt-submit.json", &repo)?,
        Duration::ZERO,
        home.path(),
    )?;
    let _ = release.send(());
    assert!(call.is_silent_success(), "{:?}", call.output);
    assert!(call.took >= Duration::from_millis(985), "{:?}", call.took);
    assert!(call.took < Duration::from_secs(20), "{:?}", call.took);
    assert_eq!(kinds(&silent.requests()?), ["hello", "retrieve"]);

    let foreign = FakeDaemon::start(&repo, home.path(), Script::Mismatch("99.0.0"))?;
    let call = hook(
        &["--agent", "claude", "--event", "Stop"],
        stop_payload(&repo)?,
        Duration::ZERO,
        home.path(),
    )?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    // A daemon of a newer Cortex is left alone: the hook sends nothing after its handshake and
    // event.
    assert_eq!(kinds(&foreign.requests()?), ["hello", "event"]);
    Ok(())
}

/// A hook that finds a daemon of an older Cortex, once the daemon has taken its event, asks it to
/// stop, so that the next hook starts a daemon of this build; one of another, older protocol takes
/// no event but is told to stop all the same. A daemon of a newer Cortex is left alone, so an older
/// binary never stops a newer daemon. Every call is silent.
#[test]
fn an_older_daemon_is_told_to_stop_and_a_newer_one_is_not() -> TestResult {
    let (home, repo) = marked_repository()?;
    let edit = || payload("post-tool-use-edit.json", &repo);
    let call = |input| {
        hook(
            &["--agent", "claude", "--event", "PostToolUse"],
            input,
            Duration::ZERO,
            home.path(),
        )
    };
    let older = FakeDaemon::start(&repo, home.path(), Script::Welcome(&["0.0.1"]))?;
    let done = call(edit()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    assert_eq!(
        kinds(&older.requests()?),
        ["hello", "event", "hello", "shutdown"]
    );
    let older_protocol = FakeDaemon::start(&repo, home.path(), Script::Mismatch("0.0.1"))?;
    let done = call(edit()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    assert_eq!(
        kinds(&older_protocol.requests()?),
        ["hello", "event", "hello", "shutdown"]
    );
    let newer = FakeDaemon::start(&repo, home.path(), Script::Welcome(&["99.0.0"]))?;
    let done = call(edit()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    assert_eq!(kinds(&newer.requests()?), ["hello", "event"]);
    // An older daemon that a newer one replaced between the hook's two connections: the hook reads
    // the handshake of the second before it asks anything, and leaves the newer daemon running.
    let replaced = FakeDaemon::start(&repo, home.path(), Script::Welcome(&["0.0.1", "99.0.0"]))?;
    let done = call(edit()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    assert_eq!(kinds(&replaced.requests()?), ["hello", "event", "hello"]);
    Ok(())
}

/// A probe, `cortex doctor`'s, measures the daemon that runs and changes none: a daemon of an
/// older Cortex takes the probe and is not told to stop, and with no daemon running the probe
/// starts none, which the hook log says. Both calls are silent.
#[test]
fn a_probe_stops_and_starts_no_daemon() -> TestResult {
    let (home, repo) = marked_repository()?;
    let probe = || -> TestResult<Vec<u8>> {
        let mut edit: serde_json::Value =
            serde_json::from_slice(&payload("post-tool-use-edit.json", &repo)?)?;
        edit["cortex_probe"] = serde_json::Value::Bool(true);
        Ok(serde_json::to_vec(&edit)?)
    };
    let call = |input| {
        hook(
            &["--agent", "claude", "--event", "PostToolUse"],
            input,
            Duration::ZERO,
            home.path(),
        )
    };
    let older = FakeDaemon::start(&repo, home.path(), Script::Welcome(&["0.0.1"]))?;
    let done = call(probe()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    assert_eq!(kinds(&older.requests()?), ["hello", "event"]);
    let done = call(probe()?)?;
    assert!(done.is_silent_success(), "{:?}", done.output);
    let log_path = UserFolder::in_home(home.path())
        .logs_folder()
        .join("hook.log");
    let log = std::fs::read_to_string(&log_path)?;
    assert!(
        log.contains("claude PostToolUse: a probe found no daemon, and probes start none"),
        "{log}"
    );
    Ok(())
}

/// A file system that stalls is given up at the hard timeout. Only a Unix one can be made to
/// stall on purpose: a pipe nobody reads makes an open for writing wait for ever.
#[cfg(unix)]
mod stalled {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use cortex_core::UserFolder;

    use super::common::{POLL_EVERY, TestResult, isolate};
    use super::marked_repository;

    /// How long a hook may run before the test takes it for stuck: far past any hard timeout.
    const STUCK_AFTER: Duration = Duration::from_secs(10);

    /// The most a call here may take: its hard timeout is 300 ms, and the rest is for starting a
    /// process on a busy machine. A hook that waited for the agent's own timeout (one second or
    /// more) to end it would take longer.
    const RETURNS_WITHIN: Duration = Duration::from_secs(2);

    /// Makes `path` a pipe nobody reads, last changed long ago, so that nothing takes it for
    /// recent and opening it to write waits for ever.
    fn stalling_pipe(path: &Path) -> TestResult {
        let made = Command::new("mkfifo").arg(path).status()?;
        let aged = Command::new("touch")
            .args(["-m", "-t", "200001010000"])
            .arg(path)
            .status()?;
        if !(made.success() && aged.success()) {
            return Err(format!("no pipe made at {}", path.display()).into());
        }
        Ok(())
    }

    /// Runs `cortex hook` with `args` and `payload` on stdin, `home` the home folder and its output
    /// to files: whether it ended with exit code 0 and nothing printed within [`RETURNS_WITHIN`]. A
    /// hook still running after [`STUCK_AFTER`] is stopped, and that is an error.
    fn bounded_hook(args: &[&str], payload: &[u8], home: &Path) -> TestResult<bool> {
        let (out, err) = (home.join("hook.out"), home.join("hook.err"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .arg("hook")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(File::create(&out)?)
            .stderr(File::create(&err)?);
        isolate(&mut command, home);
        let started = Instant::now();
        let mut child = command.spawn()?;
        // The hook may have given up its input already: that is its right.
        let _ = child
            .stdin
            .take()
            .ok_or("stdin is not piped")?
            .write_all(payload);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > STUCK_AFTER {
                child.kill()?;
                child.wait()?;
                return Err("the hook was still running after 10 s".into());
            }
            std::thread::sleep(POLL_EVERY);
        };
        Ok(started.elapsed() < RETURNS_WITHIN
            && status.code() == Some(0)
            && std::fs::read(&out)?.is_empty()
            && std::fs::read(&err)?.is_empty())
    }

    /// With the hook log a pipe nobody reads, a payload that is no JSON makes the hook's work
    /// stall noting why, and then its note that it ran out of time stall too: the hook returns all
    /// the same, silent. So does a call whose arguments name no hook, which can only note that.
    #[test]
    fn a_stalled_log_is_given_up_at_the_hard_timeout() -> TestResult {
        let (home, repo) = marked_repository()?;
        let logs = UserFolder::in_home(home.path()).logs_folder();
        std::fs::create_dir_all(&logs)?;
        stalling_pipe(&logs.join("hook.log"))?;
        let not_json = format!("{{\"cwd\": {:?}", repo.display().to_string());
        let stop = ["--agent", "claude", "--event", "Stop"];
        assert!(bounded_hook(&stop, not_json.as_bytes(), home.path())?);
        let unknown = ["--agent", "claude", "--event", "NoSuchEvent"];
        assert!(bounded_hook(&unknown, b"{}", home.path())?);
        Ok(())
    }
}
