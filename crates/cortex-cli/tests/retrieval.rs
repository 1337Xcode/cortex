//! Retrieval's acceptance, through the real binary as Claude Code runs it, on the repository the
//! captured payloads come from (pinned to medium mode, so that retrieval runs): a first task on a
//! cold map, whose session start gets the primer and whose prompt gets nothing the gate did not
//! let through; a task in the area the first worked in, whose prompt gets a work order whose every
//! line number and code line is the file's on disk, the same for the same prompt from another
//! session, and labelled used once the task edits its file; a vague prompt, which gets nothing;
//! and a held-out task, which gets nothing while what it would have got is recorded.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{
    OPS, OPS_FIXED, S1, captured, edit_back, prompt, read_of_tests, repository, tests_run,
};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, isolate, printed, wait_for};
use cortex_core::{ContentHash, Identifier, RepoPath, TaskOutcome};
use cortex_store::{InjectionRow, Queries, Reader, TaskRow};
use serde_json::Value;

/// A prompt in the area the first task worked in: it names `divide` and the file it is in.
const FAMILIAR: &str = "Make divide in calc/ops.py round its result to two places.";

/// The session the familiar prompt comes from, later: a new one, whose agent has read nothing.
const LATER: &str = "00000000-0000-4000-8000-000000000002";

/// Another new session, which asks the same.
const ELSEWHERE: &str = "00000000-0000-4000-8000-000000000003";

/// Another new session, the third to ask after the first task.
const THIRD: &str = "00000000-0000-4000-8000-000000000004";

/// The repository of the captured payloads, pinned to medium mode, with `share` of its tasks held
/// out; its home folder; and its daemon, once its graph is built.
fn started(share: &str) -> TestResult<(tempfile::TempDir, std::path::PathBuf, Daemon)> {
    let (home, repo) = repository()?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        format!("mode = \"medium\"\n[holdout]\nshare = {share}\n"),
    )?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    Ok((home, repo, daemon))
}

/// Runs `cortex hook` on `payload` as Claude Code does, and returns what it gave the agent.
fn given(payload: &Value, home: &Path) -> TestResult<Option<String>> {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let call = hook(
        &["--agent", "claude", "--event", &event],
        serde_json::to_vec(payload)?,
        Duration::ZERO,
        home,
    )?;
    call.context(&event)
}

/// `payload` as session `session` sends it.
fn in_session(mut payload: Value, session: &str) -> Value {
    payload["session_id"] = Value::from(session);
    payload
}

/// The captured prompt with `words` of its own, from session `session`.
fn later(root: &Path, words: &str, session: &str) -> TestResult<Value> {
    Ok(in_session(prompt(root, words)?, session))
}

