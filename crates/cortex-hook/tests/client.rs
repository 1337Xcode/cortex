//! The client against a scripted daemon on a real socket or named pipe: an answer, a mismatch,
//! silence until the deadline, a closed connection, broken frames, and no daemon at all.

mod common;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{Script, TestResult, response, run_files, serve, welcome};
use cortex_core::ipc::{
    CORTEX_VERSION, Hello, Mismatch, Protocol, Request, RequestBody, ResponseBody, Role,
};
use cortex_core::{Agent, Event, HookKind, Identifier, Ulid};
use cortex_hook::{ClientError, ask};
#[cfg(windows)]
use {
    cortex_core::{Endpoint, RunFiles},
    interprocess::ConnectWaitMode,
    interprocess::local_socket::traits::Listener as _,
    interprocess::local_socket::{ConnectOptions, GenericNamespaced, Stream, ToNsName},
    std::io::{Read, Write},
    std::thread::JoinHandle,
};

/// A deadline `millis` from now.
fn in_millis(millis: u64) -> Instant {
    Instant::now() + Duration::from_millis(millis)
}

/// The client sends the handshake, then the request, and returns the daemon's handshake and its
/// answer.
#[test]
fn an_answer_comes_back_with_the_handshake() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let daemon = serve(
        &files,
        Script::Answer(vec![
            response(1, ResponseBody::Welcome(welcome())),
            response(2, ResponseBody::Ack),
        ]),
    )?;
    let answer = ask(&files, Role::Cli, RequestBody::Status, in_millis(5_000))?;
    assert_eq!(answer.welcome, welcome());
    assert_eq!(answer.body, ResponseBody::Ack);
    let requests = daemon.join().map_err(|_| "the daemon thread panicked")??;
    assert_eq!(
        requests,
        [
            Request {
                request_id: 1,
                body: RequestBody::Hello(Hello {
                    protocol: Protocol::CURRENT,
                    cortex_version: CORTEX_VERSION.to_owned(),
                    role: Role::Cli,
                }),
            },
            Request {
                request_id: 2,
                body: RequestBody::Status,
            },
        ]
    );
    Ok(())
}

/// A daemon that speaks another protocol answers the handshake with its versions, which the
/// client reports.
#[test]
fn a_mismatch_is_reported_with_the_daemon_s_versions() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let theirs = Mismatch {
        protocol: Protocol {
            major: Protocol::CURRENT.major + 1,
            minor: 0,
        },
        cortex_version: "9.0.0".to_owned(),
    };
    let daemon = serve(
        &files,
        Script::Answer(vec![response(1, ResponseBody::Mismatch(theirs.clone()))]),
    )?;
    let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(5_000));
    assert!(
        matches!(&result, Err(ClientError::Mismatch(found)) if *found == theirs),
        "{result:?}"
    );
    drop(daemon.join());
    Ok(())
}

/// A daemon that reads and never answers costs the client its deadline and no more.
#[test]
fn silence_ends_at_the_deadline() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let (release, released) = mpsc::channel();
    let daemon = serve(&files, Script::Silent(released))?;
    let start = Instant::now();
    let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(300));
    let waited = start.elapsed();
    assert!(matches!(result, Err(ClientError::TimedOut)), "{result:?}");
    assert!(
        waited >= Duration::from_millis(290) && waited < Duration::from_millis(1_000),
        "{waited:?}"
    );
    release.send(())?;
    drop(daemon.join());
    Ok(())
}

/// With no daemon at the endpoint, the client says so at once; a deadline already past gives up
/// before trying.
#[test]
fn no_daemon_is_not_running() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let start = Instant::now();
    let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(2_000));
    assert!(matches!(result, Err(ClientError::NotRunning)), "{result:?}");
    assert!(start.elapsed() < Duration::from_millis(1_000));
    let past = ask(&files, Role::Hook, RequestBody::Status, Instant::now());
    assert!(matches!(past, Err(ClientError::TimedOut)), "{past:?}");
    Ok(())
}

/// A connection closed at once is a failed exchange, not an answer.
#[test]
fn a_closed_connection_is_an_error() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let daemon = serve(&files, Script::Close)?;
    let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(5_000));
    assert!(matches!(result, Err(ClientError::Io(_))), "{result:?}");
    drop(daemon.join());
    Ok(())
}

