//! The daemon's answers that are text, on a repository whose history ties its two files together,
//! through the daemon's endpoint and through `cortex hook` as Claude Code runs it: a prompt gets
//! its work order, true to the files on disk, and leaves an injection row; a held-out task's prompt
//! gets nothing and leaves what would have been given; a session's start gets the primer, the same
//! bytes for every session; a question asked directly is answered without the gate.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::git::{commit, git};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, payloads, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, ContentHash, Event, HookKind, Identifier, ReasonKind, RepoId, RunFiles, StartSource,
    Ulid, UserFolder, canonical_root, estimate_tokens, task_id_of_prompt,
};
use cortex_hook::ask;
use cortex_store::{InjectionRow, Reader};
use serde_json::{Value, json};

/// `src/orders/service.py`, its function on lines 1 to 3.
const SERVICE: &str = "def create_order(cart):\n    total = sum(cart)\n    return total\n";

/// `src/orders/model.py`, its class on lines 1 to 3.
const MODEL: &str = "class Order:\n    def __init__(self):\n        self.lines = []\n";

/// The prompt every test asks: it names `create_order`, and "orders" is in both files' paths.
const PROMPT: &str = "fix create_order in the orders service";

/// The primer of the repository of [`repository`]: medium mode, as its configuration pins it; its
/// two source files of 4 lines each once the second commit added a line break to both; no
/// landmarks while no task has ended.
const PRIMER: &str = "[cortex] Project map · medium mode · about 2 source files, 8 lines\n\
                      Cortex adds the code a prompt points at before the agent starts, when the \
                      evidence agrees.\n";

/// The Claude Code version whose captured payloads the hook is sent.
const CAPTURED: &str = "2.1.312";

/// The session of the captured payloads.
const S1: &str = "00000000-0000-4000-8000-000000000001";

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
    wait_for(|| Ok(graph_of(root, home.path())?.is_some_and(|graph| graph.has_cochange)))?;
    Ok((repo, home, daemon))
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

/// A request for the work order `prompt` gets, from a client that waits as long as it takes.
fn work_order_request(prompt: Event) -> RequestBody {
    RequestBody::Retrieve {
        event: Box::new(prompt),
        answer_by_ms: None,
    }
}

