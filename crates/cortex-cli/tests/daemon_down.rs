//! A hook that finds no daemon starts one in the background and returns at once, printing nothing:
//! every hook Cortex registers with Claude Code does, and the daemon one of them starts answers
//! within seconds and indexes the repository, a file written meanwhile included.

mod common;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{
    POLL_EVERY, READY_WITHIN, TestResult, isolate, payloads, printed, stop_late_daemon, wait_for,
};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    RepoId, RepoPath, RunFiles, UserFolder, canonical_root, create_private_folder,
    open_private_file,
};
use cortex_hook::{adapter, ask};
use cortex_store::Reader;

/// The Claude Code version whose payloads the hooks are sent: the newest Cortex is tested with.
const NEWEST: &str = "2.1.316";

/// A home folder with a repository in it, and the daemon a hook may start there: stopped, and the
/// folder removed, when the test ends however it ends, so that no process outlives the test.
struct Scene {
    /// The home folder, removed on drop once the daemon has let go of it.
    home: Option<tempfile::TempDir>,
    /// Where the repository's daemon listens.
    files: RunFiles,
    /// The repository's folder in the home folder.
    folder: &'static str,
}

impl Scene {
    /// A home folder holding a repository marked for Cortex with one Python file.
    fn new() -> TestResult<Self> {
        Self::in_folder("repo")
    }

    /// A home folder holding, in `folder`, a repository marked for Cortex with one Python file.
    fn in_folder(folder: &'static str) -> TestResult<Self> {
        let home = tempfile::tempdir()?;
        let repo = home.path().join(folder);
        std::fs::create_dir_all(repo.join(".cortex"))?;
        std::fs::write(repo.join("app.py"), "def kept():\n    pass\n")?;
        let files =
            UserFolder::in_home(home.path()).run_files(&RepoId::of(&canonical_root(&repo)?));
        Ok(Self {
            home: Some(home),
            files,
            folder,
        })
    }

    /// The home folder.
    fn home(&self) -> TestResult<&Path> {
        Ok(self.home.as_ref().ok_or("no home folder")?.path())
    }

    /// The repository, canonical.
    fn repo(&self) -> TestResult<PathBuf> {
        Ok(canonical_root(&self.home()?.join(self.folder))?)
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        // A daemon a hook started may still be starting when the test ends, early or not.
        stop_late_daemon(&self.files);
        // On Windows the files a daemon has open in the folder (its log, its database) keep it
        // from being removed until its process ends, a moment after its process-id file goes: the
        // folder is removed once they are released.
        if let Some(home) = self.home.take() {
            let path = home.keep();
            let _ = wait_for(|| Ok(std::fs::remove_dir_all(&path).is_ok()));
        }
    }
}

/// Runs `cortex hook --agent claude --event <event>` with `payload` on stdin and `home` as the home
/// folder, its output read through pipes as an agent reads it: whether it ended silently with exit
/// code 0, and how long it took. The pipes must end with the hook, however long the daemon it
/// started runs: an agent that waits for them would otherwise wait for the daemon.
fn hook(event: &str, payload: &[u8], home: &Path) -> TestResult<(bool, Duration)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["hook", "--agent", "claude", "--event", event]);
    isolate(&mut command, home);
    let started = Instant::now();
    let printed = printed(&mut command, payload)?;
    let took = started.elapsed();
    let is_silent = printed.is_success && printed.stdout.is_empty() && printed.stderr.is_empty();
    Ok((is_silent, took))
}

/// What the hook log at `log` holds: nothing when there is none.
fn logged(log: &Path) -> TestResult<String> {
    match std::fs::read_to_string(log) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// The captured payload each hook Cortex registers with Claude Code is sent: a session's start, a
/// prompt, the moment before a read, an edit's result, a failed command's, a compaction's start,
/// a subagent's start and end, the stop and the session's end.
fn fixture_of(event: &str) -> Option<&'static str> {
    Some(match event {
        "SessionStart" => "session-start-startup.json",
        "UserPromptSubmit" => "user-prompt-submit.json",
        "PreToolUse" => "pre-tool-use-read.json",
        "PostToolUse" => "post-tool-use-edit.json",
        "PostToolUseFailure" => "post-tool-use-failure-bash.json",
        "PreCompact" => "pre-compact-manual.json",
        "SubagentStart" => "subagent-start.json",
        "SubagentStop" => "subagent-stop.json",
        "Stop" => "stop.json",
        "SessionEnd" => "session-end.json",
        _ => return None,
    })
}

/// Takes the repository's run lock, as a daemon still starting holds it, so that each daemon a hook
/// starts finds the repository held and exits. Released when the file is dropped.
fn hold(files: &RunFiles, home: &Path) -> TestResult<File> {
    create_private_folder(&UserFolder::in_home(home).run_folder())?;
    let lock = open_private_file(&files.lock_path())?;
    lock.try_lock()?;
    Ok(lock)
}

/// The first start of a freshly built binary can take seconds (a virus scanner reads it), which a
/// timed call must not pay.
fn warm_up() -> TestResult {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("version")
        .stdout(Stdio::null())
        .status()?;
    Ok(())
}

