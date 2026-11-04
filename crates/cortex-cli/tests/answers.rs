//! The daemon's answers that are text, through its endpoint, on a repository whose history ties
//! its two files together: a prompt gets its work order, true to the files on disk, and leaves an
//! injection row; a held-out task's prompt gets nothing and leaves what would have been given; a
//! session's start gets the primer; a question asked directly is answered without the gate.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::git::{commit, git};
use common::{Daemon, TestResult, isolate, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, ContentHash, Event, HookKind, Identifier, ReasonKind, RepoId, RunFiles, StartSource,
    Ulid, UserFolder, canonical_root, estimate_tokens, task_id_of_prompt,
};
use cortex_hook::ask;
use cortex_store::{InjectionRow, Reader};

/// `src/orders/service.py`, its function on lines 1 to 3.
const SERVICE: &str = "def create_order(cart):\n    total = sum(cart)\n    return total\n";

/// `src/orders/model.py`, its class on lines 1 to 3.
const MODEL: &str = "class Order:\n    def __init__(self):\n        self.lines = []\n";

/// The prompt every test asks: it names `create_order`, and "orders" is in both files' paths.
const PROMPT: &str = "fix create_order in the orders service";

/// A repository whose two commits change the service and the model together, pinned to medium
/// mode, with `holdout_share` of tasks held out; its home folder; and its daemon, once the graph
/// holds the co-change links.
fn repository(holdout_share: &str) -> TestResult<(tempfile::TempDir, tempfile::TempDir, Daemon)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    git(root, home.path(), &["init", "-q"])?;
    std::fs::create_dir_all(root.join("src/orders"))?;
    for (commit_name, suffix) in [("one", ""), ("two", "\n")] {
        std::fs::write(
            root.join("src/orders/service.py"),
            format!("{SERVICE}{suffix}"),
        )?;
        std::fs::write(root.join("src/orders/model.py"), format!("{MODEL}{suffix}"))?;
        commit(
            root,
            home.path(),
            commit_name,
            &["src/orders/service.py", "src/orders/model.py"],
        )?;
    }
    std::fs::create_dir_all(root.join(".cortex"))?;
    std::fs::write(
        root.join(".cortex/config.toml"),
        format!("mode = \"medium\"\n[holdout]\nshare = {holdout_share}\n"),
    )?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(status_of(root, home.path())?.contains("with co-change links;")))?;
    Ok((repo, home, daemon))
}

/// What `cortex status` prints for the repository at `root`.
fn status_of(root: &Path, home: &Path) -> TestResult<String> {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["status", "--root"]).arg(root);
    isolate(&mut command, home);
    Ok(String::from_utf8(command.output()?.stdout)?)
}

/// The run files of the daemon of the repository at `root`.
fn run_files(root: &Path, home: &Path) -> TestResult<RunFiles> {
    Ok(UserFolder::in_home(home).run_files(&RepoId::of(&canonical_root(root)?)))
}

/// A hook event of `hook` from session `s1` at `ts_ms`.
fn event(root: &Path, hook: HookKind, ts_ms: i64) -> TestResult<Event> {
    let root = canonical_root(root)?;
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"answers"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: root.clone(),
        repo_root: root,
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// What the daemon answers `body` with: the text of its reply.
fn reply(files: &RunFiles, body: RequestBody) -> TestResult<Option<String>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    match ask(files, Role::Hook, body, deadline)?.body {
        ResponseBody::Reply { text } => Ok(text),
        other => Err(format!("{other:?}").into()),
    }
}

/// The injections of the task `prompt` began, once one is recorded.
fn injections_of(root: &Path, prompt: &Event) -> TestResult<Vec<InjectionRow>> {
    let task = task_id_of_prompt(prompt.event_id, prompt.ts_ms);
    let read = || -> TestResult<Vec<InjectionRow>> {
        Ok(Reader::open(&root.join(".cortex"), 0)?
            .queries()
            .injections_of_task(task)?)
    };
    wait_for(|| Ok(!read()?.is_empty()))?;
    read()
}

/// Every line the work order's "Current code" part shows of `path`, as (line number, text), as
/// the work order writes them: under the path's own line, each as `  N | text`.
fn shown_lines(text: &str, path: &str) -> Vec<(u32, String)> {
    let mut lines = text.lines();
    let mut shown = Vec::new();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with(&format!("{path}:")) || line.contains('(') {
            continue;
        }
        for code in lines.by_ref() {
            let Some((number, rest)) = code.split_once(" | ") else {
                break;
            };
            let Ok(number) = number.trim().parse::<u32>() else {
                break;
            };
            shown.push((number, rest.to_owned()));
        }
        break;
    }
    shown
}

