//! The note a check gets the moment before it runs again: when the same command line, in the same
//! working folder, ran earlier in the session and no source change was heard of from the moment
//! that run started, what that run gave. The moment before a check runs, the count of source
//! changes heard of is noted as its start; its result is kept in memory when the count is still
//! that (its exit code and the failing tests its output named, read as a failed check's map reads
//! them); the moment before the line runs again, that result is looked up, and the note given at
//! most twice a task. Only a line that ran a check Cortex tells apart is noted; a line that may
//! have written files counts as a change of its own before its result is taken (`Inbox`), so it is
//! never kept. A tiny repository gets nothing, as it gets no work order; nor does a held-out task,
//! or a session with no open task. The note is context only: the command runs as the agent asked.

use cortex_core::{Event, HookKind, ScaleMode, ShellCommand, ToolKind, Ulid};
use cortex_learn::{MAX_FAILED_TESTS, read_line_failure};
use cortex_retrieve::{EarlierRun, MAX_NAMED_FAILING, redundant_note};

use super::{Answer, Service, mode_of, open_task_of};
use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::once::ClaimedOnce;
use crate::runs::{RunKey, Runs};

/// The most notes one task is given: a third run of a line nothing changed is the agent's choice,
/// told twice already.
pub(crate) const MAX_NOTES_PER_TASK: u8 = 2;

impl Service {
    /// Takes in `runs` the result `event` of a shell command, `changes` the count of source
    /// changes heard of now (none when it cannot be trusted): the run is kept for the line when
    /// its start was noted at the same count and its outcome is known (an exit code, not
    /// interrupted); the line's earlier run is dropped otherwise. Nothing for a probe or a line
    /// that ran no check.
    pub(crate) fn note_run(&self, event: &Event, changes: Option<u64>, runs: &Runs) {
        if event.is_probe || event.hook != HookKind::PostTool || event.tool != Some(ToolKind::Shell)
        {
            return;
        }
        let Some(command) = event
            .tool_target
            .as_ref()
            .and_then(|target| target.command.as_ref())
        else {
            return;
        };
        if !self.is_check(command, event) {
            return;
        }
        if let Some(key) = RunKey::of(command, &event.cwd) {
            let run = self.earlier_run(command, event);
            runs.finished(&event.session_id, key, (changes, run));
        }
    }

    /// The note `event`, the moment before a shell command runs, gets, and what to record of it,
    /// the note claimed in `repeated` for the task; a check's start is noted in `runs` at
    /// `changes`, the count of source changes heard of so far. None when `hints.redundant_commands`
    /// is off (the one place the key is read: with no start noted, no run is ever kept), the count
    /// cannot be trusted, the line did not run in this folder earlier in the session or a change
    /// was heard of since, before the first index, in tiny mode, outside a task or in a held-out
    /// one, or when the task was given its [`MAX_NOTES_PER_TASK`] notes. A store that cannot be
    /// read gives nothing, and the error is logged.
    pub(crate) fn redundant_command_note(
        &self,
        event: &Event,
        changes: Option<u64>,
        (runs, repeated): (&Runs, &ClaimedOnce<(Ulid, u8)>),
    ) -> Answer {
        if !self.failures.is_redundant_commands || event.is_probe {
            return Answer::default();
        }
        let Some(command) = event.command_to_run() else {
            return Answer::default();
        };
        let Some((changes, key)) = changes.zip(RunKey::of(command, &event.cwd)) else {
            return Answer::default();
        };
        if self.is_check(command, event) {
            runs.started(&event.session_id, key, changes);
        }
        // The memory is looked at first: most commands ran no check earlier, and they get their
        // answer without a reader.
        let Some(run) = runs.unchanged_since(&event.session_id, key, changes) else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            if mode_of(queries)?.is_none_or(|mode| mode == ScaleMode::Tiny) {
                return Ok(Answer::default());
            }
            let task = open_task_of(queries, event)?;
            // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
            if task
                .as_ref()
                .is_some_and(|task| self.is_held_out_task(task))
            {
                return Ok(Answer::default());
            }
            // Outside a task nothing is said: the notes are counted, and recorded, by task.
            let Some(task_id) = task.map(|task| task.record.task_id) else {
                return Ok(Answer::default());
            };
            let Some(text) = redundant_note(command.as_str(), &run) else {
                return Ok(Answer::default());
            };
            let Some(slot) = claim_note(repeated, task_id) else {
                return Ok(Answer::default());
            };
            Ok(Answer {
                given: vec![Given::Injection(text_record(event, Some(task_id), &text))],
                claims: vec![Claim::Repeated(task_id, slot)],
                text: Some(text),
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a repeated check's note failed");
            Answer::default()
        })
    }

