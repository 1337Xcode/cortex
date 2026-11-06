//! The recap a session's start gets after its context was compacted, worked out with a reader of
//! the store, once for each compaction the daemon heard of: a second start for the same
//! compaction gets nothing. A recap depends on no index or graph: it recalls what the session's
//! task did.

use cortex_core::{ContentHash, Event, HookKind, estimate_tokens};
use cortex_retrieve::recap;
use cortex_store::InjectionRecord;

use super::{Answer, Service};
use crate::given::Given;
use crate::recapped::Recapped;

impl Service {
    /// The recap `event`, a session's start after a compaction, gets, and what to record of it:
    /// none when the session has no open task or nothing to recall, or when `recapped` says the
    /// compaction was recapped already. The compaction is claimed only once there is a recap to
    /// give, so a store that cannot be read, which gives nothing and is logged, spends no claim.
    pub(crate) fn recap(&self, event: &Event, recapped: &Recapped) -> Answer {
        let answered = self.with_reader(|queries| {
            let Some(text) = recap(queries, &event.session_id)? else {
                return Ok(Answer::default());
            };
            if !recapped.claim(&event.session_id, event.event_id) {
                return Ok(Answer::default());
            }
            let injection = InjectionRecord {
                task_id: None,
                session_id: event.session_id.clone(),
                hook: HookKind::SessionStart,
                at_ms: event.ts_ms,
                files: Vec::new(),
                tokens: u32::try_from(estimate_tokens(&text)).unwrap_or(u32::MAX),
                confidence: None,
                is_gated_out: false,
                is_holdout: false,
                text_hash: Some(ContentHash::of(text.as_bytes())),
                features: None,
            };
            Ok(Answer {
                text: Some(text),
                given: Some(Given::Injection(injection)),
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a session's recap failed");
            Answer::default()
        })
    }
}
