//! One run of one task: a fresh copy of its testbed, broken by the task's regression when it has
//! one, with Cortex installed for the arm, the task's
//! project rules noted unless the arm leaves them out, the agent's sessions in turn, the success check, what Cortex gave the measured session and what its end
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
use crate::testbed::{Testbed, apply_regression};
use crate::texts::{Texts, texts_of};
use crate::warned::{Warnings, warnings_of};

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
    /// The effort level the agent runs at, one of Claude Code's `--effort` levels.
    pub(crate) effort: String,
    /// Claude Code's own configuration folder, where its login is.
    pub(crate) claude_config: PathBuf,
    /// The folder each run's folder is made in.
    pub(crate) base: PathBuf,
    /// The folder the check scripts are in.
    pub(crate) checks: PathBuf,
    /// The folder the tasks' regression diffs are in.
    pub(crate) regressions: PathBuf,
    /// The share of tasks Cortex holds out: 0 for the treated arm, 1 for the held-out arm.
    pub(crate) holdout_share: f64,
    /// Cortex configuration lines every run's copy gets besides the holdout share, each
    /// `key = value` with a dotted key, as TOML reads them.
    pub(crate) config: Vec<String>,
    /// Whether each run notes its task's project rules before its first session.
    pub(crate) facts: bool,
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
    /// The warnings every session's edits got, by kind, as their tasks' ends labelled them; none in
    /// results from before the runner recorded them.
    #[serde(default)]
    pub(crate) warnings: Option<Warnings>,
    /// The configuration lines the run's copy got besides the holdout share; none in results from
    /// before the runner set any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) config: Vec<String>,
    /// Whether the run's arm noted its task's project rules before the first session; none in
    /// results from before an arm could leave them out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) facts: Option<bool>,
    /// How many project rules the run noted before its first session; 0 in results from before
    /// tasks had any.
    #[serde(default)]
    pub(crate) notes: u32,
    /// The effort level the agent ran at; none in results from before the runner set one, which
    /// ran at Claude Code's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

/// Runs `task` in a copy of `testbed`, from `from` when given, as run number `run`.
///
/// # Errors
///
/// A copy, a regression that does not apply, Cortex's install or its readiness, an agent that
/// cannot be started, or a store that cannot be read.
pub(crate) fn run_task(
    (task, testbed): (&Task, &Testbed),
    from: Option<&Path>,
    settings: &Settings,
    run: u32,
) -> anyhow::Result<TaskRun> {
    let folder = run_folder(&settings.base, &task.id, run);
    let (repo, home) = (folder.join("repo"), folder.join("home"));
    std::fs::create_dir_all(&home).with_context(|| format!("making {}", home.display()))?;
    testbed.prepare(from, &repo, task.history)?;
    if let Some(patch) = task.regression_in(&settings.regressions) {
        apply_regression(&repo, &patch)?;
    }
    install((&repo, &home), &folder, settings)?;
    let notes = if settings.facts {
        note_rules(task, (&repo, &home), &folder, settings)?
    } else {
        0
    };
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
    let is_success = check(
        task,
        (&repo, &home),
        &settings.checks,
        &folder.join("check.log"),
    )?;
    let given = match sessions.last().and_then(|last| last.session_id.as_deref()) {
        Some(measured) => given(&repo, measured)?,
        None => Given::default(),
    };
    let warnings = warnings_given(&repo, &sessions)?;
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
        warnings: Some(warnings),
        config: settings.config.clone(),
        facts: Some(settings.facts),
        notes,
        effort: Some(settings.effort.clone()),
    })
}

/// The configuration a run's copy gets: the arm's holdout share, then `lines`, each a dotted key
/// at the top of the file, where no table header can claim it.
pub(crate) fn config_text(holdout_share: f64, lines: &[String]) -> String {
    let mut text = format!("holdout.share = {holdout_share}\n");
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The folder run number `run` of task `task` works in, under `base`.
pub(crate) fn run_folder(base: &Path, task: &str, run: u32) -> PathBuf {
    base.join(format!("{task}-{run}"))
}

/// Deletes the folder of a run whose record is written: its copy of the testbed, its store and its
/// home, which nothing reads once the record is made. A suite of hundreds of runs would otherwise
/// keep a copy of a testbed, with the environment its checks installed, for each. Git keeps its
/// objects read-only, which Windows will not delete until that is cleared, so a first attempt that
/// fails clears the flag on every file below the folder, links not followed, and tries again.
///
/// # Errors
///
/// The folder cannot be read or deleted.
pub(crate) fn remove_run_folder(folder: &Path) -> anyhow::Result<()> {
    if std::fs::remove_dir_all(folder).is_ok() || !folder.exists() {
        return Ok(());
    }
    make_writable(folder)?;
    std::fs::remove_dir_all(folder).with_context(|| format!("deleting {}", folder.display()))
}

/// Clears the read-only flag of every file in `folder` and below it, links not followed.
fn make_writable(folder: &Path) -> anyhow::Result<()> {
    let entries =
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("reading {}", folder.display()))?
            .path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if metadata.is_dir() {
            make_writable(&path)?;
        } else if metadata.is_file() && metadata.permissions().readonly() {
            let mut permissions = metadata.permissions();
            // Only the read-only flag is cleared, on files this run made.
            #[allow(
                clippy::permissions_set_readonly_false,
                reason = "the file is about to be deleted, by the user who made it"
            )]
            permissions.set_readonly(false);
            std::fs::set_permissions(&path, permissions)
                .with_context(|| format!("making {} writable", path.display()))?;
        }
    }
    Ok(())
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

