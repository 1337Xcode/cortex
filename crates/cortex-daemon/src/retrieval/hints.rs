//! The hint a read gets before it runs, worked out with a reader of the store. A tiny repository
//! gets none, as it gets no work order: there Cortex only records; nor does a held-out task. What
//! is given is recorded as the text given to the session's task, if it has one; the read's event
//! is not recorded.

use cortex_core::{Event, ScaleMode};
use cortex_retrieve::{HintQuery, read_hint};

use super::{Answer, Service, mode_of};
use crate::given::{Given, text_record};

impl Service {
    /// The hint `event` gets, the moment before a read, and what to record of it: none for an
    /// event that is no read of a file inside the repository, before the first index, in tiny
    /// mode, in a held-out task, or when there is nothing to say. A store that cannot be read
    /// gives nothing, and the error is logged.
    pub(crate) fn read_hint(&self, event: &Event) -> Answer {
        let Some(path) = event.file_to_read() else {
            return Answer::default();
        };
        // A read of several ranges at once is asked about whole: none of them is the read.
        let asked = match event
            .tool_target
            .as_ref()
            .map(|target| target.ranges.as_slice())
        {
            Some([range]) => Some(*range),
            _ => None,
        };
        let answered = self.with_reader(|queries| {
            let Some(mode) = mode_of(queries)? else {
                return Ok(Answer::default());
            };
            if mode == ScaleMode::Tiny {
                return Ok(Answer::default());
            }
            let task = queries.open_task(&event.session_id)?;
            // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
            if task
                .as_ref()
                .is_some_and(|task| self.is_held_out_task(task))
            {
                return Ok(Answer::default());
            }
            let task_id = task.map(|task| task.record.task_id);
            let text = read_hint(
                queries,
                &HintQuery {
                    root: &self.root,
                    path,
                    asked,
                    session: &event.session_id,
                    task: task_id,
                    max_file_bytes: self.max_file_bytes,
                },
            )?;
            let given = text
                .as_deref()
                .map(|text| Given::Injection(text_record(event, task_id, text)));
            Ok(Answer {
                text,
                given,
                claim: None,
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a read's hint failed");
            Answer::default()
        })
    }
}