/// Every hook Cortex registers with Claude Code, its captured payload sent from a repository whose
/// daemon is down and cannot come up (the test holds the repository, as a daemon still starting
/// does), tries the daemon, starts one and returns: exit code 0, nothing printed, nothing logged
/// (a hook held past its hard timeout would log that it ran out of time), and a start marked. The
/// bound on how long a call took is loose, since starting a process on a busy machine is slow: the
/// budgets are the benchmarks' to check. A call that took its hook's hard timeout or longer, which
/// a loaded machine can make of any call, may rightly have fallen open before the start and logged
/// it: it shows neither, so it is made again, at most three times, and the last call counts.
#[test]
fn every_hook_is_silent_with_the_daemon_down() -> TestResult {
    let scene = Scene::new()?;
    let (home, repo) = (scene.home()?, scene.repo()?);
    let held = hold(&scene.files, home)?;
    warm_up()?;
    let hooks = adapter("claude")
        .ok_or("no adapter for Claude Code")?
        .hooks();
    let log = UserFolder::in_home(home).logs_folder().join("hook.log");
    for entry in hooks {
        let event = entry.event_name;
        let fixture = fixture_of(event).ok_or(event)?;
        let payload = payloads::payload(NEWEST, fixture, &repo)?;
        for call in 1..=3 {
            if let Err(error) = std::fs::remove_file(scene.files.start_path())
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
            let logged_before = logged(&log)?.len();
            let (is_silent, took) = hook(event, &payload, home)?;
            assert!(is_silent, "{event}");
            if took >= entry.hard_timeout && call < 3 {
                continue;
            }
            assert!(took < Duration::from_secs(10), "{event}: {took:?}");
            let logged = logged(&log)?;
            assert_eq!(logged.get(logged_before..), Some(""), "{event}");
            assert!(scene.files.start_path().exists(), "{event} made no start");
            break;
        }
    }
    assert_eq!(hooks.len(), 10);
    drop(held);
    Ok(())
}

/// With no daemon running, a session's start through the hook returns silently and within its
/// hard timeout (1.5 s), so without a line in the hook log; a daemon then answers, and a file
/// written after the hook returned is indexed.
#[test]
fn a_hook_starts_the_daemon_and_does_not_wait() -> TestResult {
    let scene = Scene::new()?;
    let (home, repo) = (scene.home()?, scene.repo()?);
    let payload = serde_json::to_vec(&serde_json::json!({
        "session_id": "s1",
        "hook_event_name": "SessionStart",
        "source": "startup",
        "cwd": repo,
    }))?;
    warm_up()?;
    let (is_silent, took) = hook("SessionStart", &payload, home)?;
    assert!(is_silent);
    // A hook held past its hard timeout would log that it ran out of time; this one logged
    // nothing. How long the call took includes starting a process, which a busy machine slows: the
    // bound only catches a hook that waited for the daemon to be ready.
    let log = UserFolder::in_home(home).logs_folder().join("hook.log");
    assert!(!log.exists(), "{:?}", std::fs::read_to_string(&log));
    assert!(took < Duration::from_secs(10), "{took:?}");
    std::fs::write(repo.join("later.py"), "def later():\n    pass\n")?;

    let answers = || -> TestResult<bool> {
        let deadline = Instant::now() + Duration::from_secs(2);
        Ok(matches!(
            ask(&scene.files, Role::Cli, RequestBody::Status, deadline),
            Ok(answer) if matches!(answer.body, ResponseBody::Status(_))
        ))
    };
    let later = RepoPath::new("later.py")?;
    let is_indexed = || -> TestResult<bool> {
        let Ok(reader) = Reader::open(&repo.join(".cortex"), 0) else {
            return Ok(false);
        };
        Ok(reader.queries().file(&later)?.is_some())
    };
    // A debug build loads every grammar before its first index: the daemon gets the time the other
    // daemon tests give it.
    let since = Instant::now();
    while !(answers()? && is_indexed()?) {
        assert!(
            since.elapsed() < READY_WITHIN,
            "no daemon answered with the file indexed"
        );
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}

/// On Windows an agent that leaves its own handles inheritable passes them to its hooks, and a hook
/// to the daemon it starts: Codex does, and then waits at its exit for a pipe the daemon holds.
/// Here `cmd /C` runs the hook with the hook's own output sent to NUL, so the pipe the test reads
/// reaches the hook only as a handle it inherited from `cmd`, as Codex's own handles reach its
/// hooks. The pipe must end with `cmd` and the hook while the daemon they started answers. The
/// repository's folder holds a space, which the daemon's new start must quote.
#[cfg(windows)]
#[test]
fn a_daemon_holds_no_handle_an_agent_left_inheritable() -> TestResult {
    use std::os::windows::process::CommandExt;

    let scene = Scene::in_folder("a repo")?;
    let (home, repo) = (scene.home()?, scene.repo()?);
    let payload = serde_json::to_vec(&serde_json::json!({
        "session_id": "s1",
        "hook_event_name": "SessionStart",
        "source": "startup",
        "cwd": repo,
    }))?;
    warm_up()?;
    let mut command = Command::new("cmd");
    command.raw_arg(format!(
        r#"/D /S /C ""{}" hook --agent claude --event SessionStart >NUL 2>NUL""#,
        env!("CARGO_BIN_EXE_cortex")
    ));
    isolate(&mut command, home);
    let ran = printed(&mut command, &payload)?;
    assert!(ran.is_success, "{}{}", ran.stdout, ran.stderr);
    assert_eq!((ran.stdout.as_str(), ran.stderr.as_str()), ("", ""));
    let answers = || -> TestResult<bool> {
        let deadline = Instant::now() + Duration::from_secs(2);
        Ok(matches!(
            ask(&scene.files, Role::Cli, RequestBody::Status, deadline),
            Ok(answer) if matches!(answer.body, ResponseBody::Status(_))
        ))
    };
    let since = Instant::now();
    while !answers()? {
        assert!(since.elapsed() < READY_WITHIN, "no daemon answered");
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}