/// Reads the store of the repository at `root` with `read`.
fn read<T>(root: &Path, read: impl FnOnce(Queries<'_>) -> TestResult<T>) -> TestResult<T> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    read(reader.queries())
}

/// Whether the store holds the file at `path` with the contents `text`: the daemon read that
/// change.
fn holds(root: &Path, path: &str, text: &str) -> TestResult<bool> {
    read(root, |queries| {
        let row = queries.file(&RepoPath::new(path)?)?;
        Ok(row.is_some_and(|row| row.file.content_hash == Some(ContentHash::of(text.as_bytes()))))
    })
}

/// The tasks of session `session`, by start.
fn tasks(root: &Path, session: &str) -> TestResult<Vec<TaskRow>> {
    let session = Identifier::new(session)?;
    read(root, |queries| Ok(queries.tasks_of_session(&session)?))
}

/// The injections of the latest task of session `session`, once it has any.
fn injections_of_latest(root: &Path, session: &str) -> TestResult<Vec<InjectionRow>> {
    let latest = || -> TestResult<Vec<InjectionRow>> {
        let Some(task) = tasks(root, session)?.last().map(|task| task.record.task_id) else {
            return Ok(Vec::new());
        };
        read(root, |queries| Ok(queries.injections_of_task(task)?))
    };
    wait_for(|| Ok(!latest()?.is_empty()))?;
    latest()
}

/// The first task of the captured session on a cold map: it searches, reads the code and its
/// tests, fixes `divide` and passes the tests, then stops; what its prompt was given, and the
/// injection the prompt left.
fn first_task(root: &Path, home: &Path) -> TestResult<(Option<String>, InjectionRow)> {
    let prompted = given(&captured("user-prompt-submit.json", root)?, home)?;
    let injection = injections_of_latest(root, S1)?
        .into_iter()
        .next()
        .ok_or("the prompt left no injection")?;
    for name in [
        "post-tool-use-glob.json",
        "post-tool-use-grep.json",
        "post-tool-use-read.json",
    ] {
        given(&captured(name, root)?, home)?;
    }
    given(&read_of_tests(root)?, home)?;
    given(&captured("post-tool-use-edit.json", root)?, home)?;
    std::fs::write(root.join("calc/ops.py"), OPS_FIXED)?;
    // The daemon reads the fixed file while the agent runs the tests, as it would in a session;
    // until it has, a work order holds back the file's code, its lines not yet the index's.
    wait_for(|| holds(root, "calc/ops.py", OPS_FIXED))?;
    given(
        &tests_run(root, "post-tool-use-bash-test-passing.json")?,
        home,
    )?;
    given(&captured("stop.json", root)?, home)?;
    let first = tasks(root, S1)?.first().ok_or("no task")?.record.task_id;
    wait_for(|| {
        read(root, |queries| {
            Ok(queries
                .episode_of_task(first)?
                .is_some_and(|episode| episode.outcome == TaskOutcome::Success))
        })
    })?;
    Ok((prompted, injection))
}

/// The lines of code a work order shows, each with its file and its number: under "Current code:",
/// a line `  path:start-end` names the file and each line `  N | code` one of its lines.
fn shown_code(work_order: &str) -> TestResult<Vec<(String, usize, String)>> {
    let (_, code) = work_order
        .split_once("Current code:\n")
        .ok_or("no current code")?;
    let mut shown = Vec::new();
    let mut file: Option<String> = None;
    for line in code.lines() {
        let Some(line) = line.strip_prefix("  ") else {
            break;
        };
        if let Some((number, text)) = line.split_once(" | ") {
            // The line marking the lines between a class's first line and a method shown under
            // it holds no number and no code.
            if number.trim().is_empty() && text == "..." {
                continue;
            }
            let number: usize = number.trim().parse()?;
            shown.push((
                file.clone().ok_or("code before its file")?,
                number,
                text.to_owned(),
            ));
        } else if let Some((path, _)) = line.rsplit_once(':') {
            file = Some(path.to_owned());
        }
    }
    Ok(shown)
}

/// F2 and F3, and F9 between them. On a cold map the session's start gets the primer; the first
/// prompt gets something only when its injection passed the gate (two kinds of evidence agreeing
/// with the words matching a file at 0.50 or more, or the work going on); the task succeeds and
/// its record is kept. Later, from a new session, a prompt naming `divide` and its file gets a
/// work order whose every line shown is the file's own line on disk, its number included; the
/// same prompt from another new session gets the same work order, byte for byte; once the task
/// edits `calc/ops.py` and stops, its injection is labelled used. "make it better" gets nothing.
#[test]
fn a_familiar_area_gets_a_work_order_true_to_the_files() -> TestResult {
    let (home, repo, _daemon) = started("0")?;
    let home = home.path();
    let primer =
        given(&captured("session-start-startup.json", &repo)?, home)?.ok_or("no primer")?;
    assert!(
        primer.starts_with("[cortex] Project map · medium mode · about 5 source files, "),
        "{primer}"
    );
    assert!(
        primer.ends_with(
            "Cortex adds the code a prompt points at before the agent starts, when the evidence \
             agrees.\n"
        ),
        "{primer}"
    );
    let (prompted, injection) = first_task(&repo, home)?;
    let features = injection.record.features.ok_or("the gate's numbers")?;
    let is_passed =
        (features.agreeing_kinds >= 2 && features.top_lexical >= 0.5) || features.is_continuing;
    assert_eq!(
        prompted.is_some(),
        injection.record.was_delivered(),
        "{injection:?}"
    );
    assert!(prompted.is_none() || is_passed, "{features:?}");

    let work_order = given(&later(&repo, FAMILIAR, LATER)?, home)?.ok_or("no work order")?;
    assert!(
        work_order.starts_with("[cortex] Work order · "),
        "{work_order}"
    );
    let shown = shown_code(&work_order)?;
    assert!(
        shown.iter().any(|(file, _, _)| file == "calc/ops.py"),
        "{work_order}"
    );
    for (file, number, text) in &shown {
        let on_disk = std::fs::read_to_string(repo.join(file))?;
        let line = on_disk
            .lines()
            .nth(number - 1)
            .ok_or("past the file's end")?;
        assert_eq!(text, line, "{file}:{number} in {work_order}");
    }
    assert_eq!(
        given(&later(&repo, FAMILIAR, ELSEWHERE)?, home)?.as_deref(),
        Some(work_order.as_str())
    );

    given(&in_session(edit_back(&repo)?, LATER), home)?;
    std::fs::write(repo.join("calc/ops.py"), OPS)?;
    given(&in_session(captured("stop.json", &repo)?, LATER), home)?;
    let familiar = tasks(&repo, LATER)?
        .first()
        .ok_or("no later task")?
        .record
        .task_id;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries
                .injections_of_task(familiar)?
                .iter()
                .any(|injection| injection.label == Some(true)))
        })
    })?;

    assert_eq!(given(&later(&repo, "make it better", LATER)?, home)?, None);
    wait_for(|| Ok(tasks(&repo, LATER)?.len() == 2))?;
    let vague = tasks(&repo, LATER)?
        .get(1)
        .ok_or("no vague task")?
        .record
        .task_id;
    let recorded = read(&repo, |queries| Ok(queries.injections_of_task(vague)?))?;
    assert!(
        recorded
            .iter()
            .all(|injection| !injection.record.was_delivered()),
        "{recorded:?}"
    );
    Ok(())
}