/// A prompt that names `create_order` gets its work order: the service named, its code the file's
/// own lines; and its task's injection is recorded as given, the service first and named, with the
/// text's tokens, at four bytes a token, and its hash.
#[test]
fn a_prompt_gets_its_work_order_and_leaves_its_injection() -> TestResult {
    let (repo, home, _daemon) = repository("0")?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let prompt = Event {
        prompt_text: Some(PROMPT.to_owned()),
        ..event(root, HookKind::Prompt, 20)?
    };
    let text = reply(
        &files,
        RequestBody::Retrieve {
            event: Box::new(prompt.clone()),
        },
    )?
    .ok_or("a work order")?;
    assert!(text.starts_with("[cortex] Work order · "), "{text}");
    assert!(
        text.contains("src/orders/service.py:1-3  create_order  (named in the prompt)"),
        "{text}"
    );
    let on_disk: Vec<(u32, String)> = SERVICE
        .lines()
        .enumerate()
        .map(|(at, line)| (u32::try_from(at + 1).unwrap_or(0), line.to_owned()))
        .collect();
    assert_eq!(shown_lines(&text, "src/orders/service.py"), on_disk);
    let [row] = &injections_of(root, &prompt)?[..] else {
        return Err("one injection".into());
    };
    let first = row.record.files.first().ok_or("a file given")?;
    assert_eq!(first.reason, Some(ReasonKind::Named));
    assert_eq!(first.spans, [(1, 3)]);
    assert_eq!(row.record.hook, HookKind::Prompt);
    assert!(!row.record.is_gated_out && !row.record.is_holdout);
    assert_eq!(row.record.tokens, u32::try_from(estimate_tokens(&text))?);
    assert_eq!(row.record.text_hash, Some(ContentHash::of(text.as_bytes())));
    assert!(row.record.features.is_some());
    Ok(())
}

/// A held-out task's prompt gets nothing, and what would have been given is recorded: the same
/// files, marked held out.
#[test]
fn a_held_out_prompt_gets_nothing_and_leaves_what_it_would_have_got() -> TestResult {
    let (repo, home, _daemon) = repository("1")?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let prompt = Event {
        prompt_text: Some(PROMPT.to_owned()),
        ..event(root, HookKind::Prompt, 30)?
    };
    let text = reply(
        &files,
        RequestBody::Retrieve {
            event: Box::new(prompt.clone()),
        },
    )?;
    assert_eq!(text, None);
    let [row] = &injections_of(root, &prompt)?[..] else {
        return Err("one injection".into());
    };
    assert!(row.record.is_holdout && !row.record.is_gated_out);
    assert_eq!(
        row.record.files.first().and_then(|file| file.reason),
        Some(ReasonKind::Named)
    );
    assert!(row.record.text_hash.is_some());
    Ok(())
}

/// A session's start gets the primer, recorded as the session's; a vague prompt gets nothing and
/// leaves no injection; a question asked directly is answered without the gate.
#[test]
fn a_session_gets_its_primer_and_a_question_its_answer() -> TestResult {
    let (repo, home, _daemon) = repository("0")?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let start = Event {
        source: Some(StartSource::Startup),
        ..event(root, HookKind::SessionStart, 10)?
    };
    let primer = reply(
        &files,
        RequestBody::Primer {
            event: Box::new(start),
        },
    )?
    .ok_or("a primer")?;
    assert!(
        primer.starts_with("[cortex] Project map · medium mode · about 2 source files, "),
        "{primer}"
    );
    let compacted = Event {
        source: Some(StartSource::Compact),
        ..event(root, HookKind::SessionStart, 15)?
    };
    assert_eq!(
        reply(
            &files,
            RequestBody::Primer {
                event: Box::new(compacted)
            }
        )?,
        None
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mismatched = RequestBody::Retrieve {
        event: Box::new(event(root, HookKind::SessionStart, 12)?),
    };
    assert!(matches!(
        ask(&files, Role::Hook, mismatched, deadline)?.body,
        ResponseBody::Error { .. }
    ));
    let vague = Event {
        prompt_text: Some("make it better".to_owned()),
        ..event(root, HookKind::Prompt, 40)?
    };
    assert_eq!(
        reply(
            &files,
            RequestBody::Retrieve {
                event: Box::new(vague)
            }
        )?,
        None
    );
    let answer = reply(
        &files,
        RequestBody::Where {
            query: PROMPT.to_owned(),
            limit: 1,
        },
    )?
    .ok_or("an answer")?;
    assert!(answer.starts_with("[cortex] Where · "), "{answer}");
    assert!(answer.contains("src/orders/service.py:1-3"), "{answer}");
    assert!(!answer.contains("src/orders/model.py"), "{answer}");
    // Another session's primer comes last: injections are recorded in the order they came, so
    // once its row is there, any the compacted start or the vague prompt left would be too.
    let later = Identifier::new("s2")?;
    let other = Event {
        source: Some(StartSource::Startup),
        session_id: later.clone(),
        ..event(root, HookKind::SessionStart, 50)?
    };
    reply(
        &files,
        RequestBody::Primer {
            event: Box::new(other),
        },
    )?
    .ok_or("the other session's primer")?;
    let reader = || Reader::open(&root.join(".cortex"), 0);
    wait_for(|| {
        Ok(!reader()?
            .queries()
            .injections_of_session(&later)?
            .is_empty())
    })?;
    let session = Identifier::new("s1")?;
    let rows = reader()?.queries().injections_of_session(&session)?;
    let hooks: Vec<HookKind> = rows.iter().map(|row| row.record.hook).collect();
    assert_eq!(hooks, [HookKind::SessionStart]);
    assert_eq!(
        rows.first().and_then(|row| row.record.text_hash),
        Some(ContentHash::of(primer.as_bytes()))
    );
    Ok(())
}
