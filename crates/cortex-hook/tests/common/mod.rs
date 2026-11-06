//! The scripted daemon the hook's tests talk to, on a real socket or named pipe: it reads a
//! client's handshake and request and answers as its script says.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

pub mod fixture;

use cortex_core::ipc::{
    Protocol, Request, Response, ResponseBody, Welcome, read_frame, write_frame,
};
use cortex_core::{Endpoint, RepoId, RunFiles, UserFolder};
use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, ToFsName, ToNsName,
};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// What the scripted daemon's thread returns: the two requests it read, or why it failed.
pub type Served = Result<Vec<Request>, String>;

/// What the scripted daemon does once a client connects.
pub enum Script {
    /// Reads the two requests and writes these responses.
    Answer(Vec<Response>),
    /// Reads the two requests, then waits for the test to let it go, answering nothing.
    Silent(mpsc::Receiver<()>),
    /// Closes the connection at once.
    Close,
    /// Reads the two requests and writes these bytes.
    Raw(Vec<u8>),
}

/// The run files of a repository of its own under a temporary home, the run folder created and
/// its process-id file naming this process, which plays the daemon.
pub fn run_files(home: &Path) -> TestResult<RunFiles> {
    run_files_of(home, home)
}

/// The run files of the repository whose canonical root is `root`, under the home folder `home`,
/// as [`run_files`] makes them.
pub fn run_files_of(home: &Path, root: &Path) -> TestResult<RunFiles> {
    let user = UserFolder::in_home(home);
    std::fs::create_dir_all(user.run_folder())?;
    let files = user.run_files(&RepoId::of(root));
    std::fs::write(files.pid_path(), format!("{}\n", std::process::id()))?;
    Ok(files)
}

/// A listener at the endpoint of `files`.
pub fn listen(files: &RunFiles) -> TestResult<Listener> {
    let name = match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?,
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?,
    };
    Ok(ListenerOptions::new().name(name).create_sync()?)
}

/// Serves one connection at the endpoint of `files` by `script`, on a thread.
pub fn serve(files: &RunFiles, script: Script) -> TestResult<JoinHandle<Served>> {
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
pub fn welcome() -> Welcome {
    Welcome {
        protocol: Protocol::CURRENT,
        cortex_version: "2.0.0".to_owned(),
        pid: 77,
    }
}

/// A response to request `request_id`.
pub fn response(request_id: u64, body: ResponseBody) -> Response {
    Response { request_id, body }
}