/// A request for the primer the session `start` begins gets, from a client that waits as long as
/// it takes.
fn primer_request(start: Event) -> RequestBody {
    RequestBody::Primer {
        event: Box::new(start),
        answer_by_ms: None,
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

/// The captured payload `name` from the repository at `root`, each of `fields` set to its text.
fn captured(name: &str, root: &Path, fields: &[(&str, &str)]) -> TestResult<Vec<u8>> {
    let mut payload: Value = serde_json::from_slice(&payloads::payload(CAPTURED, name, root)?)?;
    let object = payload
        .as_object_mut()
        .ok_or("a payload is a JSON object")?;
    for (field, text) in fields {
        object.insert((*field).to_owned(), json!(text));
    }
    Ok(serde_json::to_vec(&payload)?)
}

/// What `cortex hook` gives Claude Code at `event` for `payload`: none when it prints nothing.
fn given(event: &str, payload: Vec<u8>, home: &Path) -> TestResult<Option<String>> {
    let args = ["--agent", "claude", "--event", event];
    hook(&args, payload, Duration::ZERO, home)?.context(event)
}

/// The lines of `source`, as (line number, text), as a work order shows them.
fn numbered(source: &str) -> Vec<(u32, String)> {
    (1..)
        .zip(source.lines())
        .map(|(number, line)| (number, line.to_owned()))
        .collect()
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
    let text = reply(&files, work_order_request(prompt.clone()))?.ok_or("a work order")?;
    assert!(text.starts_with("[cortex] Work order · "), "{text}");
    assert!(
        text.contains("src/orders/service.py:1-3  create_order  (named in the prompt)"),
        "{text}"
    );
    assert_eq!(
        shown_lines(&text, "src/orders/service.py"),
        numbered(SERVICE)
    );
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
    let text = reply(&files, work_order_request(prompt.clone()))?;
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
    // A held-out task's record stands however late its answer: nothing was to be given.
    let late = Event {
        prompt_text: Some(PROMPT.to_owned()),
        ..event(root, HookKind::Prompt, 31)?
    };
    let asked = RequestBody::Retrieve {
        event: Box::new(late.clone()),
        answer_by_ms: Some(0),
    };
    assert_eq!(reply(&files, asked)?, None);
    let [row] = &injections_of(root, &late)?[..] else {
        return Err("one injection".into());
    };
    assert!(row.record.is_holdout && row.record.text_hash.is_some());
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
    let primer = reply(&files, primer_request(start))?.ok_or("a primer")?;
    assert_eq!(primer, PRIMER);
    let compacted = Event {
        source: Some(StartSource::Compact),
        ..event(root, HookKind::SessionStart, 15)?
    };
    assert_eq!(reply(&files, primer_request(compacted))?, None);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mismatched = work_order_request(event(root, HookKind::SessionStart, 12)?);
    assert!(matches!(
        ask(&files, Role::Hook, mismatched, deadline)?.body,
        ResponseBody::Error { .. }
    ));
    let vague = Event {
        prompt_text: Some("make it better".to_owned()),
        ..event(root, HookKind::Prompt, 40)?
    };
    assert_eq!(reply(&files, work_order_request(vague))?, None);
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
    reply(&files, primer_request(other))?.ok_or("the other session's primer")?;
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

/// Through `cortex hook`, as Claude Code runs it: a session's start prints the primer, and the
/// start of a second session the same bytes; a prompt that names `create_order` prints its work
/// order, true to the file on disk; each is Claude Code's hook JSON, and each is recorded as given,
/// with the hash of the text printed.
#[test]
fn the_hook_prints_the_primer_and_the_work_order() -> TestResult {
    let (repo, home, _daemon) = repository("0")?;
    let (root, home) = (repo.path(), home.path());
    let start = captured("session-start-startup.json", root, &[])?;
    let primer = given("SessionStart", start, home)?.ok_or("a primer")?;
    assert_eq!(primer, PRIMER);
    let other = captured("session-start-startup.json", root, &[("session_id", "s2")])?;
    assert_eq!(given("SessionStart", other, home)?, Some(primer.clone()));
    let prompt = captured("user-prompt-submit.json", root, &[("prompt", PROMPT)])?;
    let order = given("UserPromptSubmit", prompt, home)?.ok_or("a work order")?;
    assert!(order.starts_with("[cortex] Work order · "), "{order}");
    assert!(
        order.contains("src/orders/service.py:1-3  create_order  (named in the prompt)"),
        "{order}"
    );
    assert_eq!(
        shown_lines(&order, "src/orders/service.py"),
        numbered(SERVICE)
    );
    let session = Identifier::new(S1)?;
    let rows = || -> TestResult<Vec<(HookKind, Option<ContentHash>)>> {
        let reader = Reader::open(&root.join(".cortex"), 0)?;
        let rows = reader.queries().injections_of_session(&session)?;
        Ok(rows
            .into_iter()
            .map(|row| (row.record.hook, row.record.text_hash))
            .collect())
    };
    wait_for(|| Ok(rows()?.len() >= 2))?;
    assert_eq!(
        rows()?,
        [
            (
                HookKind::SessionStart,
                Some(ContentHash::of(primer.as_bytes()))
            ),
            (HookKind::Prompt, Some(ContentHash::of(order.as_bytes()))),
        ]
    );
    Ok(())
}

/// With every task held out, the prompt through `cortex hook` prints nothing, and is recorded as
/// held out with the hash of the text it would have been given; a session's start still prints the
/// primer, which is a session's and never held out.
#[test]
fn the_hook_prints_nothing_for_a_held_out_prompt() -> TestResult {
    let (repo, home, _daemon) = repository("1")?;
    let (root, home) = (repo.path(), home.path());
    let start = captured("session-start-startup.json", root, &[])?;
    assert_eq!(given("SessionStart", start, home)?.as_deref(), Some(PRIMER));
    let prompt = captured("user-prompt-submit.json", root, &[("prompt", PROMPT)])?;
    assert_eq!(given("UserPromptSubmit", prompt, home)?, None);
    let session = Identifier::new(S1)?;
    let prompts = || -> TestResult<Vec<InjectionRow>> {
        let reader = Reader::open(&root.join(".cortex"), 0)?;
        let mut rows = reader.queries().injections_of_session(&session)?;
        rows.retain(|row| row.record.hook == HookKind::Prompt);
        Ok(rows)
    };
    wait_for(|| Ok(!prompts()?.is_empty()))?;
    let [row] = &prompts()?[..] else {
        return Err("one prompt's injection".into());
    };
    assert!(row.record.is_holdout && !row.record.is_gated_out);
    assert!(row.record.text_hash.is_some());
    Ok(())
}

/// A request for text whose hook reads the answer until the end of time gets its text, recorded as
/// given with its hash. One whose hook stopped reading long before (at the start of 1970) gets no
/// text, the work order's and the primer's alike, and what it would have given is recorded as not
/// delivered: no hash and no tokens, neither kept back nor held out. The daemon's latest errors
/// say so, once for each.
#[test]
fn an_answer_worked_out_too_late_gives_nothing() -> TestResult {
    let (repo, home, _daemon) = repository("0")?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let prompt = |ts_ms| -> TestResult<Event> {
        Ok(Event {
            prompt_text: Some(PROMPT.to_owned()),
            ..event(root, HookKind::Prompt, ts_ms)?
        })
    };
    let in_time = prompt(60)?;
    let text = reply(
        &files,
        RequestBody::Retrieve {
            event: Box::new(in_time.clone()),
            answer_by_ms: Some(i64::MAX),
        },
    )?
    .ok_or("a work order")?;
    let [row] = &injections_of(root, &in_time)?[..] else {
        return Err("one injection".into());
    };
    assert_eq!(row.record.text_hash, Some(ContentHash::of(text.as_bytes())));
    let late = prompt(70)?;
    let asked = RequestBody::Retrieve {
        event: Box::new(late.clone()),
        answer_by_ms: Some(0),
    };
    assert_eq!(reply(&files, asked)?, None);
    let [row] = &injections_of(root, &late)?[..] else {
        return Err("one injection".into());
    };
    assert!(!row.record.is_gated_out && !row.record.is_holdout);
    assert_eq!((row.record.text_hash, row.record.tokens), (None, 0));
    assert_eq!(
        row.record.files.first().and_then(|file| file.reason),
        Some(ReasonKind::Named)
    );
    let later = Identifier::new("s3")?;
    let start = Event {
        source: Some(StartSource::Startup),
        session_id: later.clone(),
        ..event(root, HookKind::SessionStart, 80)?
    };
    let asked = RequestBody::Primer {
        event: Box::new(start),
        answer_by_ms: Some(0),
    };
    assert_eq!(reply(&files, asked)?, None);
    let primers = || -> TestResult<Vec<InjectionRow>> {
        Ok(Reader::open(&root.join(".cortex"), 0)?
            .queries()
            .injections_of_session(&later)?)
    };
    wait_for(|| Ok(!primers()?.is_empty()))?;
    let [row] = &primers()?[..] else {
        return Err("one primer".into());
    };
    assert_eq!((row.record.text_hash, row.record.tokens), (None, 0));
    let deadline = Instant::now() + Duration::from_secs(10);
    let ResponseBody::Status(report) = ask(&files, Role::Cli, RequestBody::Status, deadline)?.body
    else {
        return Err("a status".into());
    };
    let errors: Vec<(&str, u64)> = report
        .recent_errors
        .iter()
        .map(|error| (error.message.as_str(), error.count))
        .collect();
    assert_eq!(
        errors,
        [("an answer was worked out after its hook stopped waiting", 2)]
    );
    Ok(())
}
