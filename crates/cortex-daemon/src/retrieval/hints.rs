//! The hint a read gets before it runs, worked out with a reader of the store: the read hint's own
//! line, then, in a task, the line naming the file earlier tasks read next, when there is one, both
//! fit the hint's cap, and no other answer claimed that file for the task (else the next-file line
//! is left out). A tiny repository gets none, as it gets no work order: there Cortex only records;
//! nor does a held-out task. What is given is recorded as the texts given to the session's task, if
//! it has one, the next-file line naming the file it names; the read's event is not recorded.

use cortex_core::{Event, ReasonKind, RepoPath, ScaleMode, Ulid, estimate_tokens};
use cortex_retrieve::{HintQuery, MAX_HINT_TOKENS, NextCandidate, NextQuery, next_file, read_hint};
use cortex_store::{InjectedFile, InjectionRecord, NodeId, Queries, StoreError};

use super::{Answer, Service, mode_of, open_task_of};
use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::once::ClaimedOnce;

impl Service {
    /// The hint `event` gets, the moment before a read, and what to record of it, the files named
    /// as read next claimed in `read_next`: none for an event that is no read of a file inside the
    /// repository, before the first index, in tiny mode, in a held-out task, or when there is
    /// nothing to say. A store that cannot be read gives nothing, and the error is logged; one that
    /// fails only on the next-file line keeps the read hint.
    pub(crate) fn read_hint(
        &self,
        event: &Event,
        read_next: &ClaimedOnce<(Ulid, NodeId)>,
    ) -> Answer {
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
            let mut answer = Answer {
                given: text
                    .as_deref()
                    .map(|text| Given::Injection(text_record(event, task_id, text)))
                    .into_iter()
                    .collect(),
                text,
                ..Answer::default()
            };
            if let Some(task_id) = task_id {
                self.add_next_file(queries, (event, path, task_id), read_next, &mut answer);
            }
            Ok(answer)
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a read's hint failed");
            Answer::default()
        })
    }

    /// Adds to `answer`, the hint of the read `event` of `path` in task `task_id`, the line naming
    /// the file earlier tasks read next, when there is one, the two fit the read hint's cap, the
    /// file is claimed for the task in `read_next` now, and then found on disk as indexed (its
    /// claim given back when it is not). A store error leaves `answer` as it was, and is logged.
    fn add_next_file(
        &self,
        queries: Queries<'_>,
        (event, path, task_id): (&Event, &RepoPath, Ulid),
        read_next: &ClaimedOnce<(Ulid, NodeId)>,
        answer: &mut Answer,
    ) {
        let candidate = match self.next_file_of(queries, path, task_id) {
            Ok(Some(candidate)) => candidate,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "a read's next file failed");
                return;
            }
        };
        let joined = format!(
            "{}{}",
            answer.text.as_deref().unwrap_or_default(),
            candidate.line()
        );
        let node = candidate.node();
        // INVARIANT: budgeted (both lines within the read hint's cap, or the next-file line is
        // left out)
        if estimate_tokens(&joined) > MAX_HINT_TOKENS || !read_next.claim((task_id, node)) {
            return;
        }
        // The file is read and hashed last, only for a line about to be given.
        let Some(next) = candidate.checked(&self.root, self.max_file_bytes) else {
            read_next.release(&(task_id, node));
            return;
        };
        answer.given.push(Given::Injection(InjectionRecord {
            files: vec![InjectedFile {
                node,
                spans: Vec::new(),
                reason: Some(ReasonKind::ReadNext),
                shown: Vec::new(),
                content: None,
            }],
            ..text_record(event, Some(task_id), &next.line)
        }));
        answer.claims.push(Claim::ReadNext(task_id, node));
        answer.text = Some(joined);
    }

    /// The line naming the file earlier tasks read after `path`, for task `task_id`, its file not
    /// yet checked on disk.
    fn next_file_of(
        &self,
        queries: Queries<'_>,
        path: &RepoPath,
        task_id: Ulid,
    ) -> Result<Option<NextCandidate>, StoreError> {
        next_file(
            queries,
            &NextQuery {
                path,
                task: task_id,
                seq: queries.latest_task_seq()?,
                link_decay: self.edits.link_decay,
            },
        )
    }
}
