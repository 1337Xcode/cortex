//! The checks each recent session ran and what each gave, for the note a check gets before it runs
//! again with no source change since. Kept in memory only, never stored: a run is known by the
//! hash of its command line and working folder, never by the line itself, with its exit code and
//! the first failing tests its output named.
//!
//! A run is kept only when nothing changed while it ran: the moment before a line runs, the count
//! of source changes the daemon has heard of is noted as the run's start; when its result comes,
//! the run is kept only if the count is still that, so that a file changed after the run read it
//! never passes for one it ran on. A result with no start noted keeps nothing. A later run of the
//! same line in the same folder takes the earlier one's place; one whose outcome is unknown, or
//! that changed while it ran, takes it away.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use cortex_core::{ContentHash, Event, HookKind, Identifier, MAX_COMMAND_CHARS, ShellCommand};
use cortex_retrieve::EarlierRun;

/// How many sessions' runs are kept, the one that ran a check longest ago dropped first: a few
/// sessions work at once, and one quiet for a few dozen others is not about to run its check again.
const MAX_SESSIONS: usize = 64;

/// How many runs of one session are kept, the oldest dropped first: a task runs a few checks over
/// and over, rarely more than a handful of different lines.
pub(crate) const MAX_RUNS_PER_SESSION: usize = 16;

/// How many starts of one session wait for their results, the oldest dropped first: an agent runs
/// its shell commands one after another, a few at most in the background.
pub(crate) const MAX_STARTS_PER_SESSION: usize = 8;

/// What identifies a run: the hash of its command line and its working folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunKey(ContentHash);

impl RunKey {
    /// The key of `command` run in `cwd`: none for a command as long as an event carries, which
    /// may have been cut, so that two lines that differ only past the cut are never taken for one.
    pub(crate) fn of(command: &ShellCommand, cwd: &Path) -> Option<Self> {
        if command.as_str().chars().count() >= MAX_COMMAND_CHARS {
            return None;
        }
        // The line and the folder are kept apart by a byte neither holds.
        let mut bytes = command.as_str().as_bytes().to_vec();
        bytes.push(0);
        bytes.extend_from_slice(cwd.as_os_str().as_encoded_bytes());
        Some(Self(ContentHash::of(&bytes)))
    }
}

/// One run kept: its key, the source changes heard of while it ran, and what it gave.
#[derive(Debug, Clone)]
struct Kept {
    /// The run's line and folder.
    key: RunKey,
    /// The count of source changes the daemon had heard of when it started, and still when its
    /// result came.
    changes: u64,
    /// What it gave.
    run: EarlierRun,
}

/// One session's runs and the starts waiting for their results, the oldest first.
#[derive(Debug)]
struct SessionRuns {
    /// The session.
    session: Identifier,
    /// Its runs, at most [`MAX_RUNS_PER_SESSION`].
    runs: VecDeque<Kept>,
    /// The lines about to run, each with the count of source changes heard of then, at most
    /// [`MAX_STARTS_PER_SESSION`].
    starts: VecDeque<(RunKey, u64)>,
}

/// The checks each recent session ran.
#[derive(Debug, Default)]
pub(crate) struct Runs {
    /// Each session's runs, the session that ran one longest ago first.
    sessions: Mutex<VecDeque<SessionRuns>>,
}

impl Runs {
    /// Notes that the line `key` is about to run for `session`, `changes` source changes heard of
    /// so far.
    pub(crate) fn started(&self, session: &Identifier, key: RunKey, changes: u64) {
        self.with_session(session, |own| {
            if own.starts.len() == MAX_STARTS_PER_SESSION {
                own.starts.pop_front();
            }
            own.starts.push_back((key, changes));
        });
    }

    /// Takes the result of the line `key` for `session`, `changes` source changes heard of now
    /// (none when the count cannot be trusted), and `run`, what it gave (none when that is
    /// unknown): the run is kept, in place of any earlier one, when its start was noted at the same
    /// count; otherwise the earlier run is dropped, since what the line gives now is unknown.
    pub(crate) fn finished(
        &self,
        session: &Identifier,
        key: RunKey,
        (changes, run): (Option<u64>, Option<EarlierRun>),
    ) {
        self.with_session(session, |own| {
            let start = own
                .starts
                .iter()
                .position(|(started, _)| *started == key)
                .and_then(|at| own.starts.remove(at))
                .map(|(_, at_start)| at_start);
            own.runs.retain(|kept| kept.key != key);
            // INVARIANT: silent-when-unsure (a run with no start noted, or during which a change
            // was heard of, is not kept)
            let Some((changes, run)) = changes.filter(|now| start == Some(*now)).zip(run) else {
                return;
            };
            if own.runs.len() == MAX_RUNS_PER_SESSION {
                own.runs.pop_front();
            }
            own.runs.push_back(Kept { key, changes, run });
        });
    }

