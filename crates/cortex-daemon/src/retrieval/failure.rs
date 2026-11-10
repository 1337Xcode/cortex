//! What a failed check gets beside its output, worked out with a reader of the store: the output
//! is read for what failed by the parser of the check's runner, its stack frames are grounded in
//! the repository, and the failure map names the lines they ran through when the output does not
//! show them. Only a check the agent ran is read, never one Cortex runs; a failure that reaches no
//! repository frame (an import, environment or dependency error) gets nothing. The output is read
//! here, in memory, and dropped with the event; what is recorded is the map's size and the files it
//! named, as for any text given.

use cortex_core::{Event, ScaleMode};
use cortex_learn::parse_failures;
use cortex_retrieve::{FailureQuery, failure_map, ground_failure};
use cortex_store::{InjectedFile, InjectionRecord};

use super::{Answer, Service, mode_of, open_task_of};
use crate::given::{Given, text_record};

impl Service {
    /// The failure map `event`, a failed shell command's result, gets, and what to record of it:
    /// none when `hints.runtime_grounding` is off, for a command that is no check of a runner with
    /// a parser, before the first index, in tiny mode, in a held-out task, or when the map would
    /// add nothing to the output. A store that cannot be read gives nothing, and the error is
    /// logged.
    pub(crate) fn after_failure(&self, event: &Event) -> Answer {
        if !self.failures.is_runtime_grounding {
            return Answer::default();
        }
        let Some((command, output)) = event
            .tool_target
            .as_ref()
            .and_then(|target| target.command.as_ref().zip(target.failed_output.as_ref()))
        else {
            return Answer::default();
        };
        // The output is read by the parser of the check the line's exit is the failure of, else of
        // each check the line ran: `cd x && pytest` and `pytest | tail` print pytest's output.
        let told = self.failures.commands.tell(command, &self.root, &event.cwd);
        let Some(failures) = told
            .failure
            .iter()
            .chain(&told.checks)
            .find_map(|check| parse_failures(check.runner, output.as_str()))
        else {
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
            let grounded = ground_failure(
                queries,
                &failures,
                &FailureQuery {
                    root: &self.root,
                    cwd: &event.cwd,
                    max_file_bytes: self.max_file_bytes,
                },
            )?;
            let Some(map) = failure_map(queries, &grounded, (output.as_str(), task_id))? else {
                return Ok(Answer::default());
            };
            let files = map
                .files
                .iter()
                .map(|node| InjectedFile {
                    node: *node,
                    spans: Vec::new(),
                    reason: None,
                    shown: Vec::new(),
                    content: None,
                })
                .collect();
            let record = InjectionRecord {
                files,
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
}
