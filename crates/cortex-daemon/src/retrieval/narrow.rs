//! The hint a whole test-suite run gets after it finished: the test files of what the task edited,
//! and the narrow command that runs only them, once a task. Worked out with a reader of the store;
//! nothing is stored but what was given, as for any text.

use cortex_core::{Event, ScaleMode, Ulid};
use cortex_learn::{narrow_command, whole_suite};
use cortex_retrieve::{narrow_hint, narrow_tests};
use cortex_store::{InjectedFile, InjectionRecord};

use super::{Answer, Service, mode_of, open_task_of};
use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::once::ClaimedOnce;

impl Service {
    /// The hint `event`, a shell command's result, gets when its line ran a whole test suite, the
    /// task claimed in `narrowed` so that it gets one: none when `hints.narrow_tests` is off, the
    /// line selected tests or ran none, before the first index, in tiny mode, outside a task or in
    /// a held-out one, when the task was given its hint already, or edited nothing with tests. A
    /// store that cannot be read gives nothing, and the error is logged.
    pub(crate) fn after_suite(&self, event: &Event, narrowed: &ClaimedOnce<Ulid>) -> Answer {
        if !self.failures.is_narrow_tests {
            return Answer::default();
        }
        let Some(command) = event
            .tool_target
            .as_ref()
            .and_then(|target| target.command.as_ref())
        else {
            return Answer::default();
        };
        let Some(run) = whole_suite(&self.failures.commands, command, &self.root, &event.cwd)
        else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            if mode_of(queries)?.is_none_or(|mode| mode == ScaleMode::Tiny) {
                return Ok(Answer::default());
            }
            let Some(task) = open_task_of(queries, event)? else {
                return Ok(Answer::default());
            };
            // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
            if self.is_held_out_task(&task) {
                return Ok(Answer::default());
            }
            let task_id = task.record.task_id;
            let tests = narrow_tests(queries, task_id)?;
            let paths: Vec<_> = tests.iter().map(|test| test.path.clone()).collect();
            let command = narrow_command(&run, &paths);
            let Some((text, named)) = narrow_hint(&tests, command.as_deref()) else {
                return Ok(Answer::default());
            };
            if !narrowed.claim(task_id) {
                return Ok(Answer::default());
            }
            let files = tests
                .iter()
                .take(named)
                .map(|test| InjectedFile {
                    node: test.node,
                    spans: Vec::new(),
                    reason: None,
                    shown: Vec::new(),
                    content: None,
                })
                .collect();
            Ok(Answer {
                given: vec![Given::Injection(InjectionRecord {
                    files,
                    ..text_record(event, Some(task_id), &text)
                })],
                claims: vec![Claim::Narrowed(task_id)],
                text: Some(text),
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a suite run's hint failed");
            Answer::default()
        })
    }
}
