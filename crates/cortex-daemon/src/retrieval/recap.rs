//! The recap a session's start gets after its context was compacted, worked out with a reader of
//! the store, once for each compaction the daemon heard of: a second start for the same
//! compaction gets nothing, and so does a held-out task. A recap depends on no index or graph: it
//! recalls what the session's task did.

use cortex_core::Event;
use cortex_retrieve::recap;

use super::{Answer, Service};
use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::recapped::Recapped;

impl Service {
    /// The recap `event`, a session's start after a compaction, gets, and what to record of it:
    /// none when the session has no open task, its task is held out or has nothing to recall, or
    /// when `recapped` says the compaction was recapped already. The compaction is claimed only
    /// once there is a recap to give, so a store that cannot be read, which gives nothing and is
    /// logged, spends no claim.
    pub(crate) fn recap(&self, event: &Event, recapped: &Recapped) -> Answer {
        let answered = self.with_reader(|queries| {
            let Some(task) = queries.open_task(&event.session_id)? else {
                return Ok(Answer::default());
            };
            // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
            if self.is_held_out_task(&task) {
                return Ok(Answer::default());
            }
            let Some(text) = recap(queries, &task, (self.root_for(event), self.max_file_bytes))?
            else {
                return Ok(Answer::default());
            };
            if !recapped.claim(&event.session_id, event.event_id) {
                return Ok(Answer::default());
            }
            let given = text_record(event, Some(task.record.task_id), &text);
            Ok(Answer {
                text: Some(text),
                given: vec![Given::Injection(given)],
                claims: vec![Claim::Recapped(event.session_id.clone(), event.event_id)],
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a session's recap failed");
            Answer::default()
        })
    }
}
