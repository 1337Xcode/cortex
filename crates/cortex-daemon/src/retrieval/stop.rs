//! The reason strict mode gives an agent to go on when it stops, worked out with a reader of the
//! store: only when the user turned `strict_mode` on, and once a task. A held-out task is never
//! given one: it is given no warnings, so none of them is recorded to be followed up.

use cortex_core::Event;
use cortex_retrieve::stop_reason;

use super::{Answer, Service};
use crate::given::{Given, text_record};
use crate::stopped::Stopped;

impl Service {
    /// The reason `event`, an agent's stop, gets to go on, and what to record of it: none with
    /// strict mode off, outside a task, when no call-site warning of the task is open (a held-out
    /// task has none), or when `stopped` says the task was given its reason already. The task is claimed
    /// only once there is a reason to give. A store that cannot be read gives nothing, and the
    /// error is logged.
    pub(crate) fn stop_reason(&self, event: &Event, stopped: &Stopped) -> Answer {
        if !self.edits.is_strict_mode {
            return Answer::default();
        }
        let answered = self.with_reader(|queries| {
            let Some(task) = queries.open_task(&event.session_id)? else {
                return Ok(Answer::default());
            };
            let task_id = task.record.task_id;
            let Some(text) = stop_reason(queries, task_id)? else {
                return Ok(Answer::default());
            };
            if !stopped.claim(task_id) {
                return Ok(Answer::default());
            }
            let given = text_record(event, Some(task_id), &text);
            Ok(Answer {
                text: Some(text),
                given: Some(Given::Injection(given)),
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a stop's reason failed");
            Answer::default()
        })
    }
}
