//! Edits only the file watcher saw, with a real daemon: one made while a session works goes to
//! that session's task once its hold is over; one made with no session about goes to nobody; one a
//! hook reported is not recorded a second time.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{Daemon, TestResult, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, ContentHash, Event, EventKind, EventSource, HookKind, Identifier, RepoId, RepoPath,
    RunFiles, ToolKind, ToolTarget, Ulid, UserFolder, canonical_root,
};
use cortex_hook::ask;
use cortex_store::Reader;

/// The time now in UTC milliseconds: the daemon weighs hook events' times against when its watcher
/// saw a change, so the events here carry the real time.
fn now_ms() -> TestResult<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

/// An edit row as compared here: its file, session, task and source.
type Edit = (String, Option<String>, Option<Ulid>, Option<EventSource>);

/// A repository of two Python files, an empty home folder, and its daemon, ready. Fields drop in
/// order: the daemon stops and lets go of the repository's files before the folders are removed,
/// which Windows refuses while they are open.
struct Repo {
    /// The daemon, stopped when the test ends.
    _daemon: Daemon,
    /// The repository, deleted when the test ends.
    dir: tempfile::TempDir,
    /// The home folder, deleted when the test ends.
    _home: tempfile::TempDir,
    /// The repository's root, as the daemon names it.
    root: PathBuf,
    /// The daemon's run files.
    files: RunFiles,
}

impl Repo {
    /// A new repository and its daemon, once the daemon says it is ready.
    fn start() -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        std::fs::write(dir.path().join("app.py"), "def app():\n    pass\n")?;
        std::fs::write(dir.path().join("lib.py"), "def lib():\n    pass\n")?;
        let root = canonical_root(dir.path())?;
        let daemon = Daemon::start(&root, home.path())?;
        let line = daemon.first_line()?;
        assert!(line.starts_with("cortex: daemon ready"), "{line}");
        let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&root));
        Ok(Self {
            dir,
            _home: home,
            root,
            files,
            _daemon: daemon,
        })
    }

    /// An event of `hook` from `session` at `ts_ms`, nothing else known.
    fn event(&self, hook: HookKind, session: &str, ts_ms: i64) -> TestResult<Event> {
        Ok(Event {
            event_id: Ulid::from_seed(u64::try_from(ts_ms)?, session.as_bytes()),
            ts_ms,
            agent: Agent::Claude,
            agent_version: None,
            session_id: Identifier::new(session)?,
            parent_session_id: None,
            cwd: self.root.clone(),
            repo_root: self.root.clone(),
            worktree_root: None,
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

    /// Sends `event` to the daemon, which must take it.
    fn send(&self, event: Event) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(10);
        let body = RequestBody::Event {
            event: Box::new(event),
        };
        assert_eq!(
            ask(&self.files, Role::Hook, body, deadline)?.body,
            ResponseBody::Ack
        );
        Ok(())
    }

    /// Writes `text` to the file at `path` in the repository, as an editor would.
    fn write(&self, path: &str, text: &str) -> TestResult {
        Ok(std::fs::write(self.dir.path().join(path), text)?)
    }

    /// Whether the store holds the file at `path` with the contents `text`: the daemon has seen
    /// and indexed that change.
    fn holds(&self, path: &str, text: &str) -> TestResult<bool> {
        let Ok(reader) = Reader::open(&self.dir.path().join(".cortex"), 0) else {
            return Ok(false);
        };
        let row = reader.queries().file(&RepoPath::new(path)?)?;
        Ok(row.is_some_and(|row| row.file.content_hash == Some(ContentHash::of(text.as_bytes()))))
    }

    /// The edit rows stored, in order.
    fn edits(&self) -> TestResult<Vec<Edit>> {
        let Ok(reader) = Reader::open(&self.dir.path().join(".cortex"), 0) else {
            return Ok(Vec::new());
        };
        Ok(reader
            .queries()
            .events_after(0, 1_000)?
            .into_iter()
            .map(|row| row.record)
            .filter(|record| record.kind == EventKind::Edit)
            .map(|record| {
                (
                    record.path.map(|path| path.to_string()).unwrap_or_default(),
                    record.session_id.map(|session| session.as_str().to_owned()),
                    record.task_id,
                    record.meta.source,
                )
            })
            .collect())
    }
}

/// The id of the task the prompt `event` starts: made from the prompt's time and its event id's
/// text, as the recorder makes it.
fn task_of(event: &Event) -> TestResult<Ulid> {
    Ok(Ulid::from_seed(
        u64::try_from(event.ts_ms)?,
        event.event_id.to_string().as_bytes(),
    ))
}

/// While `s1` works on a task, an edit of `lib.py` that no hook reports, as an editor or an agent
/// without hooks would make it, is recorded as the watcher's, for `s1`, in its task.
#[test]
fn an_edit_no_hook_reported_goes_to_the_working_session() -> TestResult {
    let repo = Repo::start()?;
    let now = now_ms()?;
    let asked = Event {
        prompt_text: Some("tidy the library".to_owned()),
        ..repo.event(HookKind::Prompt, "s1", now + 1)?
    };
    let task = task_of(&asked)?;
    repo.send(repo.event(HookKind::SessionStart, "s1", now)?)?;
    repo.send(asked)?;
    repo.write("lib.py", "def lib():\n    return 1\n")?;
    wait_for(|| Ok(!repo.edits()?.is_empty()))?;
    let watched = Some(EventSource::Watcher);
    assert_eq!(
        repo.edits()?,
        [(
            "lib.py".to_owned(),
            Some("s1".to_owned()),
            Some(task),
            watched
        )]
    );
    Ok(())
}

/// With no session at work, an edit is recorded as the watcher's, for nobody.
#[test]
fn an_edit_with_no_session_about_goes_to_nobody() -> TestResult {
    let repo = Repo::start()?;
    repo.write("lib.py", "def lib():\n    return 2\n")?;
    wait_for(|| Ok(!repo.edits()?.is_empty()))?;
    let watched = Some(EventSource::Watcher);
    assert_eq!(repo.edits()?, [("lib.py".to_owned(), None, None, watched)]);
    Ok(())
}

/// `s1`'s hook reports its edit of `app.py`, made at once on disk: the watcher's sight of it is the
/// same edit and adds nothing. `lib.py`, written right after with no hook's word, is recorded as
/// the watcher's; its edit falls due no earlier than `app.py`'s, so once it is stored the daemon
/// has decided `app.py`'s too.
#[test]
fn an_edit_a_hook_reported_is_recorded_once() -> TestResult {
    let repo = Repo::start()?;
    let now = now_ms()?;
    let edited = Event {
        tool: Some(ToolKind::Edit),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new("app.py")?),
            ..ToolTarget::default()
        }),
        ..repo.event(HookKind::PostTool, "s1", now + 1)?
    };
    repo.send(repo.event(HookKind::SessionStart, "s1", now)?)?;
    repo.send(edited)?;
    repo.write("app.py", "def app():\n    return 3\n")?;
    repo.write("lib.py", "def lib():\n    return 3\n")?;
    wait_for(|| {
        Ok(repo
            .edits()?
            .iter()
            .any(|(path, ..)| path.as_str() == "lib.py"))
    })?;
    let s1 = Some("s1".to_owned());
    assert_eq!(
        repo.edits()?,
        [
            ("app.py".to_owned(), s1.clone(), None, None),
            ("lib.py".to_owned(), s1, None, Some(EventSource::Watcher)),
        ]
    );
    Ok(())
}

