//! The keep-list a compaction's summary is given before Claude Code compacts a session's context:
//! the working set of the session's open task, for the summary to keep. Worked out with a reader of
//! the store; nothing is stored but what was given, as for any text.

use cortex_core::{Event, ScaleMode};
use cortex_retrieve::keep_list;

use super::{Answer, Service, mode_of, open_task_of};
use crate::given::{Given, text_record};

impl Service {
    /// The keep-list `event`, a compaction about to run, gets: none when `compaction.keep_list` is
    /// off, before the first index, in tiny mode, outside a task or in a held-out one, or when the
    /// task has nothing to keep. A store that cannot be read gives nothing, and the error is
    /// logged.
    pub(crate) fn keep_list(&self, event: &Event) -> Answer {
        if !self.is_keep_list {
            return Answer::default();
        }
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
            let Some(text) = keep_list(queries, &task, (&self.root, self.max_file_bytes))? else {
                return Ok(Answer::default());
            };
            Ok(Answer {
                given: vec![Given::Injection(text_record(
                    event,
                    Some(task.record.task_id),
                    &text,
                ))],
                text: Some(text),
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a compaction's keep-list failed");
            Answer::default()
        })
    }
}
