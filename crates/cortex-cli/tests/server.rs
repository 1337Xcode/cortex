//! The daemon's endpoint on temporary repositories with an empty home folder: `cortex status` asks
//! it, a Shutdown stops it, a client of another protocol gets a mismatch and may still stop it, a
//! broken client changes nothing but the daemon's recent errors, each hook's handling time is
//! reported, and the endpoint is the user's alone.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Daemon, POLL_EVERY, TestResult, isolate};
use cortex_core::ipc::{
    CORTEX_VERSION, Hello, Mismatch, Protocol, Request, RequestBody, Response, ResponseBody, Role,
    encode, read_frame,
};
use cortex_core::{
    Agent, Endpoint, Event, EventKind, HookKind, Identifier, RepoId, RepoPath, RunFiles, ToolKind,
    ToolTarget, Ulid, UserFolder, canonical_root,
};
use cortex_hook::ask;
use cortex_store::Reader;
use interprocess::local_socket::{
    ConnectOptions, GenericFilePath, GenericNamespaced, Stream, ToFsName, ToNsName,
};

/// How long a stopped daemon may take to exit: its runtime gives open connections one second.
const EXIT_WITHIN: Duration = Duration::from_secs(30);

/// A repository holding one Python file, and an empty home folder.
fn repository() -> TestResult<(tempfile::TempDir, tempfile::TempDir)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def kept():\n    pass\n")?;
    Ok((repo, home))
}

/// The run files of the daemon of the repository at `root` for the user whose home is `home`.
fn run_files(root: &Path, home: &Path) -> TestResult<RunFiles> {
    let user = UserFolder::in_home(home);
    Ok(user.run_files(&RepoId::of(&canonical_root(root)?)))
}

/// A daemon on `repo`, ready.
fn ready_daemon(repo: &Path, home: &Path) -> TestResult<Daemon> {
    let daemon = Daemon::start(repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    Ok(daemon)
}

/// Everything `cortex status` prints for `repo`.
fn status_output(repo: &Path, home: &Path) -> TestResult<String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["status", "--root"]).arg(repo);
    isolate(&mut command, home);
    Ok(String::from_utf8(command.output()?.stdout)?)
}

/// The first line `cortex status` prints for `repo`.
fn status_line(repo: &Path, home: &Path) -> TestResult<String> {
    let stdout = status_output(repo, home)?;
    Ok(stdout.lines().next().unwrap_or_default().to_owned())
}

/// Waits for `daemon` to exit, at most [`EXIT_WITHIN`], and returns its exit code.
fn exit_code(daemon: &mut Daemon) -> TestResult<Option<i32>> {
    let start = Instant::now();
    loop {
        if let Some(status) = daemon.child.try_wait()? {
            return Ok(status.code());
        }
        if start.elapsed() > EXIT_WITHIN {
            return Err("the daemon did not exit in time".into());
        }
        std::thread::sleep(POLL_EVERY);
    }
}

/// A raw connection to the daemon's endpoint.
fn connect(files: &RunFiles) -> TestResult<Stream> {
    let name = match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?,
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?,
    };
    Ok(ConnectOptions::new().name(name).connect_sync()?)
}

/// `cortex status` names the running daemon's process and version; a Shutdown is acknowledged,
/// the daemon exits with 0 and leaves no process-id file, and status then finds no daemon.
#[test]
fn status_names_the_daemon_and_shutdown_stops_it() -> TestResult {
    let (repo, home) = repository()?;
    let mut daemon = ready_daemon(repo.path(), home.path())?;
    let line = status_line(repo.path(), home.path())?;
    let expected = format!(
        "daemon: running (process {}, Cortex {CORTEX_VERSION}, up ",
        daemon.child.id()
    );
    assert!(
        line.starts_with(&expected) && line.ends_with(" s)"),
        "{line}"
    );
    let files = run_files(repo.path(), home.path())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let answer = ask(&files, Role::Cli, RequestBody::Shutdown, deadline)?;
    assert_eq!(answer.body, ResponseBody::Ack);
    assert_eq!(answer.welcome.pid, daemon.child.id());
    assert_eq!(exit_code(&mut daemon)?, Some(0));
    assert!(!files.pid_path().exists());
    if let Endpoint::Socket(path) = files.endpoint() {
        assert!(!path.exists(), "{}", path.display());
    }
    assert_eq!(
        status_line(repo.path(), home.path())?,
        "daemon: not running"
    );
    Ok(())
}

