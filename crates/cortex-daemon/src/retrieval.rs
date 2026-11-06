//! The daemon's answers that are text: the work order a prompt gets, the primer a session's start
//! gets, or the recap when its context was compacted, the warnings an edit gets, the hint a read
//! gets, strict mode's reason at a stop, and what the engine finds for a question asked of it
//! directly.
//! Each is worked out with a reader of the store and the graph as it stands, never waiting for the
//! writer; what a prompt's retrieval gave or weighed, the primer or recap given and the warnings
//! given are returned for the writer to record after the event they answer.
//!
//! A prompt of a held-out task is retrieved for in full and gets nothing; what would have been
//! given is recorded all the same. Every retrieval takes the gate's bar as the self-check last
//! raised it, and the recent precision it last found, from the store. Before the graph is built,
//! and while the store has no mode (its first index not done), every answer but the recap is
//! silent: the recap recalls what the session did, which needs no index.

mod hints;
mod recap;
mod stop;
mod warnings;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cortex_core::{
    ContentHash, Event, HookKind, RetrievalConfig, ScaleMode, StartSource, WarningsConfig,
    is_held_out, task_id_of_prompt,
};
use cortex_index::Grammars;
use cortex_learn::{raise_of, recent_precision};
use cortex_retrieve::{Graph, Outcome, Request, Retrieval, Span, locate, primer, retrieve};
use cortex_store::{
    InjectedFile, InjectionRecord, MetaKey, NodeId, Queries, Reader, StoreError, TaskRow,
};

use crate::claim::Claim;
use crate::given::{Given, text_record};
use crate::recapped::Recapped;

/// How long a prompt's spreading may run before it stops where it is: well inside the
/// `UserPromptSubmit` hook's hard timeout of 1 s, with room for the listing and the rendering.
const SPREAD_FOR: Duration = Duration::from_millis(400);

/// How many readers are kept open between requests: more requests than this at once open readers
/// of their own, closed when done.
const KEPT_READERS: usize = 4;

/// What the retrieving answers read and how.
#[derive(Debug)]
pub(crate) struct Service {
    /// The repository's root, where files are read from.
    root: PathBuf,
    /// The store's folder.
    folder: PathBuf,
    /// The memory map each reader opens with, in bytes.
    mmap_bytes: u64,
    /// The `retrieval.*` settings.
    settings: RetrievalConfig,
    /// `index.max_file_bytes`.
    max_file_bytes: u64,
    /// `holdout.share`.
    holdout_share: f64,
    /// What an edit's warnings are worked out with.
    edits: EditSettings,
    /// Readers not in use.
    readers: Mutex<Vec<Reader>>,
}

/// What one answer comes to: the text to give, if any, and what to record of it.
#[derive(Debug, Default)]
pub(crate) struct Answer {
    /// The text the agent gets.
    pub(crate) text: Option<String>,
    /// What the writer records of it.
    pub(crate) given: Option<Given>,
    /// What working it out claimed, given back when the text does not reach the agent.
    pub(crate) claim: Option<Claim>,
}

/// What an edit's warnings are worked out with.
#[derive(Debug, Clone)]
pub(crate) struct EditSettings {
    /// Every language's grammar, to read the edited file's definitions again.
    pub(crate) grammars: Arc<Grammars>,
    /// `index.parse_timeout_ms`: how long reading one file's definitions may take.
    pub(crate) parse_timeout: Duration,
    /// The `warnings.*` settings.
    pub(crate) warnings: WarningsConfig,
    /// `learning.link_decay`: what a co-access link keeps of its weight for each task.
    pub(crate) link_decay: f64,
    /// `strict_mode`: whether a stop is given a reason to go on while call sites of a signature
    /// the task changed are still unread.
    pub(crate) is_strict_mode: bool,
}