/// Answers that break the protocol are reported as such: a frame that is not JSON, a first
/// answer that is not the handshake's, and an answer naming another request.
#[test]
fn broken_answers_break_the_protocol() -> TestResult {
    let scripts = [
        Script::Raw(b"\x05\x00\x00\x00{{{{{".to_vec()),
        Script::Answer(vec![response(1, ResponseBody::Ack)]),
        Script::Answer(vec![
            response(1, ResponseBody::Welcome(welcome())),
            response(7, ResponseBody::Ack),
        ]),
    ];
    for script in scripts {
        let home = tempfile::tempdir()?;
        let files = run_files(home.path())?;
        let daemon = serve(&files, script)?;
        let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(5_000));
        assert!(
            matches!(result, Err(ClientError::Protocol(_))),
            "{result:?}"
        );
        drop(daemon.join());
    }
    Ok(())
}

/// A request longer than a frame may be is refused before the endpoint is reached: with no daemon
/// listening, the answer is that the request is too long, not that no daemon runs.
#[test]
fn a_request_too_long_is_never_sent() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    let event = Event {
        event_id: Ulid::from_seed(1, b"long"),
        ts_ms: 1,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        hook: HookKind::Prompt,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: Some("x".repeat(1_048_576)),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    };
    let body = RequestBody::Event {
        event: Box::new(event),
    };
    let result = ask(&files, Role::Hook, body, in_millis(5_000));
    assert!(matches!(result, Err(ClientError::TooLarge)), "{result:?}");
    Ok(())
}

/// On Windows, a pipe of the daemon's name held by a process the run folder does not name as the
/// daemon (another user could create one while no daemon runs) is told nothing: the client hangs
/// up before writing a byte.
///
/// The impostor here hears every connection to its pipe until the test's own last one, which
/// says so and waits to be heard. Waiting for the client's connection alone would hang whenever the
/// client hung up before the impostor's thread began to listen: Windows then drops that connection
/// unseen and the listener waits for another, which never comes.
#[cfg(windows)]
#[test]
fn a_pipe_held_by_another_process_is_told_nothing() -> TestResult {
    let home = tempfile::tempdir()?;
    let files = run_files(home.path())?;
    std::fs::write(
        files.pid_path(),
        "4294967295
",
    )?;
    let impostor = impostor(&files)?;
    let result = ask(&files, Role::Hook, RequestBody::Status, in_millis(5_000));
    assert!(
        matches!(result, Err(ClientError::NotTheDaemon)),
        "{result:?}"
    );
    let mut last = connect_within(&files, Duration::from_secs(5))?;
    last.write_all(LAST_CONNECTION)?;
    let mut heard_back = [0_u8; 1];
    last.read_exact(&mut heard_back)?;
    let heard = impostor
        .join()
        .map_err(|_| "the impostor's thread panicked")??;
    assert!(
        heard.iter().all(Vec::is_empty),
        "the impostor read {heard:?}"
    );
    Ok(())
}

/// What the test's own last connection to the impostor writes.
#[cfg(windows)]
const LAST_CONNECTION: &[u8] = b"the test is done";

/// A pipe server at the endpoint of `files` that is not the daemon, on a thread: it reads each
/// connection until its client hangs up, and returns what each one sent once a connection sends
/// [`LAST_CONNECTION`], which it answers with one byte.
#[cfg(windows)]
fn impostor(files: &RunFiles) -> TestResult<JoinHandle<Result<Vec<Vec<u8>>, String>>> {
    let listener = common::listen(files)?;
    Ok(std::thread::spawn(move || {
        let mut heard = Vec::new();
        loop {
            let mut stream = listener.accept().map_err(|error| error.to_string())?;
            let bytes = read_until_hung_up(&mut stream, LAST_CONNECTION.len())?;
            if bytes == LAST_CONNECTION {
                stream.write_all(b"!").map_err(|error| error.to_string())?;
                return Ok(heard);
            }
            heard.push(bytes);
        }
    }))
}

/// What `stream` sends until its client hangs up, or until it has sent `enough` bytes.
#[cfg(windows)]
fn read_until_hung_up(stream: &mut impl Read, enough: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 64];
    while bytes.len() < enough {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => bytes.extend(chunk.iter().take(read)),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(bytes)
}

/// A connection to the endpoint of `files`, waiting at most `within` for the pipe to be free.
#[cfg(windows)]
fn connect_within(files: &RunFiles, within: Duration) -> TestResult<Stream> {
    let Endpoint::Pipe(pipe) = files.endpoint() else {
        return Err("a named pipe".into());
    };
    Ok(ConnectOptions::new()
        .name(pipe.to_ns_name::<GenericNamespaced>()?)
        .wait_mode(ConnectWaitMode::Timeout(within))
        .connect_sync()?)
}
