//! A fake daemon for the tests that need one: it listens where the repository's daemon would,
//! writes the process-id file as the real one does (naming the test's own process, which the
//! client checks on Windows), answers by a script, and records every request it reads.

use std::fs::File;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use cortex_core::ipc::{
    CORTEX_VERSION, GitReport, Mismatch, Protocol, Request, RequestBody, Response, ResponseBody,
    StatusReport, Welcome, read_frame, write_frame,
};
use cortex_core::{Endpoint, RepoId, RunFiles, UserFolder, canonical_root};
use interprocess::local_socket::traits::{Listener as _, Stream as _};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, Name, Stream, ToFsName, ToNsName,
};

use super::TestResult;

/// What a fake daemon does once a client connects.
pub enum Script {
    /// Reads the handshake and the request of one connection, then answers nothing until told.
    Silent(mpsc::Receiver<()>),
    /// Answers each handshake with a mismatch, as a daemon of `version` speaking another protocol,
    /// then reads the one request any version reads; stops after a stop.
    Mismatch(&'static str),
    /// Welcomes each connection as a daemon of this build whose first index is not done, answers a
    /// status saying so and a question with nothing, and stops after that many connections, as a
    /// daemon that dies while it indexes.
    Indexing(usize),
    /// Welcomes each connection as a daemon speaking this protocol, of the next of `versions` (the
    /// last for every connection past them), and takes every request; stops after a stop. Several
    /// versions stand for a daemon replaced between two connections.
    Welcome(&'static [&'static str]),
}

/// A fake daemon serving on a thread of its own. Unless silent, it holds the repository's lock as
/// a daemon does, and lets go of its endpoint and then the lock when it stops.
pub struct FakeDaemon {
    /// Where it listens.
    files: RunFiles,
    /// The thread, which returns the requests it read.
    thread: thread::JoinHandle<Result<Vec<Request>, String>>,
}

impl FakeDaemon {
    /// A fake daemon at the endpoint of the repository at `root` for the user whose home is
    /// `home`, its run folder and process-id file made as the real one makes them, serving by
    /// `script`.
    pub fn start(root: &Path, home: &Path, script: Script) -> TestResult<Self> {
        // Each step names itself in its error, so a failure on one platform says which it was.
        let step = |what: &'static str| move |error: std::io::Error| format!("{what}: {error}");
        let user = UserFolder::in_home(home);
        std::fs::create_dir_all(user.run_folder()).map_err(step("making the run folder"))?;
        let files = user.run_files(&RepoId::of(&canonical_root(root)?));
        std::fs::write(files.pid_path(), format!("{}\n", std::process::id()))
            .map_err(step("writing the process-id file"))?;
        let lock = if matches!(script, Script::Silent(_)) {
            None
        } else {
            let file = File::create(files.lock_path()).map_err(step("making the lock file"))?;
            file.try_lock()
                .map_err(|error| format!("taking the lock: {error}"))?;
            Some(file)
        };
        let listener = listen(&files)?;
        let thread = thread::spawn(move || {
            let served = serve(&listener, script);
            drop(listener);
            drop(lock);
            served
        });
        Ok(Self { files, thread })
    }

    /// The requests the fake daemon read. While it still waits for a client, a connection of the
    /// test's own, which sends nothing, ends its wait, so a client that never came fails the test
    /// instead of hanging it.
    pub fn requests(self) -> TestResult<Vec<Request>> {
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

/// How long a fake daemon waits for its endpoint's name to be free.
const ENDPOINT_FREED_WITHIN: Duration = Duration::from_secs(1);

/// Listens at the endpoint of `files`. On Windows a pipe's name stays taken for a few
/// milliseconds after its last handle is closed (10 ms seen on a loaded machine), and a listener
/// that must be the name's first instance is refused ("Access is denied") meanwhile; the fake
/// daemon before this one, under the same name, may have just let go of it. So a refusal is
/// tried again until the name is free, for at most [`ENDPOINT_FREED_WITHIN`].
fn listen(files: &RunFiles) -> TestResult<Listener> {
    let until = Instant::now() + ENDPOINT_FREED_WITHIN;
    loop {
        let name = endpoint_name(files).map_err(|error| format!("naming the endpoint: {error}"))?;
        match ListenerOptions::new().name(name).create_sync() {
            Ok(listener) => return Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && Instant::now() < until =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("listening at the endpoint: {error}").into()),
        }
    }
}

/// The name of the endpoint of `files`, as the socket library takes it.
fn endpoint_name(files: &RunFiles) -> std::io::Result<Name<'static>> {
    Ok(match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?.into_owned(),
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?.into_owned(),
    })
}