/// Where a service's settings come from.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// The repository's root.
    pub(crate) root: PathBuf,
    /// The store's folder.
    pub(crate) folder: PathBuf,
    /// The memory map each reader opens with, in bytes.
    pub(crate) mmap_bytes: u64,
    /// The `retrieval.*` settings.
    pub(crate) retrieval: RetrievalConfig,
    /// `index.max_file_bytes`.
    pub(crate) max_file_bytes: u64,
    /// `holdout.share`.
    pub(crate) holdout_share: f64,
    /// What an edit's warnings are worked out with.
    pub(crate) edits: EditSettings,
}

impl Service {
    /// A service with `settings`, no reader open yet.
    pub(crate) fn new(settings: Settings) -> Self {
        Self {
            root: settings.root,
            folder: settings.folder,
            mmap_bytes: settings.mmap_bytes,
            settings: settings.retrieval,
            max_file_bytes: settings.max_file_bytes,
            holdout_share: settings.holdout_share,
            edits: settings.edits,
            readers: Mutex::new(Vec::new()),
        }
    }

    /// The work order `event`, a prompt, gets over `graph`, seeded with `uncommitted`, the file
    /// nodes of the uncommitted diff, and what to record of the retrieval. A store that cannot be
    /// read gives nothing, and the error is logged.
    pub(crate) fn prompt(
        &self,
        event: &Event,
        graph: Option<&Arc<Graph<NodeId>>>,
        uncommitted: &[NodeId],
    ) -> Answer {
        let Some((graph, prompt)) = graph.zip(event.prompt_text.as_deref()) else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            let Some(mode) = mode_of(queries)? else {
                return Ok(Answer::default());
            };
            let task_id = task_id_of_prompt(event.event_id, event.ts_ms);
            let is_holdout = is_held_out(task_id, self.holdout_share);
            let check = queries.self_check()?;
            let request = Request {
                prompt,
                session: Some(&event.session_id),
                uncommitted,
                mode,
                settings: &self.settings,
                max_file_bytes: self.max_file_bytes,
                raise: raise_of(&check),
                recent_precision: recent_precision(&check),
                deadline: Some(Instant::now() + SPREAD_FOR),
                root: &self.root,
            };
            let retrieval = retrieve(queries, graph, &request)?;
            let injection = retrieval.features.map(|features| InjectionRecord {
                task_id: Some(task_id),
                session_id: event.session_id.clone(),
                hook: HookKind::Prompt,
                at_ms: event.ts_ms,
                files: files_of(&retrieval, graph),
                tokens: rendered(&retrieval).map_or(0, |(_, tokens)| tokens),
                confidence: None,
                is_gated_out: rendered(&retrieval).is_none(),
                is_holdout,
                text_hash: rendered(&retrieval).map(|(text, _)| ContentHash::of(text.as_bytes())),
                features: Some(features),
            });
            // INVARIANT: silent-when-unsure (a held-out task gets nothing, however sure the gate)
            let text = rendered(&retrieval)
                .filter(|_| !is_holdout)
                .map(|(text, _)| text.to_owned());
            Ok(Answer {
                text,
                given: injection.map(Given::Injection),
                claim: None,
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a prompt's retrieval failed");
            Answer::default()
        })
    }