/// A held-out task gets nothing, and what it would have got is recorded: with every task held
/// out, after the same first task, the prompt naming `divide` prints nothing, and its injection is
/// recorded as held out with the hash of the work order it would have been given.
#[test]
fn a_held_out_task_gets_nothing_but_its_injection_is_recorded() -> TestResult {
    let (home, repo, _daemon) = started("1")?;
    let home = home.path();
    first_task(&repo, home)?;
    assert_eq!(given(&later(&repo, FAMILIAR, LATER)?, home)?, None);
    let held = injections_of_latest(&repo, LATER)?;
    let [injection] = held.as_slice() else {
        return Err(format!("{} injections", held.len()).into());
    };
    assert!(injection.record.is_holdout, "{injection:?}");
    assert!(!injection.record.is_gated_out, "{injection:?}");
    assert!(injection.record.text_hash.is_some(), "{injection:?}");
    assert!(!injection.record.was_delivered());
    Ok(())
}

/// A slash command naming the file the familiar prompt names, and "ok", ask for no work order:
/// each prints nothing and starts a task of its own, which holds no injection, since the daemon
/// was sent the prompt's event alone and retrieved nothing for it. The familiar prompt, asked the
/// same way, gets one.
#[test]
fn a_command_or_a_word_starts_a_task_and_asks_for_nothing() -> TestResult {
    let (home, repo, _daemon) = started("0")?;
    for (words, count) in [("/review calc/ops.py divide", 1), ("ok", 2)] {
        assert_eq!(given(&later(&repo, words, LATER)?, home.path())?, None);
        wait_for(|| Ok(tasks(&repo, LATER)?.len() == count))?;
    }
    let asked = given(&later(&repo, FAMILIAR, LATER)?, home.path())?;
    assert!(asked.is_some(), "the familiar prompt got nothing");
    let all = tasks(&repo, LATER)?;
    let [command, word, familiar] = &all[..] else {
        return Err(format!("{} tasks", all.len()).into());
    };
    wait_for(|| {
        read(&repo, |queries| {
            Ok(!queries
                .injections_of_task(familiar.record.task_id)?
                .is_empty())
        })
    })?;
    for task in [command, word] {
        let recorded = read(&repo, |queries| {
            Ok(queries.injections_of_task(task.record.task_id)?)
        })?;
        assert!(recorded.is_empty(), "{recorded:?}");
    }
    Ok(())
}

