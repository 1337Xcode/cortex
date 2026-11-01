//! The daemon's log on temporary repositories with an empty home folder: what a daemon that ran and
//! was stopped wrote, with nothing of the prompt it was sent, and why a daemon that could not run
//! stopped.

mod common;

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{Daemon, POLL_EVERY, TestResult, daemon_command};
use cortex_core::ipc::{CORTEX_VERSION, RequestBody, Response, ResponseBody, Role, read_frame};
use cortex_core::{
    Agent, Endpoint, Event, HookKind, Identifier, RepoId, Ulid, UserFolder, canonical_root,
};
use cortex_hook::ask;
use interprocess::local_socket::{
    ConnectOptions, GenericFilePath, GenericNamespaced, ToFsName, ToNsName,
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

/// The lines of the log of the daemon of `repo` for the user whose home is `home`, each without
/// its time, which is checked to be a number of milliseconds.
fn log_lines(repo: &Path, home: &Path) -> TestResult<Vec<String>> {
    let id = RepoId::of(&canonical_root(repo)?);
    let path = UserFolder::in_home(home)
        .logs_folder()
        .join(format!("daemon-{id}.log"));
    let mut lines = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let (time, text) = line.split_once(' ').ok_or("a line without a time")?;
        time.parse::<i64>()?;
        lines.push(text.to_owned());
    }
    Ok(lines)
}

/// An event of `hook` from session `s1` in `root`, its id made from `ts_ms`, with `prompt` as its
/// prompt's text.
fn event(root: &Path, (hook, ts_ms): (HookKind, i64), prompt: Option<&str>) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"log"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: prompt.map(str::to_owned),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
    })
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

/// A daemon sent a session start, a prompt and a frame that is not a request, then stopped, logged
/// that it started (its version and process), its first index (one file written, tiny mode),
/// each hook it handled with the request's id, the broken connection in its own words, that it was
/// asked to stop, and last that it stopped. No line holds a word of the prompt or the client's
/// bytes.
#[test]
fn a_daemon_logs_its_life_and_nothing_of_a_prompt() -> TestResult {
    let (repo, home) = repository()?;
    let mut daemon = Daemon::start(repo.path(), home.path())?;
    let ready = daemon.first_line()?;
    assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
    let root = canonical_root(repo.path())?;
    let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&root));
    let deadline = || Instant::now() + Duration::from_secs(10);
    let prompt = "tell me about the pancakes";
    for sent in [
        event(&root, (HookKind::SessionStart, 10), None)?,
        event(&root, (HookKind::Prompt, 20), Some(prompt))?,
    ] {
        let body = RequestBody::Event {
            event: Box::new(sent),
        };
        assert_eq!(
            ask(&files, Role::Hook, body, deadline())?.body,
            ResponseBody::Ack
        );
    }
    let name = match files.endpoint() {
        Endpoint::Socket(path) => path.to_fs_name::<GenericFilePath>()?,
        Endpoint::Pipe(pipe) => pipe.to_ns_name::<GenericNamespaced>()?,
    };
    let mut garbage = ConnectOptions::new().name(name).connect_sync()?;
    garbage.write_all(b"\x05\x00\x00\x00{{{{{")?;
    assert!(read_frame::<Response>(&mut garbage).is_err());
    let stop = ask(&files, Role::Cli, RequestBody::Shutdown, deadline())?;
    assert_eq!(stop.body, ResponseBody::Ack);
    assert_eq!(exit_code(&mut daemon)?, Some(0));
    let lines = log_lines(repo.path(), home.path())?;
    let started = format!(
        "INFO cortex_daemon: daemon started version={CORTEX_VERSION} pid={} root=",
        daemon.child.id()
    );
    assert!(
        lines.first().is_some_and(|line| line.starts_with(&started)),
        "{lines:#?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("INFO cortex_daemon: daemon stopped"),
        "{lines:#?}"
    );
    for expected in [
        "INFO cortex_daemon: first index done files_written=1 files_unchanged=0 mode=tiny \
         files_waiting=0",
        "WARN cortex_daemon::server: a connection broke error=a request could not be read: a \
         frame body is not the message expected",
        "INFO cortex_daemon::host: stopping: asked to",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected}\n{lines:#?}"
        );
    }
    for (request_id, hook) in [(2, "session_start"), (2, "prompt")] {
        let handled = format!(
            "INFO cortex_daemon::server: hook handled request_id={request_id} hook={hook} \
             latency_us="
        );
        let count = lines
            .iter()
            .filter(|line| line.starts_with(&handled))
            .count();
        assert_eq!(count, 1, "{handled}\n{lines:#?}");
    }
    let text = lines.join("\n");
    assert!(!text.contains("pancakes") && !text.contains("{{"), "{text}");
    Ok(())
}

/// A daemon whose store cannot be opened (the database's name is taken by a folder) stops with an
/// error, and its log says so last, naming the store.
#[test]
fn a_daemon_that_cannot_run_logs_why() -> TestResult {
    let (repo, home) = repository()?;
    std::fs::create_dir_all(repo.path().join(".cortex").join("cortex.db"))?;
    let output = daemon_command(repo.path(), home.path()).output()?;
    assert!(!output.status.success());
    let lines = log_lines(repo.path(), home.path())?;
    let last = lines.last().ok_or("an empty log")?;
    assert!(
        last.starts_with(
            "ERROR cortex_daemon: daemon stopped on an error error=opening the store in "
        ),
        "{lines:#?}"
    );
    Ok(())
}
