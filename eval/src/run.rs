//! One run of one task: a fresh copy of its testbed with Cortex installed for the arm, the agent's
//! sessions in turn, the success check, and what Cortex gave the measured session and what its
//! end made of it, read from the copy's store.
//!
//! Each run has its own folder under the base folder: the repository, a home folder for Cortex's
//! user files and daemon, each session's stream, and the log of every command the run waits for.
//! They stay there and never reach the results.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::Identifier;
use cortex_store::{InjectionRow, Reader};
use serde::Serialize;

use crate::agent::{self, Session};
use crate::process::{run_logged, tail_of};
use crate::task::Task;
use crate::testbed::Testbed;

/// How long a copy's first index and graph may take before the run gives up.
const READY_WITHIN: Duration = Duration::from_secs(900);

/// How long the daemon has, after a session ends, to end its tasks and label their injections.
const SETTLED_WITHIN: Duration = Duration::from_secs(120);

/// How long one agent session may run before it is stopped.
const SESSION_WITHIN: Duration = Duration::from_secs(1800);

/// How long a `cortex` command or a check may run before it is stopped.
const COMMAND_WITHIN: Duration = Duration::from_secs(300);

/// What every run of an evaluation shares.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// The `cortex` binary installed in each copy.
    pub(crate) cortex: PathBuf,
    /// The model the agent runs.
    pub(crate) model: String,
    /// Claude Code's own configuration folder, where its login is.
    pub(crate) claude_config: PathBuf,
    /// The folder each run's folder is made in.
    pub(crate) base: PathBuf,
    /// The folder the check scripts are in.
    pub(crate) checks: PathBuf,
    /// The share of tasks Cortex holds out: 0 for the treated arm, 1 for the held-out arm.
    pub(crate) holdout_share: f64,
}

/// What Cortex gave the measured session, as its store holds it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Given {
    /// Work orders that reached the agent.
    pub(crate) delivered: u64,
    /// Of those, the ones labelled at their task's end.
    pub(crate) labelled: u64,
    /// Of those, the ones labelled used.
    pub(crate) used: u64,
}

/// One run of one task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TaskRun {
    /// The task.
    pub(crate) task: String,
    /// Its kind.
    pub(crate) kind: String,
    /// The run's number, from 1.
    pub(crate) run: u32,
    /// The arm's holdout share.
    pub(crate) holdout_share: f64,
    /// Each session, the measured one last.
    pub(crate) sessions: Vec<Session>,
    /// Whether the check passed after the last session.
    pub(crate) is_success: bool,
    /// What Cortex gave the measured session.
    pub(crate) given: Given,
}

/// Runs `task` in a copy of `testbed`, from `from` when given, as run number `run`.
///
/// # Errors
///
/// A copy, Cortex's install or its readiness, an agent that cannot be started, or a store that
/// cannot be read.
pub(crate) fn run_task(
    (task, testbed): (&Task, &Testbed),
    from: Option<&Path>,
    settings: &Settings,
    run: u32,
) -> anyhow::Result<TaskRun> {
    let folder = settings.base.join(format!("{}-{run}", task.id));
    let (repo, home) = (folder.join("repo"), folder.join("home"));
    std::fs::create_dir_all(&home).with_context(|| format!("making {}", home.display()))?;
    testbed.prepare(from, &repo)?;
    install((&repo, &home), &folder, settings)?;
    let mut sessions = Vec::new();
    let prompts = task.warmups.iter().chain(std::iter::once(&task.prompt));
    for (at, prompt) in prompts.enumerate() {
        let stream = folder.join(format!("session-{at}.jsonl"));
        let session = session(prompt, (&repo, &home), task, settings, &stream)?;
        settle(&repo, session.session_id.as_deref())?;
        sessions.push(session);
    }
    let is_success = check(task, &repo, &settings.checks, &folder.join("check.log"))?;
    let given = match sessions.last().and_then(|last| last.session_id.as_deref()) {
        Some(measured) => given(&repo, measured)?,
        None => Given::default(),
    };
    uninstall((&repo, &home), &folder, settings)?;
    Ok(TaskRun {
        task: task.id.clone(),
        kind: task.kind.clone(),
        run,
        holdout_share: settings.holdout_share,
        sessions,
        is_success,
        given,
    })
}

/// The `cortex` command, run in `repo` with `home` as the home folder.
fn cortex(settings: &Settings, repo: &Path, home: &Path) -> Command {
    let mut command = Command::new(&settings.cortex);
    command
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CLAUDE_CONFIG_DIR", &settings.claude_config);
    command
}