/// A hook's report that comes after the watcher saw the edit still claims it, within the 2 s the
/// edit is held: `app.py` is written, the daemon indexes the change, and only then does `s1`'s
/// hook report its edit. One row is recorded, the hook's. `lib.py`, written next with no hook's
/// word, falls due after `app.py`, so once its row is stored `app.py`'s edit has been decided.
#[test]
fn a_hook_s_late_report_claims_the_edit() -> TestResult {
    let repo = Repo::start()?;
    repo.send(repo.event(HookKind::SessionStart, "s1", now_ms()?)?)?;
    let text = "def app():\n    return 4\n";
    repo.write("app.py", text)?;
    wait_for(|| repo.holds("app.py", text))?;
    let edited = Event {
        tool: Some(ToolKind::Edit),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new("app.py")?),
            ..ToolTarget::default()
        }),
        ..repo.event(HookKind::PostTool, "s1", now_ms()?)?
    };
    repo.send(edited)?;
    repo.write("lib.py", "def lib():\n    return 4\n")?;
    wait_for(|| {
        Ok(repo
            .edits()?
            .iter()
            .any(|(path, ..)| path.as_str() == "lib.py"))
    })?;
    let s1 = Some("s1".to_owned());
    assert_eq!(
        repo.edits()?,
        [
            ("app.py".to_owned(), s1.clone(), None, None),
            ("lib.py".to_owned(), s1, None, Some(EventSource::Watcher)),
        ]
    );
    Ok(())
}