/// A client speaking the next protocol gets the daemon's protocol and version back, and its
/// Shutdown, which every version reads, still stops the daemon.
#[test]
fn another_protocol_gets_a_mismatch_and_may_stop_the_daemon() -> TestResult {
    let (repo, home) = repository()?;
    let mut daemon = ready_daemon(repo.path(), home.path())?;
    let mut stream = connect(&run_files(repo.path(), home.path())?)?;
    let hello = Request {
        request_id: 1,
        body: RequestBody::Hello(Hello {
            protocol: Protocol {
                major: Protocol::CURRENT.major + 1,
                minor: 0,
            },
            cortex_version: "99.0.0".to_owned(),
            role: Role::Hook,
        }),
    };
    let shutdown = Request {
        request_id: 2,
        body: RequestBody::Shutdown,
    };
    let mut frames = encode(&hello)?;
    frames.extend(encode(&shutdown)?);
    stream.write_all(&frames)?;
    let mismatch: Response = read_frame(&mut stream)?;
    assert_eq!(
        mismatch,
        Response {
            request_id: 1,
            body: ResponseBody::Mismatch(Mismatch {
                protocol: Protocol::CURRENT,
                cortex_version: CORTEX_VERSION.to_owned(),
            }),
        }
    );
    let acknowledged: Response = read_frame(&mut stream)?;
    assert_eq!(acknowledged.body, ResponseBody::Ack);
    assert_eq!(exit_code(&mut daemon)?, Some(0));
    Ok(())
}

/// A client that sends a frame that is not a request, a request before saying hello, or a second
/// hello is cut off, the last after being told why; the daemon answers the next client as before.
/// Each is one of the daemon's recent errors, in the daemon's words: the bytes the client sent are
/// not repeated.
#[test]
fn a_broken_client_changes_nothing() -> TestResult {
    let (repo, home) = repository()?;
    let daemon = ready_daemon(repo.path(), home.path())?;
    let files = run_files(repo.path(), home.path())?;
    let mut garbage = connect(&files)?;
    garbage.write_all(b"\x05\x00\x00\x00{{{{{")?;
    let mut skipped = connect(&files)?;
    skipped.write_all(&encode(&Request {
        request_id: 1,
        body: RequestBody::Status,
    })?)?;
    let mut repeated = connect(&files)?;
    let hello = |request_id| Request {
        request_id,
        body: RequestBody::Hello(Hello {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
            role: Role::Cli,
        }),
    };
    let mut twice = encode(&hello(1))?;
    twice.extend(encode(&hello(2))?);
    repeated.write_all(&twice)?;
    // The daemon keeps each error before the client can see the connection end or read why, so
    // all three are kept by now.
    assert!(read_frame::<Response>(&mut garbage).is_err());
    assert!(read_frame::<Response>(&mut skipped).is_err());
    let welcome: Response = read_frame(&mut repeated)?;
    assert!(
        matches!(welcome.body, ResponseBody::Welcome(_)),
        "{welcome:?}"
    );
    let refusal: Response = read_frame(&mut repeated)?;
    assert_eq!(
        refusal.body,
        ResponseBody::Error {
            message: "a connection says hello once".to_owned()
        }
    );
    let stdout = status_output(repo.path(), home.path())?;
    let line = stdout.lines().next().unwrap_or_default();
    assert!(
        line.starts_with(&format!("daemon: running (process {}", daemon.child.id())),
        "{line}"
    );
    let mut errors: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("error: "))
        .filter_map(|error| error.rsplit_once(" (").map(|(message, _)| message))
        .collect();
    errors.sort_unstable();
    assert_eq!(
        errors,
        [
            "a client asked before its handshake",
            "a client sent its handshake twice",
            "a request could not be read: a frame body is not the message expected"
        ],
        "{stdout}"
    );
    assert!(!stdout.contains("{{"), "{stdout}");
    Ok(())
}

