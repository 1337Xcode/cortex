//! The daemon's answers that are text: the work order a prompt gets, the primer a session's start
//! gets, and what the engine finds for a question asked of it directly. Each is worked out with a
//! reader of the store and the graph as it stands, never waiting for the writer; what a prompt's
//! retrieval gave or weighed, and the primer given, are returned for the writer to record after the
//! event they answer.
//!
//! A prompt of a held-out task is retrieved for in full and gets nothing; what would have been
//! given is recorded all the same. Every retrieval takes the gate's bar as the self-check last
//! raised it, and the recent precision it last found, from the store. Before the graph is built, and while the store has no mode (its
//! first index not done), every answer is silent.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cortex_core::{
    ContentHash, Event, HookKind, RetrievalConfig, ScaleMode, StartSource, estimate_tokens,
    is_held_out, task_id_of_prompt,
};
use cortex_learn::{raise_of, recent_precision};
use cortex_retrieve::{Graph, Outcome, Request, Retrieval, locate, primer, retrieve};
use cortex_store::{InjectedFile, InjectionRecord, MetaKey, NodeId, Queries, Reader, StoreError};

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
    /// Readers not in use.
    readers: Mutex<Vec<Reader>>,
}

/// What one answer comes to: the text to give, if any, and what to record of it.
#[derive(Debug, Default)]
pub(crate) struct Answer {
    /// The text the agent gets.
    pub(crate) text: Option<String>,
    /// What the writer records of it.
    pub(crate) injection: Option<InjectionRecord>,
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
            Ok(Answer { text, injection })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a prompt's retrieval failed");
            Answer::default()
        })
    }

    /// The primer `event`, a session's start, gets over `graph`, and what to record of it. A
    /// start after a compaction gets none: what follows a compaction is its own.
    pub(crate) fn primer(&self, event: &Event, graph: Option<&Arc<Graph<NodeId>>>) -> Answer {
        let Some(graph) = graph else {
            return Answer::default();
        };
        if event.source == Some(StartSource::Compact) {
            return Answer::default();
        }
        let answered = self.with_reader(|queries| {
            let Some(text) = primer(queries, graph)? else {
                return Ok(Answer::default());
            };
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
                injection: Some(injection),
            })
        });
        answered.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a session's primer failed");
            Answer::default()
        })
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
                Some(InjectedFile {
                    node: graph.id(entry.file)?,
                    spans: entry
                        .spans
                        .iter()
                        .map(|span| (span.lines.start_line(), span.lines.end_line()))
                        .collect(),
                    reason: Some(entry.item.reason.kind()),
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
        })
        .collect()
}
