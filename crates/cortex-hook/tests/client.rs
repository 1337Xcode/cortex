//! The client against a scripted daemon on a real socket or named pipe: an answer, a mismatch,
//! silence until the deadline, a closed connection, broken frames, and no daemon at all.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cortex_core::ipc::{
    CORTEX_VERSION, Hello, Mismatch, Protocol, Request, RequestBody, Response, ResponseBody, Role,
    Welcome, read_frame, write_frame,
};
use cortex_core::{Endpoint, RepoId, RunFiles, UserFolder};
use cortex_hook::{ClientError, ask};
use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, ToFsName, ToNsName,
};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// What the scripted daemon's thread returns: the two requests it read, or why it failed.
type Served = Result<Vec<Request>, String>;

/// What the scripted daemon does once a client connects.
enum Script {
    /// Reads the two requests and writes these responses.
    Answer(Vec<Response>),
    /// Reads the two requests, then waits for the test to let it go, answering nothing.
    Silent(mpsc::Receiver<()>),
    /// Closes the connection at once.
    Close,
    /// Reads the two requests and writes these bytes.
    Raw(Vec<u8>),
}

/// The run files of a repository of its own under a temporary home, the run folder created.
fn run_files(home: &Path) -> TestResult<RunFiles> {
    let user = UserFolder::in_home(home);
    std::fs::create_dir_all(user.run_folder())?;
    Ok(user.run_files(&RepoId::of(home)))
}

/// A listener at the endpoint of `files`.
fn listen(files: &RunFiles) -> TestResult<Listener> {
    let name = match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?,
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?,
    };
    Ok(ListenerOptions::new().name(name).create_sync()?)
}

/// Serves one connection at the endpoint of `files` by `script`, on a thread.
fn serve(files: &RunFiles, script: Script) -> TestResult<JoinHandle<Served>> {
    let listener = listen(files)?;
    Ok(thread::spawn(move || -> Served {
        let mut stream = listener.accept().map_err(|error| error.to_string())?;
        let mut read = || -> Served {
            let hello = read_frame(&mut stream).map_err(|error| error.to_string())?;
            let request = read_frame(&mut stream).map_err(|error| error.to_string())?;
            Ok(vec![hello, request])
        };
        match script {
            Script::Close => Ok(Vec::new()),
            Script::Answer(responses) => {
                let requests = read()?;
                for response in responses {
                    write_frame(&mut stream, &response).map_err(|error| error.to_string())?;
                }
                Ok(requests)
            }
            Script::Silent(release) => {
                let requests = read()?;
                // An error means the test has already given up waiting: stop either way.
                let _ = release.recv();
                Ok(requests)
            }
            Script::Raw(bytes) => {
                let requests = read()?;
                stream
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?;
                Ok(requests)
            }
        }
    }))
}

/// The daemon's side of a handshake.
fn welcome() -> Welcome {
    Welcome {
        protocol: Protocol::CURRENT,
        cortex_version: "2.0.0".to_owned(),
        pid: 77,
    }
}

/// A response to request `request_id`.
fn response(request_id: u64, body: ResponseBody) -> Response {
    Response { request_id, body }
}

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
