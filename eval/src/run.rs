//! One run of one task: a fresh copy of its testbed with Cortex installed for the arm, the agent's
//! sessions in turn, the success check, what Cortex gave the measured session and what its end
//! made of it, read from the copy's store, and whether every code line a session was shown was the
//! line its file held when the session began.
//!
//! Each run has its own folder under the base folder: the repository, a home folder for Cortex's
//! user files and daemon, each session's stream, and the log of every command the run waits for.
//! They stay there and never reach the results.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::{HookKind, Identifier};
use cortex_store::{InjectionRecord, InjectionRow, Reader};
use serde::{Deserialize, Serialize};

use crate::agent::{self, Session};
use crate::process::{run_logged, tail_of};
use crate::shown::{self, Shown};
use crate::snapshot::Snapshot;
use crate::task::Task;
use crate::testbed::Testbed;
use crate::texts::{Texts, texts_of};

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Given {
    /// Work orders that reached the agent.
    pub(crate) delivered: u64,
    /// Of those, the ones labelled at their task's end.
    pub(crate) labelled: u64,
    /// Of those, the ones labelled used.
    pub(crate) used: u64,
}

/// One run of one task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// The check of the code every session was shown; none in results from before the runner
    /// checked it.
    #[serde(default)]
    pub(crate) shown: Option<Shown>,
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
    let folder = run_folder(&settings.base, &task.id, run);
    let (repo, home) = (folder.join("repo"), folder.join("home"));
    std::fs::create_dir_all(&home).with_context(|| format!("making {}", home.display()))?;
    testbed.prepare(from, &repo)?;
    install((&repo, &home), &folder, settings)?;
    let mut sessions = Vec::new();
    let mut shown = Shown::default();
    let prompts = task.warmups.iter().chain(std::iter::once(&task.prompt));
    for (at, prompt) in prompts.enumerate() {
        let stream = folder.join(format!("session-{at}.jsonl"));
        let before = Snapshot::take(&repo, &folder, &format!("snapshot-{at}"))?;
        let mut session = session(prompt, (&repo, &home), task, settings, &stream)?;
        settle(&repo, session.session_id.as_deref())?;
        session.texts = texts_given(&repo, session.session_id.as_deref())?;
        let checked = check_shown(&repo, session.session_id.as_deref(), &before, &folder)?;
        shown = shown.add(checked);
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
        shown: Some(shown),
    })
}

/// The folder run number `run` of task `task` works in, under `base`.
pub(crate) fn run_folder(base: &Path, task: &str, run: u32) -> PathBuf {
    base.join(format!("{task}-{run}"))
}

/// The run folders of `tasks`, each run `runs` times, that exist under `base` already: a run never
/// works in a folder an earlier one left, whose copy, store and home are not fresh.
pub(crate) fn taken_folders(base: &Path, tasks: &[&str], runs: u32) -> Vec<PathBuf> {
    tasks
        .iter()
        .flat_map(|task| (1..=runs).map(move |run| run_folder(base, task, run)))
        .filter(|folder| folder.exists())
        .collect()
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

/// A recorded text and the label its task's end gave it, as the runner weighs them.
type Labelled<'a> = (&'a InjectionRecord, Option<bool>);

