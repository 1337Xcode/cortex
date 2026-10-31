//! The daemon's endpoint on temporary repositories with an empty home folder: `cortex status` asks
//! it, a Shutdown stops it, a client of another protocol gets a mismatch and may still stop it, a
//! broken client changes nothing, and the endpoint is the user's alone.

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
use cortex_core::{Endpoint, RepoId, RunFiles, UserFolder, canonical_root};
use cortex_hook::ask;
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

/// The first line `cortex status` prints for `repo`.
fn status_line(repo: &Path, home: &Path) -> TestResult<String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["status", "--root"]).arg(repo);
    isolate(&mut command, home);
    let output = command.output()?;
    let stdout = String::from_utf8(output.stdout)?;
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

/// A client that sends a frame that is not a request, or a request before saying hello, is cut
/// off; the daemon answers the next client as before.
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
    assert!(read_frame::<Response>(&mut skipped).is_err());
    let line = status_line(repo.path(), home.path())?;
    assert!(
        line.starts_with(&format!("daemon: running (process {}", daemon.child.id())),
        "{line}"
    );
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
