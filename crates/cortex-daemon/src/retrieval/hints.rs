//! The hint a read gets before it runs, worked out with a reader of the store: the read hint's own
//! line, then, in a task, the line naming the file earlier tasks read next, when there is one and
//! both fit the hint's cap (else the next-file line is left out). A tiny repository gets none, as it
//! gets no work order: there Cortex only records; nor does a held-out task. What is given is
//! recorded as the texts given to the session's task, if it has one, the next-file line naming the
//! file it names; the read's event is not recorded.

use cortex_core::{Event, ReasonKind, ScaleMode, estimate_tokens};
use cortex_retrieve::{HintQuery, MAX_HINT_TOKENS, NextQuery, next_file, read_hint};
use cortex_store::{InjectedFile, InjectionRecord};

use super::{Answer, Service, mode_of, open_task_of};
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
            let task = open_task_of(queries, event)?;
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
            let mut given: Vec<Given> = text
                .as_deref()
                .map(|text| Given::Injection(text_record(event, task_id, text)))
                .into_iter()
                .collect();
            let next = match task_id {
                Some(task) => next_file(
                    queries,
                    &NextQuery {
                        root: &self.root,
                        path,
                        task,
                        seq: queries.latest_task_seq()?,
                        link_decay: self.edits.link_decay,
                        max_file_bytes: self.max_file_bytes,
                    },
                )?,
                None => None,
            };
            let mut text = text;
            if let Some(next) = next {
                let joined = format!("{}{}", text.as_deref().unwrap_or_default(), next.line);
                // INVARIANT: budgeted (both lines within the read hint's cap, or the next-file
                // line is left out)
                if estimate_tokens(&joined) <= MAX_HINT_TOKENS {
                    given.push(Given::Injection(InjectionRecord {
                        files: vec![InjectedFile {
                            node: next.node,
                            spans: Vec::new(),
                            reason: Some(ReasonKind::ReadNext),
                            shown: Vec::new(),
                            content: None,
                        }],
                        ..text_record(event, task_id, &next.line)
                    }));
                    text = Some(joined);
                }
            }
            Ok(Answer {
                text,
                given,
                ..Answer::default()
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a read's hint failed");
            Answer::default()
        })
    }
}
