//! What the adoption tests share: a home folder of a test's own with a temporary folder inside it,
//! as Windows keeps one, commands and hooks run there, and the captured calculator made a git
//! repository.
//!
//! The system's temporary folder, where the test's folders are, would hold every folder a test
//! makes, and nothing in it is ever set up: each command is given the test's own. No folder a hook
//! or an adoption is started from is outside the test's home folder unless a test means it to be:
//! everything there is inside the real home folder.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads read here are JSON objects, where indexing a missing key gives null"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use cortex_core::{RepoId, UserFolder, canonical_root};
use serde_json::Value;

use super::calc::{MAIN, OPS, TESTS};
use super::git::commit;
use super::hook_call::{Call, hook_with};
use super::{POLL_EVERY, Printed, TestResult, isolate, stand_in_claude, test_path};

/// What the user is told once, when Cortex set a repository up on its own.
pub const WELCOME: &str =
    "Cortex set up this repository for your agents; `cortex off` turns it off here.";

/// How long a start that sets nothing up may take: it reads a configuration and writes a file, or
/// takes back what it wrote, and serves nothing.
pub const NOTHING_SET_UP_WITHIN: Duration = Duration::from_secs(60);

/// A prompt in the area of `divide`: it names `divide` and the file it is in.
pub const FAMILIAR: &str = "Make divide in calc/ops.py round its result to two places.";

/// A home folder of a test's own holding its temporary folder, in a base folder removed when
/// dropped.
pub struct Machine {
    /// The base folder.
    pub base: tempfile::TempDir,
    /// The home folder, canonical.
    pub home: PathBuf,
    /// The temporary folder the commands are given, canonical, inside the home folder.
    pub temp: PathBuf,
}

impl Machine {
    /// A fresh base folder holding a home folder that holds an empty temporary folder.
    pub fn new() -> TestResult<Self> {
        let base = tempfile::tempdir()?;
        let home = canonical_root(base.path())?.join("home");
        let temp = home.join("AppData").join("Local").join("Temp");
        std::fs::create_dir_all(&temp)?;
        Ok(Self { base, home, temp })
    }

    /// The user's own Cortex folder.
    pub fn user(&self) -> UserFolder {
        UserFolder::in_home(&self.home)
    }

    /// The variables every command of the test gets: the temporary folder `temp` under each name
    /// a platform reads it by, and git looking for no repository above the base folder.
    pub fn variables<'a>(&'a self, temp: &'a Path) -> [(&'static str, &'a Path); 4] {
        [
            ("TMP", temp),
            ("TEMP", temp),
            ("TMPDIR", temp),
            ("GIT_CEILING_DIRECTORIES", self.base.path()),
        ]
    }

    /// Runs `cortex <args>` with the test's home and temporary folders, git and a stand-in for
    /// Claude Code 2.1.312 on the PATH, and reads what it printed.
    pub fn cortex(&self, args: &[&str]) -> TestResult<Printed> {
        self.cortex_with(args, &["git"])
    }

    /// Runs `cortex <args>` as [`Machine::cortex`] does, with only `programs` (and the stand-in
    /// for Claude Code) on the PATH.
    pub fn cortex_with(&self, args: &[&str], programs: &[&str]) -> TestResult<Printed> {
        if !super::init::claude_in(&self.home).exists() {
            stand_in_claude(&self.home, "2.1.312")?;
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args(args).envs(self.variables(&self.temp));
        super::init::run(&mut command, &self.home, programs)
    }

    /// Runs `cortex daemon --root <folder> --adopt`, as a hook starts it, but waited for, with git
    /// on the PATH, and checks that it ended, and ended well, without setting the folder up: a
    /// start that sets it up goes on to serve it, so one still running after
    /// [`NOTHING_SET_UP_WITHIN`] is stopped and fails the test, which never waits for a daemon's
    /// idle stop.
    pub fn adopt_setting_nothing_up(&self, folder: &Path) -> TestResult {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(["daemon", "--root"])
            .arg(folder)
            .arg("--adopt")
            .envs(self.variables(&self.temp))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate(&mut command, &self.home);
        command.env("PATH", test_path(&self.home, &["git"])?);
        let mut child = command.spawn()?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                assert!(status.success(), "{}: {status}", folder.display());
                return Ok(());
            }
            if start.elapsed() > NOTHING_SET_UP_WITHIN {
                drop(child.kill());
                drop(child.wait());
                return Err(format!("{} was set up and served", folder.display()).into());
            }
            std::thread::sleep(POLL_EVERY);
        }
    }

    /// Runs `cortex hook` on the Claude Code `payload` as Claude Code does.
    pub fn hook(&self, payload: &Value) -> TestResult<Call> {
        self.hook_of("claude", payload, &self.temp)
    }

    /// Runs `cortex hook --agent <agent>` on `payload`, the temporary folder being `temp`.
    pub fn hook_of(&self, agent: &str, payload: &Value, temp: &Path) -> TestResult<Call> {
        let event = payload["hook_event_name"]
            .as_str()
            .ok_or("no event name")?
            .to_owned();
        hook_with(
            &["--agent", agent, "--event", &event],
            serde_json::to_vec(payload)?,
            (Duration::ZERO, &self.home),
            &self.variables(temp),
        )
    }

    /// Turns adoption on and keeps `config` as the user's configuration, as `cortex init
    /// --global --yes` does once the holdout is answered there.
    pub fn adopting(&self, config: &str) -> TestResult {
        let user = self.user();
        std::fs::create_dir_all(user.path())?;
        std::fs::write(user.config_path(), config)?;
        let done = self.cortex(&["init", "--global", "--yes"])?;
        assert!(done.is_success, "{}", done.stderr);
        assert!(user.adopt_path().is_file());
        Ok(())
    }

    /// The file that marks when a hook last started a daemon for `folder`.
    pub fn start_marker(&self, folder: &Path) -> TestResult<PathBuf> {
        let id = RepoId::of(&canonical_root(folder)?);
        Ok(self.user().run_files(&id).start_path())
    }

    /// The refusal remembered for `folder`, once there is one.
    pub fn refusal(&self, folder: &Path) -> TestResult<PathBuf> {
        let id = RepoId::of(&canonical_root(folder)?);
        Ok(self.user().refusal_path(&id))
    }

    /// The log of set-ups that failed.
    pub fn failures_log(&self) -> PathBuf {
        self.user().logs_folder().join("adopt.log")
    }

    /// Moves the start marker of `folder` an hour back, long past the gap between two starts, so
    /// that a hook would start one again, and returns the time it now says.
    pub fn age_start_marker(&self, folder: &Path) -> TestResult<SystemTime> {
        let long_ago = SystemTime::now() - Duration::from_secs(3_600);
        std::fs::File::options()
            .write(true)
            .open(self.start_marker(folder)?)?
            .set_modified(long_ago)?;
        Ok(long_ago)
    }
}