/// Events sent to the daemon are recorded: a session start, a prompt that starts task 1 and a read
/// after it ran become the session, the task and three rows; the moment before the read is
/// answered and not kept. Status then reports nothing waiting and nothing lost, each hook's one
/// request in hook order (one time is every percentile), no error, and nothing of the prompt.
#[test]
fn events_sent_to_the_daemon_are_recorded() -> TestResult {
    let (repo, home) = repository()?;
    let _daemon = ready_daemon(repo.path(), home.path())?;
    let files = run_files(repo.path(), home.path())?;
    let root = canonical_root(repo.path())?;
    let event = |hook: HookKind, ts_ms: i64| -> TestResult<Event> {
        Ok(Event {
            event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"server"),
            ts_ms,
            agent: Agent::Claude,
            agent_version: None,
            session_id: Identifier::new("s1")?,
            parent_session_id: None,
            cwd: root.clone(),
            repo_root: root.clone(),
            hook,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        })
    };
    let read = |hook: HookKind, ts_ms: i64| -> TestResult<Event> {
        Ok(Event {
            tool: Some(ToolKind::Read),
            tool_target: Some(ToolTarget {
                path: Some(RepoPath::new("app.py")?),
                ..ToolTarget::default()
            }),
            ..event(hook, ts_ms)?
        })
    };
    let prompt = Event {
        prompt_text: Some("look at app".to_owned()),
        ..event(HookKind::Prompt, 20)?
    };
    for sent in [
        event(HookKind::SessionStart, 10)?,
        prompt,
        read(HookKind::PreTool, 30)?,
        read(HookKind::PostTool, 40)?,
    ] {
        let deadline = Instant::now() + Duration::from_secs(10);
        let body = RequestBody::Event {
            event: Box::new(sent),
        };
        assert_eq!(
            ask(&files, Role::Hook, body, deadline)?.body,
            ResponseBody::Ack
        );
    }
    let session = Identifier::new("s1")?;
    let rows = || -> TestResult<Vec<(EventKind, i64)>> {
        let reader = Reader::open(&repo.path().join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_of_session(&session)?
            .into_iter()
            .map(|row| (row.record.kind, row.record.ts_ms))
            .collect())
    };
    common::wait_for(|| Ok(rows()?.len() >= 3))?;
    assert_eq!(
        rows()?,
        [
            (EventKind::SessionStart, 10),
            (EventKind::Prompt, 20),
            (EventKind::Read, 40)
        ]
    );
    let stdout = status_output(repo.path(), home.path())?;
    assert!(
        stdout.contains(
            "\nqueue: 0 events and 0 changes waiting; 0 events dropped, 0 batches not recorded, \
             0 checkpoints failed\n"
        ),
        "{stdout}"
    );
    let latencies: Vec<(&str, &str)> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("daemon latency: "))
        .filter_map(|line| line.split_once(" p50 "))
        .collect();
    let hooks: Vec<&str> = latencies.iter().map(|(hook, _)| *hook).collect();
    assert_eq!(hooks, ["session_start", "prompt", "pre_tool", "post_tool"]);
    for (hook, times) in latencies {
        let (time, _) = times.split_once(", ").unwrap_or_default();
        assert_eq!(
            times,
            format!("{time}, p95 {time}, p99 {time} (last 1 request)"),
            "{hook}"
        );
    }
    assert!(stdout.contains("\nerrors: none\n"), "{stdout}");
    assert!(!stdout.contains("look at app"), "{stdout}");
    Ok(())
}