/// Serves `listener` by `script` and returns the requests read: one connection when silent, else
/// connections until one asks it to stop or the test's own connection ends the wait.
fn serve(listener: &Listener, script: Script) -> Result<Vec<Request>, String> {
    let text = |error: &dyn std::fmt::Display| error.to_string();
    let mut requests = Vec::new();
    for connection in 0.. {
        let mut stream = listener.accept().map_err(|error| text(&error))?;
        // The test's own connection sends nothing.
        let Ok(hello) = read_frame::<Request>(&mut stream) else {
            return Ok(requests);
        };
        let request_id = hello.request_id;
        requests.push(hello);
        let version = match &script {
            Script::Silent(release) => {
                let request: Request = read_frame(&mut stream).map_err(|error| text(&error))?;
                requests.push(request);
                // Released once the client has given up; one that never does meets the timeout
                // instead, and the test fails on its time rather than hanging.
                let _ = release.recv_timeout(Duration::from_secs(30));
                return Ok(requests);
            }
            Script::Mismatch(version) => {
                let body = ResponseBody::Mismatch(Mismatch {
                    protocol: Protocol {
                        major: Protocol::CURRENT.major + 1,
                        minor: 0,
                    },
                    cortex_version: (*version).to_owned(),
                });
                write_frame(&mut stream, &Response { request_id, body })
                    .map_err(|error| text(&error))?;
                if let Ok(next) = read_frame::<Request>(&mut stream) {
                    let is_stop = next.body == RequestBody::Shutdown;
                    let request_id = next.request_id;
                    requests.push(next);
                    if is_stop {
                        // As a daemon does, the stop is acknowledged; it holds whether or not the
                        // client stays for the acknowledgement.
                        let ack = Response {
                            request_id,
                            body: ResponseBody::Ack,
                        };
                        drop(write_frame(&mut stream, &ack));
                        return Ok(requests);
                    }
                }
                continue;
            }
            Script::Welcome(versions) => versions
                .get(connection)
                .or(versions.last())
                .ok_or("no version to welcome with")?,
            Script::Indexing(connections) => {
                if connection >= *connections {
                    return Ok(requests);
                }
                CORTEX_VERSION
            }
        };
        let body = ResponseBody::Welcome(Welcome {
            protocol: Protocol::CURRENT,
            cortex_version: (*version).to_owned(),
            pid: std::process::id(),
        });
        write_frame(&mut stream, &Response { request_id, body }).map_err(|error| text(&error))?;
        while let Ok(request) = read_frame::<Request>(&mut stream) {
            let is_stop = request.body == RequestBody::Shutdown;
            let body = match (&script, &request.body) {
                (Script::Indexing(_), RequestBody::Status) => ResponseBody::Status(indexing()),
                (Script::Indexing(_), RequestBody::Where { .. }) => {
                    ResponseBody::Reply { text: None }
                }
                _ => ResponseBody::Ack,
            };
            let answer = Response {
                request_id: request.request_id,
                body,
            };
            requests.push(request);
            write_frame(&mut stream, &answer).map_err(|error| text(&error))?;
            if is_stop {
                return Ok(requests);
            }
        }
    }
    Ok(requests)
}

/// The status of a daemon of this build, this process, whose first index is not done.
fn indexing() -> StatusReport {
    StatusReport {
        cortex_version: CORTEX_VERSION.to_owned(),
        protocol: Protocol::CURRENT,
        pid: std::process::id(),
        uptime_ms: 0,
        events_waiting: 0,
        changes_waiting: 0,
        events_dropped: 0,
        failed_batches: 0,
        failed_checkpoints: 0,
        probes_answered: 0,
        is_indexed: false,
        latencies: Vec::new(),
        recent_errors: Vec::new(),
        git: Some(GitReport::default()),
        graph: None,
        worktree: None,
    }
}

/// The kind of each request, for comparing: `hello`, `event`, `retrieve`, `primer`, `where`,
/// `status` or `shutdown`.
pub fn kinds(requests: &[Request]) -> Vec<&'static str> {
    requests
        .iter()
        .map(|request| match request.body {
            RequestBody::Hello(_) => "hello",
            RequestBody::Event { .. } => "event",
            RequestBody::Retrieve { .. } => "retrieve",
            RequestBody::Primer { .. } => "primer",
            RequestBody::Where { .. } => "where",
            RequestBody::Status => "status",
            RequestBody::Shutdown => "shutdown",
        })
        .collect()
}