/// Waits until every task of session `session` in `repo`'s store has ended and each work order
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
        if is_settled(tasks_ended, &labelled(&rows)) {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("session {session} did not settle in time");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Whether a recorded text is a work order: the text a prompt was given, and the only kind a
/// task's end labels. An edit's warnings, a read's hint, the recap and strict mode's reason are
/// recorded too, and never labelled.
fn is_work_order(record: &InjectionRecord) -> bool {
    record.hook == HookKind::Prompt
}

/// Whether a session has settled: its tasks have `tasks_ended`, and each work order of `rows` that
/// reached the agent is labelled.
fn is_settled(tasks_ended: bool, rows: &[Labelled<'_>]) -> bool {
    tasks_ended
        && rows
            .iter()
            .filter(|(record, _)| is_work_order(record))
            .all(|(record, label)| !record.was_delivered() || label.is_some())
}

/// Each of `rows` with its label.
fn labelled(rows: &[InjectionRow]) -> Vec<Labelled<'_>> {
    rows.iter().map(|row| (&row.record, row.label)).collect()
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
    Ok(given_of(&labelled(&rows)))
}

/// What the work orders of `rows` come to: how many reached the agent, and of those how many were
/// labelled and labelled used.
fn given_of(rows: &[Labelled<'_>]) -> Given {
    let count = |keep: &dyn Fn(Option<bool>) -> bool| {
        let kept = rows.iter().filter(|(record, label)| {
            is_work_order(record) && record.was_delivered() && keep(*label)
        });
        u64::try_from(kept.count()).unwrap_or(u64::MAX)
    };
    Given {
        delivered: count(&|_| true),
        labelled: count(&|label| label.is_some()),
        used: count(&|label| label == Some(true)),
    }
}

/// The check of the code the session `session` of `repo` was shown, against `before`, the copy's
/// files when the session began. The work orders are read from the session's transcript, found
/// through the store; the path and number of each line that differs, and why a session could not be
/// checked, go to `shown.log` in `folder`.
///
/// # Errors
///
/// The store cannot be read, or the snapshot's git fails.
fn check_shown(
    repo: &Path,
    session: Option<&str>,
    before: &Snapshot,
    folder: &Path,
) -> anyhow::Result<Shown> {
    let log_path = folder.join("shown.log");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let unread = |log: &mut std::fs::File, why: &str| -> anyhow::Result<Shown> {
        writeln!(
            log,
            "session {}: not checked, {why}",
            session.unwrap_or("unknown")
        )?;
        Ok(Shown {
            unread_sessions: 1,
            ..Shown::default()
        })
    };
    let Some(session) = session else {
        return unread(&mut log, "its stream named no session");
    };
    let reader = Reader::open(&repo.join(".cortex"), 0).context("opening the store")?;
    let id = Identifier::new(session).context("reading the session id")?;
    let transcript = reader
        .queries()
        .session(&id)?
        .and_then(|row| row.record.transcript_path);
    let Some(transcript) = transcript else {
        return unread(&mut log, "the store holds no transcript for it");
    };
    let Ok(text) = std::fs::read(&transcript) else {
        return unread(&mut log, "its transcript could not be read");
    };
    let lines = shown::shown_lines(&String::from_utf8_lossy(&text));
    let (shown, differing) = shown::check(&lines, |path| before.file(path))?;
    writeln!(
        log,
        "session {session}: {} lines, {} differ",
        shown.lines, shown.differ
    )?;
    for (path, number) in differing {
        writeln!(log, "  {path}:{number}")?;
    }
    Ok(shown)
}

/// The texts Cortex gave the session `session` of `repo`, by the hook they answered: its
/// primer and work orders, and every warning, hint, recap and reason. None for a session whose
/// stream named no id.
///
/// # Errors
///
/// The store cannot be read.
fn texts_given(repo: &Path, session: Option<&str>) -> anyhow::Result<Option<Texts>> {
    let Some(session) = session else {
        return Ok(None);
    };
    let reader = Reader::open(&repo.join(".cortex"), 0).context("opening the store")?;
    let id = Identifier::new(session).context("reading the session id")?;
    let rows = reader.queries().injections_of_session(&id)?;
    Ok(Some(texts_of(rows.iter().map(|row| &row.record))))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Of the runs planned, the folders an earlier evaluation left are found, and only those: run 2
    /// of `fix-tax` is taken; run 1 is not, nor is another task's folder counted.
    #[test]
    fn a_run_folder_left_by_an_earlier_evaluation_is_found() -> std::io::Result<()> {
        let base = tempfile::tempdir()?;
        std::fs::create_dir(base.path().join("fix-tax-2"))?;
        std::fs::create_dir(base.path().join("other-1"))?;
        let taken = taken_folders(base.path(), &["fix-tax"], 2);
        assert_eq!(taken, [base.path().join("fix-tax-2")]);
        assert!(taken_folders(base.path(), &["fix-tax"], 1).is_empty());
        Ok(())
    }

    /// A text of `hook` recorded for session `s`, delivered or kept back by the gate.
    fn text(hook: HookKind, is_delivered: bool) -> anyhow::Result<InjectionRecord> {
        Ok(InjectionRecord {
            task_id: None,
            session_id: Identifier::new("s")?,
            hook,
            at_ms: 1,
            files: Vec::new(),
            tokens: 10,
            confidence: None,
            is_gated_out: !is_delivered,
            is_holdout: false,
            text_hash: Some(cortex_core::ContentHash::of(b"text")),
            features: None,
        })
    }

    /// A session's warnings and read hints are never labelled, so they do not hold a session from
    /// settling, nor count as work orders: with a work order delivered and labelled used, a
    /// warning and a read hint delivered and unlabelled, and a work order the gate kept back, the
    /// session settles once its tasks have ended, and one work order was delivered, labelled and
    /// used. Until the work order is labelled, or while a task runs, it has not settled.
    #[test]
    fn only_work_orders_are_waited_for_and_counted() -> anyhow::Result<()> {
        let order = text(HookKind::Prompt, true)?;
        let warning = text(HookKind::PostTool, true)?;
        let hint = text(HookKind::PreTool, true)?;
        let kept = text(HookKind::Prompt, false)?;
        let rows: [Labelled<'_>; 4] = [
            (&order, Some(true)),
            (&warning, None),
            (&hint, None),
            (&kept, None),
        ];
        assert!(is_settled(true, &rows));
        assert!(!is_settled(false, &rows));
        assert!(!is_settled(true, &[(&order, None), (&warning, None)]));
        assert_eq!(
            given_of(&rows),
            Given {
                delivered: 1,
                labelled: 1,
                used: 1
            }
        );
        Ok(())
    }
}
