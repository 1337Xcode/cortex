//! What a failed check gets beside its output, worked out with a reader of the store: the output
//! is read for what failed by the parser of the check's runner, its stack frames are grounded in
//! the repository, and the failure map names the lines they ran through when the output does not
//! show them. The grounded frames are kept in memory as the session's latest failure, which its
//! next prompt seeds from; a later run of the same runner that passes drops them. Only a check the
//! agent ran is read, never one Cortex runs; a failure that reaches no repository frame (an
//! import, environment or dependency error) gets nothing and seeds nothing. The output is read
//! here, in memory, and dropped with the event; what is recorded is the map's size and the files
//! it named, as for any text given.

use cortex_core::{Event, FailedOutput, ScaleMode, ShellCommand};
use cortex_learn::{Failures, Ran, read_line_failure};
use cortex_retrieve::{FailureQuery, GroundedFailure, failure_map, ground_failure};
use cortex_store::{InjectedFile, InjectionRecord, NodeId};

use super::{Answer, Service, mode_of, open_task_of};
use crate::failed::Failed;
use crate::given::{Given, text_record};

/// A failed check's output as read: what failed, and the runners the line ran.
struct ReadFailure<'e> {
    /// What the output says failed.
    failures: Failures,
    /// The output itself, to tell what the map would add to it.
    output: &'e FailedOutput,
    /// The runners the line ran, whose passing run drops the failure.
    runners: Vec<&'static str>,
}

impl Service {
    /// The failure map `event`, a failed shell command's result, gets, and what to record of it,
    /// its grounded frames kept in `failed` as the session's latest failure: none when
    /// `hints.runtime_grounding` is off, for a command that is no check of a runner with a parser,
    /// before the first index, in tiny mode, in a held-out task, or when the map would add nothing
    /// to the output. A store that cannot be read gives nothing, and the error is logged.
    pub(crate) fn after_failure(&self, event: &Event, failed: &Failed) -> Answer {
        let Some(read) = self.read_failure(event) else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            let Some(mode) = mode_of(queries)? else {
                return Ok(Answer::default());
            };
            if mode == ScaleMode::Tiny {
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
            let task_id = task.map(|task| task.record.task_id);
            let query = FailureQuery {
                root: &self.root,
                cwd: &event.cwd,
                max_file_bytes: self.max_file_bytes,
            };
            let grounded = ground_failure(queries, &read.failures, &query)?;
            if !grounded.frames.is_empty() {
                let runners = read.runners.clone();
                failed.keep(
                    &event.session_id,
                    event.ts_ms,
                    (runners, seeds_of(&grounded)),
                );
            }
            let Some(map) = failure_map(queries, &grounded, (read.output.as_str(), task_id))?
            else {
                return Ok(Answer::default());
            };
            let record = InjectionRecord {
                files: map.files.iter().map(|node| named(*node)).collect(),
                ..text_record(event, task_id, &map.text)
            };
            Ok(Answer {
                text: Some(map.text),
                given: vec![Given::Injection(record)],
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a failed check's map failed");
            Answer::default()
        })
    }

    /// The seeds `event`, a prompt, takes from its session's latest failure in `failed`: none when
    /// `hints.runtime_grounding` is off. Taken either way, so that a failure seeds one prompt.
    pub(crate) fn failure_seeds(&self, event: &Event, failed: &Failed) -> Vec<NodeId> {
        let seeds = failed.take(&event.session_id, event.ts_ms);
        if self.failures.is_runtime_grounding {
            seeds
        } else {
            Vec::new()
        }
    }

    /// Drops the latest failure of `event`'s session from `failed` when `event` is a shell
    /// command's result that passed and its line ran one of that failure's runners.
    pub(crate) fn note_passed(&self, event: &Event, failed: &Failed) {
        if !self.failures.is_runtime_grounding {
            return;
        }
        let Some(command) = passed_command(event) else {
            return;
        };
        let ran = self
            .failures
            .commands
            .recognise(command, &self.root, &event.cwd);
        let runners: Vec<&str> = ran
            .iter()
            .filter_map(|ran| match ran {
                Ran::Check(check) => Some(check.runner),
                Ran::Restore(_) => None,
            })
            .collect();
        if !runners.is_empty() {
            failed.passed(&event.session_id, &runners);
        }
    }

    /// The output of `event`, a failed shell command's result, read by the parser of the check
    /// the line's exit is the failure of, else of each check the line ran (`cd x && pytest` and
    /// `pytest | tail` print pytest's output); none when `hints.runtime_grounding` is off, the
    /// event carries no output, or no parser finds a failure in it.
    fn read_failure<'e>(&self, event: &'e Event) -> Option<ReadFailure<'e>> {
        if !self.failures.is_runtime_grounding {
            return None;
        }
        let target = event.tool_target.as_ref()?;
        let (command, output) = target.command.as_ref().zip(target.failed_output.as_ref())?;
        let read = read_line_failure(
            &self.failures.commands,
            command,
            (&self.root, &event.cwd),
            output.as_str(),
        )?;
        Some(ReadFailure {
            failures: read.failures,
            output,
            runners: read.runners,
        })
    }
}

/// The command of `event` when it is a shell command's result that passed.
fn passed_command(event: &Event) -> Option<&ShellCommand> {
    let is_passed = event.tool_result.is_some_and(|result| result.is_success);
    if !is_passed {
        return None;
    }
    event.tool_target.as_ref()?.command.as_ref()
}

/// The nodes `failure`'s frames seed: each frame's innermost definition, or its file when no
/// definition holds the line.
fn seeds_of(failure: &GroundedFailure) -> Vec<NodeId> {
    failure
        .frames
        .iter()
        .map(|frame| frame.symbol.as_ref().map_or(frame.file, |(node, _)| *node))
        .collect()
}

/// A file a failure map named, as recorded with what was given.
fn named(node: NodeId) -> InjectedFile {
    InjectedFile {
        node,
        spans: Vec::new(),
        reason: None,
        shown: Vec::new(),
        content: None,
    }
}