    /// The primer `event`, a session's start, gets over `graph`, and what to record of it. A
    /// start after a compaction gets the recap of what its task was doing instead, once for each
    /// compaction `recapped` heard of.
    pub(crate) fn primer(
        &self,
        event: &Event,
        graph: Option<&Arc<Graph<NodeId>>>,
        recapped: &Recapped,
    ) -> Answer {
        if event.source == Some(StartSource::Compact) {
            return self.recap(event, recapped);
        }
        let Some(graph) = graph else {
            return Answer::default();
        };
        let answered = self.with_reader(|queries| {
            let Some(text) = primer(queries, graph)? else {
                return Ok(Answer::default());
            };
            let given = text_record(event, None, &text);
            Ok(Answer {
                text: Some(text),
                given: Some(Given::Injection(given)),
                claim: None,
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a session's primer failed");
            Answer::default()
        })
    }

    /// Whether `task` is held out, as its record says, or, for a task stored without that, as its
    /// id falls against `holdout.share`: a held-out task is given nothing Cortex would say, so
    /// that it can be compared with the rest.
    fn is_held_out_task(&self, task: &TaskRow) -> bool {
        task.record
            .holdout
            .unwrap_or_else(|| is_held_out(task.record.task_id, self.holdout_share))
    }

    /// What the engine finds for `question` without the gate, at most `limit` files, over `graph`
    /// and seeded with `uncommitted`, as text; none before the graph or the first index, or when it
    /// finds nothing to show.
    ///
    /// # Errors
    ///
    /// The store's errors, and a self-check record that is not one.
    pub(crate) fn locate(
        &self,
        question: &str,
        limit: usize,
        (graph, uncommitted): (Option<&Arc<Graph<NodeId>>>, &[NodeId]),
    ) -> Result<Option<String>, cortex_retrieve::RetrieveError> {
        let Some(graph) = graph else {
            return Ok(None);
        };
        self.with_reader(|queries| {
            let Some(mode) = mode_of(queries)? else {
                return Ok(None);
            };
            let check = queries.self_check()?;
            let request = Request {
                prompt: question,
                session: None,
                uncommitted,
                mode,
                settings: &self.settings,
                max_file_bytes: self.max_file_bytes,
                raise: raise_of(&check),
                recent_precision: recent_precision(&check),
                deadline: Some(Instant::now() + SPREAD_FOR),
                root: &self.root,
            };
            let location = locate(queries, graph, &request, limit)?;
            Ok(location.rendered.map(|rendered| rendered.text))
        })
    }

    /// Runs `read` with a reader of the store, a kept one if one is free, and keeps it for the next
    /// request unless enough are kept.
    fn with_reader<T>(
        &self,
        read: impl FnOnce(Queries<'_>) -> Result<T, cortex_retrieve::RetrieveError>,
    ) -> Result<T, cortex_retrieve::RetrieveError> {
        let kept = self
            .readers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let reader = match kept {
            Some(reader) => reader,
            None => Reader::open(&self.folder, self.mmap_bytes)?,
        };
        let found = reader.consistently(read);
        let mut readers = self.readers.lock().unwrap_or_else(PoisonError::into_inner);
        if readers.len() < KEPT_READERS {
            readers.push(reader);
        }
        found
    }
}

/// The store's scale mode; none before the first index set one.
pub(crate) fn mode_of(queries: Queries<'_>) -> Result<Option<ScaleMode>, StoreError> {
    Ok(queries
        .meta(MetaKey::ScaleMode)?
        .and_then(|name| ScaleMode::from_name(&name)))
}

/// The text `retrieval` came to and its tokens, when it came to a work order.
fn rendered(retrieval: &Retrieval) -> Option<(&str, u32)> {
    match &retrieval.outcome {
        Outcome::WorkOrder(_, rendered) => Some((rendered.text.as_str(), rendered.tokens)),
        Outcome::Silent(_) => None,
    }
}

/// What to record of the files `retrieval` gave or weighed over `graph`: for a work order, the
/// files listed with the spans it wrote and the kind of each reason; otherwise the files the gate
/// weighed, with neither.
fn files_of(retrieval: &Retrieval, graph: &Graph<NodeId>) -> Vec<InjectedFile> {
    if rendered(retrieval).is_some() {
        return retrieval
            .listed
            .iter()
            .filter_map(|entry| {
                let lines = |spans: &[Span]| -> Vec<(u32, u32)> {
                    spans
                        .iter()
                        .map(|span| (span.lines.start_line(), span.lines.end_line()))
                        .collect()
                };
                Some(InjectedFile {
                    node: graph.id(entry.file)?,
                    spans: lines(&entry.spans),
                    reason: Some(entry.item.reason.kind()),
                    shown: lines(&entry.shown),
                    content: entry.item.content.map(|content| content.short()),
                })
            })
            .collect();
    }
    retrieval
        .weighed
        .iter()
        .map(|node| InjectedFile {
            node: *node,
            spans: Vec::new(),
            reason: None,
            shown: Vec::new(),
            content: None,
        })
        .collect()
}
