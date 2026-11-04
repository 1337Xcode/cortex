//! The work tree's uncommitted changes through a real daemon: what its status says of them, and how
//! a prompt about a changed file gets its work order from them where the words alone get nothing.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::git::{commit, git};
use common::{Daemon, TestResult, graph_of, wait_for, worktree_of};
use cortex_core::ipc::{RequestBody, ResponseBody, Role, WorktreeState};
use cortex_core::{
    Agent, Event, HookKind, Identifier, ReasonKind, RepoId, RunFiles, Ulid, UserFolder,
    canonical_root, task_id_of_prompt,
};
use cortex_hook::ask;
use cortex_store::{InjectionRow, Reader};

/// `src/orders/service.py`, its function on lines 1 to 3.
const SERVICE: &str = "def create_order(cart):\n    total = sum(cart)\n    return total\n";

/// `src/orders/model.py`.
const MODEL: &str = "class Order:\n    def __init__(self):\n        self.lines = []\n";

/// `src/util.py`, which no word of the prompt matches.
const UTIL: &str = "def clamp(value, low, high):\n    return max(low, min(value, high))\n";

/// The prompt: it names `create_order`, and "orders" is in two paths.
const PROMPT: &str = "fix create_order in the orders service";

/// A repository whose three files were committed once, so that no history ties them, pinned to
/// medium mode with no task held out; its home folder; and its daemon, once the graph is built and
/// the work tree looked at.
fn repository() -> TestResult<(tempfile::TempDir, tempfile::TempDir, Daemon)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    git(root, home.path(), &["init", "-q"])?;
    std::fs::create_dir_all(root.join("src/orders"))?;
    std::fs::write(root.join("src/orders/service.py"), SERVICE)?;
    std::fs::write(root.join("src/orders/model.py"), MODEL)?;
    std::fs::write(root.join("src/util.py"), UTIL)?;
    commit(
        root,
        home.path(),
        "one",
        &[
            "src/orders/service.py",
            "src/orders/model.py",
            "src/util.py",
        ],
    )?;
    std::fs::create_dir_all(root.join(".cortex"))?;
    std::fs::write(
        root.join(".cortex/config.toml"),
        "mode = \"medium\"\n[holdout]\nshare = 0\n",
    )?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    wait_for(|| Ok(worktree_of(root, home.path())?.is_some()))?;
    Ok((repo, home, daemon))
}

/// The run files of the daemon of the repository at `root`.
fn run_files(root: &Path, home: &Path) -> TestResult<RunFiles> {
    Ok(UserFolder::in_home(home).run_files(&RepoId::of(&canonical_root(root)?)))
}