    /// Drops what the shell command of `event`, a tool's result whose outcome the daemon is not
    /// told, gave earlier in its session, and its start: what the line gives now is unknown.
    pub(crate) fn forget_result_of(&self, event: &Event) {
        if event.hook != HookKind::PostTool {
            return;
        }
        let Some(command) = event
            .tool_target
            .as_ref()
            .and_then(|target| target.command.as_ref())
        else {
            return;
        };
        if let Some(key) = RunKey::of(command, &event.cwd) {
            self.finished(&event.session_id, key, (None, None));
        }
    }

    /// What the line `key` gave `session` when it last ran, if `changes` source changes had been
    /// heard of then, as now: none when it never ran, or any change was heard of since.
    pub(crate) fn unchanged_since(
        &self,
        session: &Identifier,
        key: RunKey,
        changes: u64,
    ) -> Option<EarlierRun> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let own = sessions.iter().find(|kept| kept.session == *session)?;
        let kept = own.runs.iter().find(|kept| kept.key == key)?;
        // INVARIANT: silent-when-unsure (a change heard of since the run, to any file, withholds
        // the run's result)
        (kept.changes == changes).then(|| kept.run.clone())
    }

    /// Runs `change` on `session`'s runs, which become the ones kept longest; a session left with
    /// nothing is no longer kept, and past [`MAX_SESSIONS`] the one changed longest ago goes.
    fn with_session(&self, session: &Identifier, change: impl FnOnce(&mut SessionRuns)) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let mut own = match sessions.iter().position(|kept| kept.session == *session) {
            Some(at) => sessions.remove(at),
            None => None,
        }
        .unwrap_or_else(|| SessionRuns {
            session: session.clone(),
            runs: VecDeque::new(),
            starts: VecDeque::new(),
        });
        change(&mut own);
        if own.runs.is_empty() && own.starts.is_empty() {
            return;
        }
        if sessions.len() == MAX_SESSIONS {
            sessions.pop_front();
        }
        sessions.push_back(own);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A run that exited `exit_code`, naming no test.
    fn exited(exit_code: i32) -> EarlierRun {
        EarlierRun {
            exit_code,
            failing: Vec::new(),
            failing_count: 0,
        }
    }

    /// The key of `line` run in `/repo`.
    fn key(line: &str) -> Result<RunKey, String> {
        RunKey::of(&ShellCommand::cut(line), Path::new("/repo")).ok_or(format!("no key: {line}"))
    }

    /// Runs `line` for `session` as the daemon hears it: started, then finished with `run`, both
    /// at `changes`.
    fn ran(runs: &Runs, session: &Identifier, line: &str, (changes, run): (u64, EarlierRun)) {
        if let Ok(key) = key(line) {
            runs.started(session, key, changes);
            runs.finished(session, key, (Some(changes), Some(run)));
        }
    }

    /// A run is found by its session, line and folder, while the changes heard of are those of
    /// its run; another line, folder or session finds nothing; a later run takes its place, and
    /// a run of unknown outcome takes it away.
    #[test]
    fn a_run_is_found_while_nothing_changed() -> Result<(), Box<dyn std::error::Error>> {
        let runs = Runs::default();
        let (one, two) = (Identifier::new("s1")?, Identifier::new("s2")?);
        let pytest = key("pytest -q")?;
        ran(&runs, &one, "pytest -q", (4, exited(1)));
        assert_eq!(runs.unchanged_since(&one, pytest, 4), Some(exited(1)));
        assert_eq!(runs.unchanged_since(&one, pytest, 5), None);
        assert_eq!(runs.unchanged_since(&two, pytest, 4), None);
        assert_eq!(runs.unchanged_since(&one, key("pytest")?, 4), None);
        let elsewhere = RunKey::of(&ShellCommand::cut("pytest -q"), Path::new("/repo/sub"));
        assert_ne!(elsewhere, Some(pytest));
        ran(&runs, &one, "pytest -q", (5, exited(0)));
        assert_eq!(runs.unchanged_since(&one, pytest, 5), Some(exited(0)));
        runs.started(&one, pytest, 5);
        runs.finished(&one, pytest, (Some(5), None));
        assert_eq!(runs.unchanged_since(&one, pytest, 5), None);
        Ok(())
    }

    /// A run during which a change was heard of (started at 3, its result at 4) is not kept, and
    /// takes the earlier run's place all the same; nor is a result whose start was never noted, or
    /// one that came when the count could not be trusted.
    #[test]
    fn a_run_that_changed_while_it_ran_is_not_kept() -> Result<(), Box<dyn std::error::Error>> {
        let runs = Runs::default();
        let session = Identifier::new("s1")?;
        let pytest = key("pytest -q")?;
        ran(&runs, &session, "pytest -q", (3, exited(1)));
        runs.started(&session, pytest, 3);
        runs.finished(&session, pytest, (Some(4), Some(exited(1))));
        assert_eq!(runs.unchanged_since(&session, pytest, 3), None);
        assert_eq!(runs.unchanged_since(&session, pytest, 4), None);
        runs.finished(&session, pytest, (Some(4), Some(exited(1))));
        assert_eq!(runs.unchanged_since(&session, pytest, 4), None);
        runs.started(&session, pytest, 4);
        runs.finished(&session, pytest, (None, Some(exited(1))));
        assert_eq!(runs.unchanged_since(&session, pytest, 4), None);
        Ok(())
    }

    /// A line as long as an event carries may have been cut, and has no key.
    #[test]
    fn a_line_that_may_have_been_cut_has_no_key() {
        let cwd = Path::new("/repo");
        let longest = "x".repeat(MAX_COMMAND_CHARS - 1);
        assert!(RunKey::of(&ShellCommand::cut(&longest), cwd).is_some());
        let cut = "x".repeat(MAX_COMMAND_CHARS + 5);
        assert_eq!(RunKey::of(&ShellCommand::cut(&cut), cwd), None);
    }

    /// A session keeps 16 runs, the oldest dropped first, and 8 starts, the oldest dropped first;
    /// 64 sessions are kept, the one that ran a check longest ago dropped first, and a session
    /// that runs one again is kept longer.
    #[test]
    fn the_memory_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let runs = Runs::default();
        let first = Identifier::new("s0")?;
        for at in 0..=MAX_RUNS_PER_SESSION {
            ran(&runs, &first, &format!("pytest -k t{at}"), (0, exited(1)));
        }
        assert_eq!(runs.unchanged_since(&first, key("pytest -k t0")?, 0), None);
        assert_eq!(
            runs.unchanged_since(&first, key("pytest -k t1")?, 0),
            Some(exited(1))
        );
        let lines: Vec<String> = (0..=MAX_STARTS_PER_SESSION)
            .map(|at| format!("cargo test -p c{at}"))
            .collect();
        for line in &lines {
            runs.started(&first, key(line)?, 0);
        }
        for line in &lines {
            runs.finished(&first, key(line)?, (Some(0), Some(exited(101))));
        }
        let kept = |line: &str| runs.unchanged_since(&first, key(line).ok()?, 0);
        assert_eq!(kept("cargo test -p c0"), None);
        assert_eq!(kept("cargo test -p c1"), Some(exited(101)));
        let second = Identifier::new("s1")?;
        ran(&runs, &second, "cargo test", (0, exited(101)));
        // The first session runs again, so the second is now the oldest.
        ran(&runs, &first, "pytest -k t1", (0, exited(1)));
        for at in 2..MAX_SESSIONS {
            let session = Identifier::new(&format!("s{at}"))?;
            ran(&runs, &session, "pytest", (0, exited(0)));
        }
        assert_eq!(
            runs.unchanged_since(&second, key("cargo test")?, 0),
            Some(exited(101))
        );
        let last = Identifier::new(&format!("s{MAX_SESSIONS}"))?;
        ran(&runs, &last, "pytest", (0, exited(0)));
        assert_eq!(runs.unchanged_since(&second, key("cargo test")?, 0), None);
        assert_eq!(
            runs.unchanged_since(&first, key("pytest -k t1")?, 0),
            Some(exited(1))
        );
        Ok(())
    }

    /// A shell's result the daemon is not told the outcome of drops the line's earlier run and its
    /// start; the moment before a command runs drops nothing.
    #[test]
    fn a_result_of_unknown_outcome_drops_the_run() -> Result<(), Box<dyn std::error::Error>> {
        let runs = Runs::default();
        let session = Identifier::new("s1")?;
        let pytest = key("pytest -q")?;
        ran(&runs, &session, "pytest -q", (0, exited(1)));
        let mut event = Event {
            event_id: cortex_core::Ulid::from_seed(1, b"e"),
            ts_ms: 1,
            agent: cortex_core::Agent::Claude,
            agent_version: None,
            session_id: session.clone(),
            parent_session_id: None,
            cwd: PathBuf::from("/repo"),
            repo_root: PathBuf::from("/repo"),
            worktree_root: None,
            hook: HookKind::PreTool,
            source: None,
            tool: Some(cortex_core::ToolKind::Shell),
            tool_target: Some(cortex_core::ToolTarget {
                command: Some(ShellCommand::cut("pytest -q")),
                ..cortex_core::ToolTarget::default()
            }),
            tool_result: None,
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        };
        runs.forget_result_of(&event);
        assert_eq!(runs.unchanged_since(&session, pytest, 0), Some(exited(1)));
        runs.started(&session, pytest, 0);
        event.hook = HookKind::PostTool;
        runs.forget_result_of(&event);
        assert_eq!(runs.unchanged_since(&session, pytest, 0), None);
        // The start went with it: a result arriving now has none to match.
        runs.finished(&session, pytest, (Some(0), Some(exited(1))));
        assert_eq!(runs.unchanged_since(&session, pytest, 0), None);
        Ok(())
    }
}