/// Installs Cortex in `repo` with the arm's holdout share, then waits for its index and graph,
/// logging each command in `folder`.
///
/// # Errors
///
/// `cortex init` fails, or the index or graph are not ready in time.
fn install((repo, home): (&Path, &Path), folder: &Path, settings: &Settings) -> anyhow::Result<()> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        format!("[holdout]\nshare = {}\n", settings.holdout_share),
    )?;
    let mut init = cortex(settings, repo, home);
    init.args(["init", "--yes"]);
    let log = folder.join("init.log");
    if !run_logged(init, &log, COMMAND_WITHIN)?.success() {
        bail!("cortex init failed:\n{}", tail_of(&log));
    }
    let log = folder.join("status.log");
    let until = Instant::now() + READY_WITHIN;
    loop {
        let mut status = cortex(settings, repo, home);
        status.arg("status");
        run_logged(status, &log, COMMAND_WITHIN)?;
        let said = std::fs::read_to_string(&log).context("reading cortex status")?;
        let is_ready = said.lines().any(|line| line == "index: ready")
            && said
                .lines()
                .any(|line| line.starts_with("graph: ") && !line.contains("none built"));
        if is_ready {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!(
                "the index and graph of {} were not ready in time:\n{said}",
                repo.display()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Runs one session of `prompt` in `repo`, its stream written to `stream`, and returns what it
/// took.
///
/// # Errors
///
/// The agent cannot be started, outlasts its time, or its stream cannot be read.
fn session(
    prompt: &str,
    (repo, home): (&Path, &Path),
    task: &Task,
    settings: &Settings,
    stream: &Path,
) -> anyhow::Result<Session> {
    let command = agent::command(
        prompt,
        repo,
        (&settings.model, &task.tools),
        (home, &settings.claude_config),
    );
    run_logged(command, stream, SESSION_WITHIN)?;
    let text = std::fs::read(stream).with_context(|| format!("reading {}", stream.display()))?;
    Ok(Session::of_stream(&String::from_utf8_lossy(&text)))
}

/// Waits until every task of session `session` in `repo`'s store has ended and each injection
/// that reached the agent is labelled, as the daemon does once the session's stop is handled.
///
/// # Errors
///
/// The store cannot be read; or the daemon did not settle in time.
fn settle(repo: &Path, session: Option<&str>) -> anyhow::Result<()> {
    let Some(session) = session else {
        return Ok(());
    };
    let until = Instant::now() + SETTLED_WITHIN;
    loop {
        let (tasks_ended, rows) = read_session(repo, session)?;
        let is_labelled = rows
            .iter()
            .all(|row| !row.record.was_delivered() || row.label.is_some());
        if tasks_ended && is_labelled {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("session {session} did not settle in time");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Whether every task of `session` has ended, and the injections of its tasks.
///
/// # Errors
///
/// The store cannot be read.
fn read_session(repo: &Path, session: &str) -> anyhow::Result<(bool, Vec<InjectionRow>)> {
    let reader = Reader::open(&repo.join(".cortex"), 0).context("opening the store")?;
    let queries = reader.queries();
    let session = Identifier::new(session).context("reading the session id")?;
    let tasks = queries.tasks_of_session(&session)?;
    let mut rows = Vec::new();
    for task in &tasks {
        rows.extend(queries.injections_of_task(task.record.task_id)?);
    }
    Ok((tasks.iter().all(|task| task.ended_at_ms.is_some()), rows))
}

/// What Cortex gave the measured session `session` of `repo`.
///
/// # Errors
///
/// The store cannot be read.
fn given(repo: &Path, session: &str) -> anyhow::Result<Given> {
    let (_, rows) = read_session(repo, session)?;
    let count = |keep: &dyn Fn(&InjectionRow) -> bool| {
        u64::try_from(rows.iter().filter(|row| keep(row)).count()).unwrap_or(u64::MAX)
    };
    Ok(Given {
        delivered: count(&|row| row.record.was_delivered()),
        labelled: count(&|row| row.record.was_delivered() && row.label.is_some()),
        used: count(&|row| row.record.was_delivered() && row.label == Some(true)),
    })
}

/// Whether `task`'s check passes in `repo`, its `{checks}` taken to the folder `checks`, its
/// output written to `log`.
///
/// # Errors
///
/// The check's program cannot be started or outlasts its time.
fn check(task: &Task, repo: &Path, checks: &Path, log: &Path) -> anyhow::Result<bool> {
    let words: Vec<String> = task
        .check
        .iter()
        .map(|word| word.replace("{checks}", &checks.to_string_lossy()))
        .collect();
    let (program, args) = words.split_first().context("a check with no program")?;
    let mut command = Command::new(program);
    command.args(args).current_dir(repo);
    let status = run_logged(command, log, COMMAND_WITHIN)
        .with_context(|| format!("running the check of {}", task.id))?;
    Ok(status.success())
}

/// Takes Cortex out of `repo`, stopping its daemon, logging in `folder`.
///
/// # Errors
///
/// `cortex uninstall` cannot run or fails.
fn uninstall(
    (repo, home): (&Path, &Path),
    folder: &Path,
    settings: &Settings,
) -> anyhow::Result<()> {
    let mut command = cortex(settings, repo, home);
    command.arg("uninstall");
    let log = folder.join("uninstall.log");
    if !run_logged(command, &log, COMMAND_WITHIN)?.success() {
        bail!("cortex uninstall failed:\n{}", tail_of(&log));
    }
    Ok(())
}