/// Probes are answered, counted and leave no row: a probe session start and prompt from session
/// `probe` get their acknowledgements and the daemon's status counts two probes, and once a real
/// session start is stored after them, the store holds that session's row alone, and no session
/// `probe`.
#[test]
fn probes_leave_no_rows() -> TestResult {
    let (repo, home) = repository()?;
    let _daemon = ready_daemon(repo.path(), home.path())?;
    let files = run_files(repo.path(), home.path())?;
    let root = canonical_root(repo.path())?;
    let event = |hook: HookKind, session: &str, is_probe: bool| -> TestResult<Event> {
        Ok(Event {
            event_id: Ulid::from_seed(1, session.as_bytes()),
            ts_ms: 10,
            agent: Agent::Claude,
            agent_version: None,
            session_id: Identifier::new(session)?,
            parent_session_id: None,
            cwd: root.clone(),
            repo_root: root.clone(),
            hook,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe,
        })
    };
    for sent in [
        event(HookKind::SessionStart, "probe", true)?,
        Event {
            event_id: Ulid::from_seed(2, b"probe"),
            ..event(HookKind::Prompt, "probe", true)?
        },
        event(HookKind::SessionStart, "real", false)?,
    ] {
        let deadline = Instant::now() + Duration::from_secs(10);
        let body = RequestBody::Event {
            event: Box::new(sent),
        };
        assert_eq!(
            ask(&files, Role::Hook, body, deadline)?.body,
            ResponseBody::Ack
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let answer = ask(&files, Role::Cli, RequestBody::Status, deadline)?;
    let ResponseBody::Status(report) = answer.body else {
        return Err(format!("{:?}", answer.body).into());
    };
    assert_eq!(report.probes_answered, 2);
    let stored = || -> TestResult<Vec<(String, EventKind)>> {
        let reader = Reader::open(&repo.path().join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_after(0, 10)?
            .into_iter()
            .map(|row| {
                let session = row.record.session_id.map(|id| id.as_str().to_owned());
                (session.unwrap_or_default(), row.record.kind)
            })
            .collect())
    };
    common::wait_for(|| Ok(!stored()?.is_empty()))?;
    assert_eq!(stored()?, [("real".to_owned(), EventKind::SessionStart)]);
    let reader = Reader::open(&repo.path().join(".cortex"), 0)?;
    assert_eq!(reader.queries().session(&Identifier::new("probe")?)?, None);
    Ok(())
}

/// On Unix the socket is the user's alone (mode 0600) in a run folder that is too (0700).
#[cfg(unix)]
#[test]
fn the_endpoint_is_the_user_s_alone() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let (repo, home) = repository()?;
    let _daemon = ready_daemon(repo.path(), home.path())?;
    let files = run_files(repo.path(), home.path())?;
    let mode = |path: &Path| -> TestResult<u32> {
        Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
    };
    assert_eq!(mode(&files.socket_path())?, 0o600);
    assert_eq!(mode(&UserFolder::in_home(home.path()).run_folder())?, 0o700);
    Ok(())
}

/// On Windows the pipe's access list names one account, the current user, as PowerShell reads it
/// through a connected client.
#[cfg(windows)]
#[test]
fn the_endpoint_is_the_user_s_alone() -> TestResult {
    let (repo, home) = repository()?;
    let _daemon = ready_daemon(repo.path(), home.path())?;
    let files = run_files(repo.path(), home.path())?;
    let script = format!(
        "$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', '{}', \
         [System.IO.Pipes.PipeDirection]::InOut); $c.Connect(5000); \
         $c.GetAccessControl().Access | ForEach-Object {{ $_.IdentityReference.Value + '|' + \
         $_.AccessControlType }}; $c.Dispose()",
        files.pipe_name()
    );
    let listed = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()?;
    assert!(listed.status.success(), "{listed:?}");
    let whoami = Command::new("whoami").output()?;
    let account = String::from_utf8(whoami.stdout)?.trim().to_lowercase();
    let rules: Vec<String> = String::from_utf8(listed.stdout)?
        .lines()
        .map(|line| line.trim().to_lowercase())
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(rules, [format!("{account}|allow")]);
    Ok(())
}