/// The `cortex` command, run in `repo` with `home` as the home folder, as a person would from their
/// own terminal, whoever runs the evaluation.
fn cortex(settings: &Settings, repo: &Path, home: &Path) -> Command {
    let mut command = Command::new(&settings.cortex);
    command
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CLAUDE_CONFIG_DIR", &settings.claude_config);
    for name in cortex_core::AGENT_SHELL_MARKERS {
        command.env_remove(name);
    }
    command
}

/// Installs Cortex in `repo` for Claude Code, the agent the evaluation runs, named so that no other
/// agent on this machine is set up, with the arm's holdout share; then waits for its index and
/// graph, logging each command in `folder`.
///
/// # Errors
///
/// `cortex init` fails, or the index or graph are not ready in time.
fn install((repo, home): (&Path, &Path), folder: &Path, settings: &Settings) -> anyhow::Result<()> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        config_text(settings.holdout_share, &settings.config),
    )?;
    let mut init = cortex(settings, repo, home);
    init.args(["init", "--yes", "--agents", "claude"]);
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

/// Notes `task`'s project rules in `repo` with `cortex note`, logging each command in `folder`,
/// and returns how many it noted.
///
/// # Errors
///
/// A rule `cortex note` refuses: a run without the rules its arm gives measures nothing.
fn note_rules(
    task: &Task,
    (repo, home): (&Path, &Path),
    folder: &Path,
    settings: &Settings,
) -> anyhow::Result<u32> {
    for (at, note) in task.notes.iter().enumerate() {
        let mut command = cortex(settings, repo, home);
        command.args(note.args());
        let log = folder.join(format!("note-{at}.log"));
        if !run_logged(command, &log, COMMAND_WITHIN)?.success() {
            bail!("cortex note failed:\n{}", tail_of(&log));
        }
    }
    Ok(u32::try_from(task.notes.len()).unwrap_or(u32::MAX))
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
        (&settings.model, &settings.effort, &task.tools),
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

/// Whether a recorded text is a work order: the text a prompt's retrieval gave, the one that keeps
/// the gate's features, and the only kind a task's end labels. A prompt is given other texts too,
/// a project rule or a handoff, recorded without features; and an edit's warnings, a read's hint,
/// the recap and strict mode's reason are recorded at other hooks. None of those is labelled.
fn is_work_order(record: &InjectionRecord) -> bool {
    record.hook == HookKind::Prompt && record.features.is_some()
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

/// The warnings the edits of every session of `sessions` in `repo` got, by kind, as their tasks'
/// ends labelled them.
///
/// # Errors
///
/// The store cannot be read.
fn warnings_given(repo: &Path, sessions: &[Session]) -> anyhow::Result<Warnings> {
    let reader = Reader::open(&repo.join(".cortex"), 0).context("opening the store")?;
    let queries = reader.queries();
    let mut rows = Vec::new();
    for session in sessions
        .iter()
        .filter_map(|session| session.session_id.as_deref())
    {
        let id = Identifier::new(session).context("reading the session id")?;
        for task in queries.tasks_of_session(&id)? {
            let warnings = queries.warnings_of_task(task.record.task_id)?;
            rows.extend(
                warnings
                    .into_iter()
                    .map(|row| (row.record.kind, row.resolved)),
            );
        }
    }
    Ok(warnings_of(rows))
}

/// The command of `task`'s check, its `{checks}` taken to the folder `checks`, run in `repo` with
/// `home` as the home folder, as the agent's sessions have it: a check that reads or writes the
/// user's files, as serena's tests do its configuration, works in the run's own and never in the
/// real ones.
fn check_command(
    task: &Task,
    (repo, home): (&Path, &Path),
    checks: &Path,
) -> anyhow::Result<Command> {
    let words: Vec<String> = task
        .check
        .iter()
        .map(|word| word.replace("{checks}", &checks.to_string_lossy()))
        .collect();
    let (program, args) = words.split_first().context("a check with no program")?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home);
    Ok(command)
}

/// Whether `task`'s check passes in `repo`, with `home` as its home folder, its `{checks}` taken
/// to the folder `checks`, its output written to `log`.
///
/// # Errors
///
/// The check's program cannot be started or outlasts its time.
fn check(
    task: &Task,
    (repo, home): (&Path, &Path),
    checks: &Path,
    log: &Path,
) -> anyhow::Result<bool> {
    let command = check_command(task, (repo, home), checks)?;
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
    use std::ffi::OsStr;

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

    /// A finished run's folder is deleted with everything in it, a read-only file two folders down
    /// included, as git leaves its objects; a folder already gone is not an error.
    #[test]
    fn a_finished_run_s_folder_is_deleted_read_only_files_and_all() -> anyhow::Result<()> {
        let base = tempfile::tempdir()?;
        let folder = run_folder(base.path(), "fix-tax", 2);
        let objects = folder.join("repo").join(".git").join("objects");
        std::fs::create_dir_all(&objects)?;
        let object = objects.join("ab12");
        std::fs::write(&object, b"blob")?;
        let mut permissions = std::fs::metadata(&object)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&object, permissions)?;
        remove_run_folder(&folder)?;
        assert!(!folder.exists());
        assert!(base.path().exists());
        remove_run_folder(&folder)?;
        Ok(())
    }

    /// A check runs in the copy with the run's own home folder, so that what it reads and writes
    /// under the home folder (serena's configuration and its lock) is the run's and not the
    /// user's: its `{checks}` becomes the checks folder, and `HOME` and `USERPROFILE` the home.
    #[test]
    fn a_check_runs_with_the_runs_own_home() -> anyhow::Result<()> {
        let text = "id = \"fix-tax\"\ntestbed = \"shop\"\nkind = \"fix\"\nprompt = \"Fix it.\"\n\
                    check = [\"python\", \"{checks}/fix-tax.py\"]\ntouches = [\"a.py\"]\n\
                    tools = [\"Read\"]\n";
        let task = Task::parse(text, Path::new("fix-tax.toml"))?;
        let command = check_command(
            &task,
            (Path::new("run/repo"), Path::new("run/home")),
            Path::new("suite/checks"),
        )?;
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["suite/checks/fix-tax.py"]);
        assert_eq!(command.get_current_dir(), Some(Path::new("run/repo")));
        for name in ["HOME", "USERPROFILE"] {
            let value = command.get_envs().find(|(key, _)| *key == name);
            assert_eq!(
                value,
                Some((OsStr::new(name), Some(OsStr::new("run/home"))))
            );
        }
        Ok(())
    }

    /// A run's configuration, as Cortex loads it: the treated arm's share, 0, then a warning
    /// setting given on the command line, each a dotted key at the top, so neither is read as part
    /// of the other's table and Cortex warns of no unknown key.
    #[test]
    fn a_run_s_configuration_reads_as_cortex_loads_it() -> anyhow::Result<()> {
        let text = config_text(0.0, &["warnings.max_per_edit = 0".to_owned()]);
        assert_eq!(
            text,
            "holdout.share = 0
warnings.max_per_edit = 0
"
        );
        let loaded = cortex_core::load_config(&[cortex_core::ConfigLayer {
            kind: cortex_core::LayerKind::Repository,
            origin: "config.toml",
            text: &text,
        }])?;
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert!(loaded.config.holdout_share.abs() < f64::EPSILON);
        assert_eq!(loaded.config.warnings.max_per_edit, 0);
        Ok(())
    }

    /// The gate's features of a retrieval, any values.
    const FEATURES: cortex_core::GateFeatures = cortex_core::GateFeatures {
        top_lexical: 0.5,
        top_mass: 0.5,
        agreeing_kinds: 1,
        supporting_kinds: 1,
        similar_task: 0.0,
        recent_precision: None,
        is_continuing: false,
    };

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

    /// A session's warnings, read hints and a prompt's other texts (a project rule, a handoff) are
    /// never labelled, so they do not hold a session from settling, nor count as work orders: with
    /// a work order delivered and labelled used, a warning, a read hint and a rule's text at the
    /// prompt delivered and unlabelled, and a work order the gate kept back, the session settles
    /// once its tasks have ended, and one work order was delivered, labelled and used. Until the
    /// work order is labelled, or while a task runs, it has not settled.
    #[test]
    fn only_work_orders_are_waited_for_and_counted() -> anyhow::Result<()> {
        let order = InjectionRecord {
            features: Some(FEATURES),
            ..text(HookKind::Prompt, true)?
        };
        let warning = text(HookKind::PostTool, true)?;
        let hint = text(HookKind::PreTool, true)?;
        let rule = text(HookKind::Prompt, true)?;
        let kept = InjectionRecord {
            features: Some(FEATURES),
            ..text(HookKind::Prompt, false)?
        };
        let rows: [Labelled<'_>; 5] = [
            (&order, Some(true)),
            (&warning, None),
            (&hint, None),
            (&rule, None),
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