/// The captured calculator at `folder`, made a git repository with every file committed, with
/// `home` as the user's home folder.
pub fn calc_repository(folder: &Path, home: &Path) -> TestResult {
    std::fs::create_dir_all(folder.join("calc"))?;
    std::fs::create_dir_all(folder.join("tests"))?;
    std::fs::write(folder.join("calc/__init__.py"), "")?;
    std::fs::write(folder.join("calc/ops.py"), OPS)?;
    std::fs::write(folder.join("tests/__init__.py"), "")?;
    std::fs::write(folder.join("tests/test_ops.py"), TESTS)?;
    std::fs::write(folder.join("main.py"), MAIN)?;
    super::git::git(folder, home, &["init", "-q", "-b", "main"])?;
    commit(folder, home, "calc", &["."])?;
    Ok(())
}

/// What a call gave the agent: none when it printed nothing, else its one line of hook JSON. It
/// must have ended as an agent requires: exit code 0 and nothing on stderr.
pub fn given(call: &Call) -> TestResult<Option<Value>> {
    let output = &call.output;
    if output.status.code() != Some(0) || !output.stderr.is_empty() {
        return Err(format!("{output:?}").into());
    }
    if output.stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&output.stdout)?))
}

/// `payload` as session `session` sends it.
pub fn in_session(mut payload: Value, session: &str) -> Value {
    payload["session_id"] = Value::from(session);
    payload
}

/// Every string `value` holds, in the order the file holds them, one space between each: a
/// command line, whether one string or a program and its arguments, reads as one.
pub fn words_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(words_of).collect::<Vec<_>>().join(" "),
        Value::Object(fields) => fields.values().map(words_of).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

/// Checks that the hook output `given` tells the user the welcome line, in its message for the
/// user, and nothing the model reads holds it: every other field, at any depth, is free of it.
pub fn welcomes_only_the_user(given: &Value) -> TestResult {
    assert_eq!(given["systemMessage"], Value::from(WELCOME), "{given}");
    let mut rest = given.clone();
    if let Value::Object(fields) = &mut rest {
        fields.remove("systemMessage");
    }
    assert!(
        !words_of(&rest).contains("Cortex set up this repository"),
        "{given}"
    );
    Ok(())
}

/// Waits until `condition` holds, for at most `within`.
pub fn wait_within(within: Duration, condition: impl Fn() -> TestResult<bool>) -> TestResult {
    let start = Instant::now();
    while !condition()? {
        if start.elapsed() > within {
            return Err("the condition did not hold in time".into());
        }
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}

/// What `git status --porcelain` says of the repository at `folder`: nothing for a clean one.
pub fn git_status(folder: &Path, home: &Path) -> TestResult<String> {
    let mut command = Command::new("git");
    command.args(["status", "--porcelain"]).current_dir(folder);
    isolate(&mut command, home);
    Ok(String::from_utf8(command.output()?.stdout)?)
}

/// The names in `folder`, sorted; none when it is not there.
pub fn names_in(folder: &Path) -> TestResult<Vec<String>> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Ok(Vec::new());
    };
    let mut names = entries
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}