    /// Whether `command`, run in `event`'s folder, ran a check Cortex tells apart.
    fn is_check(&self, command: &ShellCommand, event: &Event) -> bool {
        !self
            .failures
            .commands
            .tell(command, &self.root, &event.cwd)
            .checks
            .is_empty()
    }

    /// What the run of `command` whose result `event` is gave: its exit code and, for a run that
    /// failed, the failing tests its output named; none when its outcome is unknown.
    fn earlier_run(&self, command: &ShellCommand, event: &Event) -> Option<EarlierRun> {
        let result = event.tool_result.filter(|result| !result.is_interrupted)?;
        let exit_code = result.exit_code?;
        let names = event
            .failed_output()
            .filter(|_| exit_code != 0)
            .and_then(|output| {
                read_line_failure(
                    &self.failures.commands,
                    command,
                    (&self.root, &event.cwd),
                    output.as_str(),
                )
            })
            .map(|read| read.failures.failed_tests)
            .unwrap_or_default();
        let (failing, failing_count) = named_failing(names);
        Some(EarlierRun {
            exit_code,
            failing,
            failing_count,
        })
    }
}

/// The failing tests a run's output named, `names`, as a note tells them: the first
/// [`MAX_NAMED_FAILING`] and how many in all; none when the parser's list may have been cut at
/// its [`MAX_FAILED_TESTS`], since how many more there were is then unknown.
fn named_failing(names: Vec<String>) -> (Vec<String>, usize) {
    if names.len() >= MAX_FAILED_TESTS {
        return (Vec::new(), 0);
    }
    let count = names.len();
    (names.into_iter().take(MAX_NAMED_FAILING).collect(), count)
}

/// Claims in `repeated` the next of task `task_id`'s [`MAX_NOTES_PER_TASK`] notes: its count from
/// 0, or none once the task was given them all.
fn claim_note(repeated: &ClaimedOnce<(Ulid, u8)>, task_id: Ulid) -> Option<u8> {
    (0..MAX_NOTES_PER_TASK).find(|slot| repeated.claim((task_id, *slot)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task is given two notes, its first and second, and no third; another task is given its
    /// own two; a note given back is given again.
    #[test]
    fn a_task_is_given_two_notes() {
        let repeated = ClaimedOnce::default();
        let (task, other) = (Ulid::from_seed(1, b"task"), Ulid::from_seed(2, b"task"));
        assert_eq!(claim_note(&repeated, task), Some(0));
        assert_eq!(claim_note(&repeated, task), Some(1));
        assert_eq!(claim_note(&repeated, task), None);
        assert_eq!(claim_note(&repeated, other), Some(0));
        repeated.release(&(task, 1));
        assert_eq!(claim_note(&repeated, task), Some(1));
        assert_eq!(claim_note(&repeated, task), None);
    }

    /// Of five failing names the first three are kept and five counted; of 19, three and 19; of
    /// 20, the parser's whole list, which may have been cut, none.
    #[test]
    fn a_list_that_may_have_been_cut_names_nothing() {
        let names =
            |count: usize| -> Vec<String> { (0..count).map(|at| format!("t{at}")).collect() };
        assert_eq!(
            named_failing(names(5)),
            (vec!["t0".to_owned(), "t1".to_owned(), "t2".to_owned()], 5)
        );
        assert_eq!(named_failing(names(19)).1, 19);
        assert_eq!(named_failing(names(19)).0.len(), 3);
        assert_eq!(MAX_FAILED_TESTS, 20);
        assert_eq!(named_failing(names(20)), (Vec::new(), 0));
    }
}