/// A prompt asking `PROMPT` in session `s1` at `ts_ms`.
fn prompt(root: &Path, ts_ms: i64) -> TestResult<Event> {
    let root = canonical_root(root)?;
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"worktree"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: root.clone(),
        repo_root: root,
        hook: HookKind::Prompt,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: Some(PROMPT.to_owned()),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// The work order the daemon answers `event` with, if any.
fn work_order(files: &RunFiles, event: &Event) -> TestResult<Option<String>> {
    let body = RequestBody::Retrieve {
        event: Box::new(event.clone()),
        answer_by_ms: None,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    match ask(files, Role::Hook, body, deadline)?.body {
        ResponseBody::Reply { text } => Ok(text),
        other => Err(format!("{other:?}").into()),
    }
}

/// The injection of the task `prompt` began, once it is recorded.
fn injection_of(root: &Path, prompt: &Event) -> TestResult<InjectionRow> {
    let task = task_id_of_prompt(prompt.event_id, prompt.ts_ms);
    let read = || -> TestResult<Vec<InjectionRow>> {
        Ok(Reader::open(&root.join(".cortex"), 0)?
            .queries()
            .injections_of_task(task)?)
    };
    wait_for(|| Ok(!read()?.is_empty()))?;
    read()?
        .into_iter()
        .next()
        .ok_or_else(|| "an injection".into())
}

/// With the work tree clean, the status says so, and the prompt gets nothing: only the words point
/// at the service, one kind of evidence, and nothing says the work is under way. Once the service
/// and `util.py` are changed and not committed, the status counts the two, and the same prompt
/// gets its work order: the service is changed and the words match it, so the work continues. The
/// service is named in the prompt, and `util.py`, which no word matches, is listed as an
/// uncommitted change, as its row of the injection says too.
#[test]
fn a_prompt_about_a_changed_file_continues_its_work() -> TestResult {
    let (repo, home, _daemon) = repository()?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let clean = worktree_of(root, home.path())?.ok_or("a look")?;
    assert_eq!(clean.state, WorktreeState::Read);
    assert_eq!(clean.uncommitted_files, 0);
    assert_eq!(work_order(&files, &prompt(root, 10)?)?, None);
    std::fs::write(
        root.join("src/orders/service.py"),
        format!("{SERVICE}# changed\n"),
    )?;
    std::fs::write(root.join("src/util.py"), format!("{UTIL}# changed\n"))?;
    wait_for(|| {
        Ok(worktree_of(root, home.path())?.is_some_and(|report| report.uncommitted_files == 2))
    })?;
    let asked = prompt(root, 20)?;
    let text = work_order(&files, &asked)?.ok_or("a work order")?;
    assert!(
        text.starts_with("[cortex] Work order · continuing the work under way\n"),
        "{text}"
    );
    assert!(
        text.contains("src/orders/service.py:1-3  create_order  (named in the prompt)"),
        "{text}"
    );
    assert!(text.contains("src/util.py"), "{text}");
    assert!(text.contains("(an uncommitted change)"), "{text}");
    let row = injection_of(root, &asked)?;
    let reasons: Vec<Option<ReasonKind>> =
        row.record.files.iter().map(|file| file.reason).collect();
    assert!(
        reasons.contains(&Some(ReasonKind::Uncommitted)),
        "{reasons:?}"
    );
    assert!(
        row.record
            .features
            .is_some_and(|features| features.is_continuing)
    );
    Ok(())
}

/// A commit empties the diff though it changes no file in the work tree: once the changed service
/// seeds retrievals, committing it leaves no uncommitted file, none listed and none seeding, as a
/// look the daemon read after the commit says.
#[test]
fn a_commit_empties_the_diff() -> TestResult {
    let (repo, home, _daemon) = repository()?;
    let root = repo.path();
    std::fs::write(
        root.join("src/orders/service.py"),
        format!("{SERVICE}# changed\n"),
    )?;
    wait_for(|| read_with(root, home.path(), (1, 1)))?;
    commit(root, home.path(), "two", &["src/orders/service.py"])?;
    wait_for(|| read_with(root, home.path(), (0, 0)))?;
    Ok(())
}

/// A repository whose own git configuration comes to name a filter is not looked at again, and
/// the files a look kept before are let go: once the changed service is kept, a filter is named
/// in `.git/config` and another file changed, which asks for a look; the status then says the
/// look was refused, with no file kept, and the prompt gets nothing, as with a clean work tree.
#[test]
fn a_filter_named_later_lets_the_diff_go() -> TestResult {
    let (repo, home, _daemon) = repository()?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    std::fs::write(
        root.join("src/orders/service.py"),
        format!("{SERVICE}# changed\n"),
    )?;
    wait_for(|| read_with(root, home.path(), (1, 1)))?;
    git(
        root,
        home.path(),
        &["config", "filter.marking.clean", "cat"],
    )?;
    std::fs::write(root.join("src/util.py"), format!("{UTIL}# changed\n"))?;
    wait_for(|| {
        Ok(worktree_of(root, home.path())?
            .is_some_and(|report| report.state == WorktreeState::Refused))
    })?;
    let refused = worktree_of(root, home.path())?.ok_or("a look")?;
    assert_eq!((refused.listed_files, refused.uncommitted_files), (0, 0));
    assert_eq!(work_order(&files, &prompt(root, 30)?)?, None);
    Ok(())
}

/// Whether the daemon of the repository at `root` last read the diff with `(listed, kept)` files.
fn read_with(root: &Path, home: &Path, (listed, kept): (u64, u64)) -> TestResult<bool> {
    Ok(worktree_of(root, home)?.is_some_and(|report| {
        report.state == WorktreeState::Read
            && (report.listed_files, report.uncommitted_files) == (listed, kept)
    }))
}