/// The code a work order showed is not shown again in its session until the session's context is
/// compacted: the familiar prompt's work order shows `divide` (calc/ops.py, lines 12 to 13); the
/// same prompt again in the same session does not show those lines; after a compaction it does.
#[test]
fn a_session_is_not_shown_the_same_code_twice() -> TestResult {
    let (home, repo, _daemon) = started("0")?;
    let divide = "  12 | def divide(a, b):\n";
    let first = given(&later(&repo, FAMILIAR, LATER)?, home.path())?.ok_or("no work order")?;
    assert!(first.contains(divide), "{first}");
    injections_of_latest(&repo, LATER)?;
    let again = given(&later(&repo, FAMILIAR, LATER)?, home.path())?;
    assert!(
        again.as_deref().is_none_or(|again| !again.contains(divide)),
        "{again:?}"
    );
    wait_for(|| Ok(tasks(&repo, LATER)?.len() == 2))?;

    given(
        &in_session(captured("pre-compact-manual.json", &repo)?, LATER),
        home.path(),
    )?;
    wait_for(|| {
        read(&repo, |queries| {
            let compacted = queries
                .events_of_session(&Identifier::new(LATER)?)?
                .iter()
                .any(|row| row.record.kind == cortex_core::EventKind::Compact);
            Ok(compacted)
        })
    })?;
    let after = given(&later(&repo, FAMILIAR, LATER)?, home.path())?.ok_or("no work order")?;
    assert!(after.contains(divide), "{after}");
    Ok(())
}

/// What `cortex` printed with `args` in the repository at `root`, `home` being the home folder,
/// having succeeded.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).arg("--root").arg(root).current_dir(root);
    isolate(&mut command, home);
    let printed = printed(&mut command, b"")?;
    if !printed.is_success {
        return Err(format!("cortex {args:?} failed: {}", printed.stderr).into());
    }
    Ok(printed.stdout)
}

/// F10. After the first task, the user notes that `divide` rounds to two places. A prompt naming
/// `divide`, from a new session, gets a work order carrying the rule under "Project facts", with
/// `calc/ops.py:divide`, and the fact counts as given once. `divide` changes on disk (back to
/// floor division): the daemon reads the change, the fact turns suspect, and the same prompt from
/// another new session gets a work order with no fact. Confirmed against the code as it is now, the
/// fact is carried again, to a third session.
#[test]
fn a_fact_is_given_while_its_code_holds_and_withheld_once_it_changes() -> TestResult {
    let (home, repo, _daemon) = started("0")?;
    let home = home.path();
    first_task(&repo, home)?;
    let rule = "divide rounds its result to two places.";
    assert_eq!(
        cortex(&repo, home, &["note", rule, "--at", "calc/ops.py:divide"])?,
        "fact 1 added\n"
    );
    let line = format!("Project facts:\n  - {rule} (anchored to calc/ops.py:divide)\n");
    let work_order = given(&later(&repo, FAMILIAR, LATER)?, home)?.ok_or("no work order")?;
    assert!(work_order.contains(&line), "{work_order}");
    wait_for(|| Ok(cortex(&repo, home, &["facts", "list"])?.contains("given once")))?;

    std::fs::write(repo.join("calc/ops.py"), OPS)?;
    wait_for(|| Ok(cortex(&repo, home, &["facts", "list"])?.contains("1  suspect")))?;
    let withheld = given(&later(&repo, FAMILIAR, ELSEWHERE)?, home)?.ok_or("no work order")?;
    assert!(!withheld.contains("Project facts"), "{withheld}");

    assert_eq!(
        cortex(&repo, home, &["facts", "confirm", "1"])?,
        "fact 1 confirmed\n"
    );
    let again = given(&later(&repo, FAMILIAR, THIRD)?, home)?.ok_or("no work order")?;
    assert!(again.contains(&line), "{again}");
    Ok(())
}
