//! The daemon's side of the protocol: accept connections, answer the handshake, then each request,
//! until the client closes, asks to stop, or breaks the protocol.
//!
//! Each connection is a task of the tokio runtime; at most [`MAX_CONNECTIONS`] are served at once
//! and a frame must arrive within [`FRAME_TIMEOUT`], so no client can tie the daemon up. A
//! connection that breaks is kept among the daemon's recent errors, in the daemon's own words and
//! never with the client's bytes, and the time each hook's request took to handle is kept for the
//! daemon's status.

use std::io;
use std::sync::atomic::{self, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use cortex_core::ipc::{
    CORTEX_VERSION, FrameError, LENGTH_PREFIX_BYTES, Mismatch, Protocol, Request, RequestBody,
    Response, ResponseBody, StatusReport, Welcome, body_len, decode, encode,
};
use cortex_core::{Event, HookKind};
use cortex_retrieve::Graph;
use cortex_store::NodeId;
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::claim::{Claim, Claims};
use crate::facts::{FACTS_START_WITHIN, FACTS_WRITE_WITHIN, FactsRequest};
use crate::given::{Given, recorded};
use crate::graph::Snapshot;
use crate::host::now_ms;
use crate::inbox::Inbox;
use crate::latency::Latencies;
use crate::retrieval::Service;
use crate::signatures::Signatures;
use crate::troubles::Taken;
use crate::worktree::Worktree;

/// Most connections served at once. Hooks and commands hold a connection for one exchange, so more
/// at once means something is flooding the daemon; a connection past this is closed at once.
const MAX_CONNECTIONS: usize = 64;

/// How long a frame may take to arrive once its connection opened or the previous frame was read.
/// Hooks and commands send at once; a client that does not is closed.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// What the connections share.
#[derive(Debug)]
pub(crate) struct Server {
    /// When the daemon started.
    started: Instant,
    /// The writer's inbox, which a Shutdown stops and which keeps the daemon's recent errors.
    inbox: Arc<Inbox>,
    /// The graph retrieval spreads over.
    snapshot: Arc<Snapshot>,
    /// The work tree's uncommitted diff, which seeds retrievals.
    worktree: Arc<Worktree>,
    /// The signature changes the writer's indexes found lately, for an edit's warnings.
    signatures: Arc<Signatures>,
    /// What answers claimed: the pairs each task was warned of, the compactions recapped, the
    /// tasks given strict mode's reason.
    claims: Claims,
    /// What answers with text, once the first index is done.
    service: Arc<OnceLock<Service>>,
    /// How long each hook's latest requests took to handle.
    latencies: Mutex<Latencies>,
    /// Probes, `cortex doctor`'s, answered since the daemon started.
    probes_answered: AtomicU64,
}

/// How long before its hook stops reading a reply holding text must be sent: the moment the reply
/// takes to reach the hook's reading thread. Text worked out later is not sent.
const SEND_MARGIN_MS: i64 = 10;

/// Whether a hook that stops reading its answer at `reads_until_ms` has stopped by `at_ms`, both
/// UTC milliseconds; never for a hook that waits as long as it takes.
fn is_past(reads_until_ms: Option<i64>, at_ms: i64) -> bool {
    reads_until_ms.is_some_and(|until| at_ms > until)
}

/// Whether a reply holding text, ready at `now_ms`, comes too late to send to a hook that stops
/// reading at `reads_until_ms`: it would not reach the hook [`SEND_MARGIN_MS`] before then.
fn is_too_late_to_send(reads_until_ms: Option<i64>, now_ms: i64) -> bool {
    is_past(reads_until_ms, now_ms.saturating_add(SEND_MARGIN_MS))
}

/// The most files a question asked directly names.
const MAX_WHERE_FILES: u8 = 5;

/// Which text a hook's request asks for.
#[derive(Debug, Clone, Copy)]
enum Asked {
    /// The work order a prompt gets.
    WorkOrder,
    /// The primer a session's start gets.
    Primer,
    /// The warnings an edit gets.
    Warnings,
    /// The hint a read gets, before it runs.
    ReadHint,
    /// The reason strict mode gives a stop to go on.
    StopReason,
}

/// A request as read, with when the first byte of its frame arrived.
#[derive(Debug)]
struct Received {
    /// The request.
    request: Request,
    /// When its frame began to arrive.
    began: Instant,
}

/// Why a connection broke off.
#[derive(Debug)]
enum Broken {
    /// A request could not be read.
    Unreadable(FrameError),
    /// The first request was not the handshake.
    NoHello,
    /// The handshake came a second time.
    HelloAgain,
    /// An answer could not be written.
    Unwritable(FrameError),
}

impl Broken {
    /// What broke, for the daemon's recent errors, in words of the daemon's own.
    fn message(&self) -> String {
        match self {
            Self::Unreadable(FrameError::Io(error)) if error.kind() == io::ErrorKind::TimedOut => {
                format!(
                    "a request was cut off: the rest of its frame did not arrive within {} s",
                    FRAME_TIMEOUT.as_secs()
                )
            }
            Self::Unreadable(FrameError::Io(_)) => {
                "a request was cut off: the connection closed in the middle of its frame".to_owned()
            }
            // INVARIANT: no-content-stored (a frame's own error names its kind or size only; the
            // JSON error inside it, which can quote the client's text, is left out)
            Self::Unreadable(error) => format!("a request could not be read: {error}"),
            Self::NoHello => "a client asked before its handshake".to_owned(),
            Self::HelloAgain => "a client sent its handshake twice".to_owned(),
            // The operating system's reason, such as a pipe the client had already closed.
            Self::Unwritable(FrameError::Io(error)) => {
                format!("an answer could not be written: {error}")
            }
            Self::Unwritable(error) => format!("an answer could not be written: {error}"),
        }
    }
}

/// An answer with text, worked out: its reply, its hook, what to record of it, and what to undo
/// when its text does not reach the hook.
struct Answered {
    /// The reply.
    body: ResponseBody,
    /// The hook it answers.
    hook: Option<HookKind>,
    /// What the writer records of it.
    given: Vec<Given>,
    /// What to undo when its text does not reach the hook.
    undo: Undo,
}

impl Answered {
    /// The refusal `body` of a request from hook `hook`: nothing recorded, nothing to undo.
    fn refused(body: ResponseBody, hook: HookKind) -> Self {
        Self {
            body,
            hook: Some(hook),
            given: Vec::new(),
            undo: Undo::default(),
        }
    }
}

/// What an answer whose text never reaches its hook leaves to undo: the claim working it out made,
/// and a stop held back until its reason arrived, which happened after all.
#[derive(Debug, Default)]
struct Undo {
    /// What working the answer out claimed.
    claims: Vec<Claim>,
    /// The stop the answer held back.
    stop: Option<Event>,
    /// The failures the answer took to tell the user.
    troubles: Option<Taken>,
}

impl Server {
    /// A server for a daemon started now, whose writer reads `inbox`, answering from the graph of
    /// `snapshot`, the uncommitted diff of `worktree` and the signature changes `signatures` holds
    /// with `service`.
    pub(crate) fn new(
        inbox: Arc<Inbox>,
        (snapshot, worktree, signatures): (Arc<Snapshot>, Arc<Worktree>, Arc<Signatures>),
        service: Arc<OnceLock<Service>>,
    ) -> Self {
        Self {
            started: Instant::now(),
            inbox,
            snapshot,
            worktree,
            signatures,
            claims: Claims::default(),
            service,
            latencies: Mutex::new(Latencies::default()),
            probes_answered: AtomicU64::new(0),
        }
    }

    /// Accepts connections on `listener` and serves each on a task of its own, until the runtime
    /// shuts down.
    pub(crate) async fn serve(self: Arc<Self>, listener: Listener) {
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            // A connection that fails while being accepted concerns that client alone.
            let Ok(stream) = listener.accept().await else {
                continue;
            };
            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                server.connection(stream).await;
                drop(permit);
            });
        }
    }

    /// Serves one connection. One that breaks ends, and what broke it is kept among the recent
    /// errors before the connection closes, so that a client that sees it closed finds the error
    /// in the status; the daemon goes on.
    async fn connection(&self, stream: Stream) {
        let (mut reader, mut writer) = tokio::io::split(stream);
        if let Err(broken) = self.exchange(&mut reader, &mut writer).await {
            self.note(&broken);
        }
    }

    /// The handshake, then each request in turn, until the client closes or asks to stop.
    async fn exchange(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<(), Broken> {
        let Some(Received { request: hello, .. }) =
            read_request(reader).await.map_err(Broken::Unreadable)?
        else {
            return Ok(());
        };
        self.inbox.note_request();
        let RequestBody::Hello(greeting) = hello.body else {
            return Err(Broken::NoHello);
        };
        if !Protocol::CURRENT.is_compatible_with(greeting.protocol) {
            return self.mismatch(hello.request_id, reader, writer).await;
        }
        let welcome = ResponseBody::Welcome(Welcome {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
            pid: std::process::id(),
        });
        write_response(writer, hello.request_id, welcome)
            .await
            .map_err(Broken::Unwritable)?;
        while let Some(received) = read_request(reader).await.map_err(Broken::Unreadable)? {
            self.inbox.note_request();
            if !self.answer(received, writer).await? {
                break;
            }
        }
        Ok(())
    }

    /// Answers the handshake of a client of another protocol, `request_id`, with this daemon's
    /// protocol and version. The client may still send the one request every version reads, a
    /// stop; anything else, which this daemon may not be able to read, ends the connection.
    async fn mismatch(
        &self,
        request_id: u64,
        reader: &mut (impl AsyncRead + Unpin),
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<(), Broken> {
        let mismatch = ResponseBody::Mismatch(Mismatch {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
        });
        write_response(writer, request_id, mismatch)
            .await
            .map_err(Broken::Unwritable)?;
        if let Ok(Some(Received { request: next, .. })) = read_request(reader).await
            && next.body == RequestBody::Shutdown
        {
            let acknowledged = write_response(writer, next.request_id, ResponseBody::Ack).await;
            // A stop holds whether or not the client stays for its acknowledgement.
            self.inbox.stop();
            acknowledged.map_err(Broken::Unwritable)?;
        }
        Ok(())
    }

    /// Answers the request `received`, and returns whether the connection goes on: not after a
    /// stop or a second handshake. The time a hook's request took, from the first byte of its
    /// frame until its answer is ready to write, is kept.
    async fn answer(
        &self,
        received: Received,
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<bool, Broken> {
        let Received { request, began } = received;
        let is_shutdown = request.body == RequestBody::Shutdown;
        let mut given = Vec::new();
        let mut undo = Undo::default();
        let mut reads_until_ms = None;
        let (body, hook) = match request.body {
            RequestBody::Status => (ResponseBody::Status(self.report()), None),
            RequestBody::Shutdown => (ResponseBody::Ack, None),
            RequestBody::Hello(_) => {
                // Kept before the refusal is written, so that a client that reads it finds the
                // error in the status, as one that sees a broken connection close does.
                self.note(&Broken::HelloAgain);
                let refusal = error("a connection says hello once");
                write_response(writer, request.request_id, refusal)
                    .await
                    .map_err(Broken::Unwritable)?;
                return Ok(false);
            }
            RequestBody::Retrieve {
                event,
                answer_by_ms,
            } => {
                reads_until_ms = answer_by_ms;
                let asked = (Asked::WorkOrder, answer_by_ms);
                let answered = self.answer_with_text(*event, asked).await;
                (given, undo) = (answered.given, answered.undo);
                (answered.body, answered.hook)
            }
            RequestBody::Primer {
                event,
                answer_by_ms,
            } => {
                reads_until_ms = answer_by_ms;
                let asked = (Asked::Primer, answer_by_ms);
                let answered = self.answer_with_text(*event, asked).await;
                (given, undo) = (answered.given, answered.undo);
                (answered.body, answered.hook)
            }
            RequestBody::PostEdit {
                event,
                answer_by_ms,
            } => {
                reads_until_ms = answer_by_ms;
                let asked = (Asked::Warnings, answer_by_ms);
                let answered = self.answer_with_text(*event, asked).await;
                (given, undo) = (answered.given, answered.undo);
                (answered.body, answered.hook)
            }
            RequestBody::ReadHint {
                event,
                answer_by_ms,
            } => {
                reads_until_ms = answer_by_ms;
                let asked = (Asked::ReadHint, answer_by_ms);
                let answered = self.answer_with_text(*event, asked).await;
                (given, undo) = (answered.given, answered.undo);
                (answered.body, answered.hook)
            }
            RequestBody::StopReason {
                event,
                answer_by_ms,
            } => {
                reads_until_ms = answer_by_ms;
                let asked = (Asked::StopReason, answer_by_ms);
                let answered = self.answer_with_text(*event, asked).await;
                (given, undo) = (answered.given, answered.undo);
                (answered.body, answered.hook)
            }
            RequestBody::Where { query, limit } => (self.locate(query, limit).await, None),
            RequestBody::Note(note) => (self.ask_writer(FactsRequest::Note(note)).await, None),
            RequestBody::Facts(ask) => (self.ask_writer(FactsRequest::Ask(ask)).await, None),
            RequestBody::Event { event } => {
                let hook = event.hook;
                // The moment before a tool runs is answered, not recorded: the tool's result is,
                // once it ran. A probe, `cortex doctor`'s, is answered like the agent's and
                // recorded never: no session, task or event of it reaches the store. It is
                // counted, so that doctor sees its probes arrive.
                let is_taken = if event.is_probe {
                    self.probes_answered.fetch_add(1, atomic::Ordering::Relaxed);
                    true
                } else {
                    // A compaction is heard of as it is reported, before the writer records it:
                    // the start that follows may come first.
                    if hook == HookKind::Compact {
                        self.claims.recapped.compacted(&event.session_id);
                    }
                    hook == HookKind::PreTool || self.inbox.push_event(*event)
                };
                // A daemon that is stopping records nothing more, and says so rather than take an
                // event it would lose.
                let body = if is_taken {
                    ResponseBody::Ack
                } else {
                    error("the daemon is stopping and records no more events")
                };
                (body, Some(hook))
            }
        };
        let took = began.elapsed();
        if let Some(hook) = hook {
            self.latencies().note(hook, took);
        }
        let holds_text = matches!(&body, ResponseBody::Reply { text: Some(_), .. });
        let tells_user = matches!(
            &body,
            ResponseBody::Reply {
                to_user: Some(_),
                ..
            }
        );
        let answered = write_response(writer, request.request_id, body).await;
        // Recorded once its answer is written, after its event, as `recorded` says.
        if !given.is_empty() {
            let is_late = is_past(reads_until_ms, now_ms());
            if holds_text && answered.is_ok() && is_late {
                self.inbox.note_error(
                    "an answer was written after its hook stopped waiting",
                    now_ms(),
                );
            }
            for given in given {
                if let Some(given) = recorded(given, holds_text, answered.is_ok(), is_late) {
                    self.inbox.push_given(given, now_ms());
                }
            }
        }
        // Text that did not reach its hook gives back what it claimed, and a stop it held back is
        // recorded: the agent stopped after all. A message for the user alone gives back the
        // failures it took to tell, though the reply held no text.
        if (holds_text || tells_user) && (answered.is_err() || is_past(reads_until_ms, now_ms())) {
            self.undo(undo);
        }
        if is_shutdown {
            // A stop holds whether or not the client stays for its acknowledgement.
            self.inbox.stop();
        }
        answered.map_err(Broken::Unwritable)?;
        // Logged once the hook has its answer: the line costs it nothing.
        if let Some(hook) = hook {
            let latency_us = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
            tracing::info!(
                request_id = request.request_id,
                hook = hook.name(),
                latency_us,
                "hook handled"
            );
        }
        Ok(!is_shutdown)
    }

    /// Records `event` as an event request does, then works out the text `asked` names on the
    /// blocking pool, returning the answer and what is to be recorded of it once the answer is
    /// written. Text that cannot reach the hook by `answer_by_ms`, when the hook stops reading, is
    /// not sent, and is recorded as not delivered. A probe is answered and counted, and nothing of
    /// it recorded; an event of another hook than `asked` takes is refused, and nothing of it
    /// recorded either.
    async fn answer_with_text(
        &self,
        event: Event,
        (asked, answer_by_ms): (Asked, Option<i64>),
    ) -> Answered {
        let (hook, is_probe) = (event.hook, event.is_probe);
        let expected = match asked {
            Asked::WorkOrder => HookKind::Prompt,
            Asked::Primer => HookKind::SessionStart,
            Asked::Warnings => HookKind::PostTool,
            Asked::ReadHint => HookKind::PreTool,
            Asked::StopReason => HookKind::Stop,
        };
        if hook != expected {
            return Answered::refused(
                error("the request's event is of another hook than it asks for"),
                hook,
            );
        }
        // The moment before a tool runs is answered, not recorded, as for an event request. A stop
        // is recorded once its answer is known: one given a reason to go on did not happen, and
        // its task goes on; and the reason is worked out before the stop could end the task.
        let is_stop = hook == HookKind::Stop;
        if is_probe {
            self.probes_answered.fetch_add(1, atomic::Ordering::Relaxed);
        } else if hook != HookKind::PreTool && !is_stop && !self.inbox.push_event(event.clone()) {
            return Answered::refused(
                error("the daemon is stopping and records no more events"),
                hook,
            );
        }
        let stop = (is_stop && !is_probe).then(|| event.clone());
        let agent = event.agent;
        let signatures = Arc::clone(&self.signatures);
        let claims = self.claims.clone();
        let mut answer = self
            .with_service(move |service, graph, uncommitted| {
                let answer = match asked {
                    Asked::WorkOrder => {
                        service.prompt(&event, (graph, uncommitted), &claims.handed)
                    }
                    Asked::Primer => service.primer(&event, graph, &claims.recapped),
                    Asked::Warnings => service.post_edit(&event, (&signatures, &claims.told)),
                    Asked::ReadHint => service.read_hint(&event),
                    Asked::StopReason => service.stop_reason(&event, &claims.stopped),
                };
                // Notices go with every text but a stop's, which would hold the stop back. A
                // probe's session is no agent's: it is given no notice and leaves none.
                let has_notices = !matches!(asked, Asked::StopReason) && !is_probe;
                if has_notices {
                    service.with_notices(&event, answer, &claims.noticed)
                } else {
                    answer
                }
            })
            .await
            .unwrap_or_default();
        // INVARIANT: measured-claims. Text that cannot reach the hook before it stops reading
        // reaches no agent: it is not sent, and what it would have given is recorded as not
        // delivered.
        if answer.text.is_some() && is_too_late_to_send(answer_by_ms, now_ms()) {
            answer.text = None;
            answer.given = answer
                .given
                .into_iter()
                .filter_map(|given| recorded(given, true, false, true))
                .collect();
            if !is_probe {
                self.inbox.note_error(
                    "an answer was worked out after its hook stopped waiting",
                    now_ms(),
                );
            }
            for claim in answer.claims.drain(..) {
                self.claims.release(&claim);
            }
        }
        // A stop given a reason is held back until the reason reaches its hook.
        let (stop, held_stop) = if answer.text.is_none() {
            (stop, None)
        } else {
            (None, stop)
        };
        if let Some(stop) = stop
            && !self.inbox.push_event(stop)
        {
            return Answered::refused(
                error("the daemon is stopping and records no more events"),
                hook,
            );
        }
        let given = if is_probe { Vec::new() } else { answer.given };
        // A session's start tells the user, never the model, which steps the daemon runs on its
        // own failed since the user was last told; a probe's start is no user's, a start answered
        // too late reaches no one, and an agent that shows its user no hook's message cannot tell
        // them, so none of those takes them.
        let (to_user, troubles) = (matches!(asked, Asked::Primer)
            && !is_probe
            && agent.shows_user_messages()
            && !is_too_late_to_send(answer_by_ms, now_ms()))
        .then(|| self.inbox.take_troubles(now_ms()))
        .flatten()
        .map_or((None, None), |(text, taken)| (Some(text), Some(taken)));
        Answered {
            body: ResponseBody::Reply {
                text: answer.text,
                to_user,
            },
            hook: Some(hook),
            given,
            undo: Undo {
                claims: if is_probe { Vec::new() } else { answer.claims },
                stop: held_stop,
                troubles,
            },
        }
    }

    /// Undoes `undo`: gives back its claim and records the stop it held back.
    fn undo(&self, undo: Undo) {
        for claim in &undo.claims {
            self.claims.release(claim);
        }
        if let Some(stop) = undo.stop
            && !self.inbox.push_event(stop)
        {
            self.inbox
                .note_error("a stop was not recorded: the daemon is stopping", now_ms());
        }
        if let Some(taken) = undo.troubles {
            self.inbox.put_back_troubles(taken);
        }
    }

    /// What the engine finds for `query` without the gate, at most `limit` files (1 to 5): an
    /// error, kept among the recent errors, when the engine could not look.
    async fn locate(&self, query: String, limit: u8) -> ResponseBody {
        let limit = usize::from(limit.clamp(1, MAX_WHERE_FILES));
        let located = self
            .with_service(move |service, graph, uncommitted| {
                service.locate(&query, limit, (graph, uncommitted))
            })
            .await;
        match located {
            Some(Ok(text)) => ResponseBody::Reply {
                text,
                to_user: None,
            },
            None => ResponseBody::Reply {
                text: None,
                to_user: None,
            },
            Some(Err(error)) => {
                let message = format!("{error:#}");
                tracing::warn!(error = %message, "a question was not answered");
                self.inbox
                    .note_error("a question was not answered", now_ms());
                ResponseBody::Error { message }
            }
        }
    }

    /// Hands a fact request to the writer and waits, on the blocking pool, for its answer: within
    /// [`FACTS_START_WITHIN`] for the writer to take it, then [`FACTS_WRITE_WITHIN`] to write it. A
    /// request the writer did not take in time is never written.
    async fn ask_writer(&self, request: FactsRequest) -> ResponseBody {
        let Some(answer) = self.inbox.push_facts(request) else {
            return error("the daemon is stopping, or too many fact requests wait: try again");
        };
        let within = FACTS_START_WITHIN + FACTS_WRITE_WITHIN;
        let waited = tokio::task::spawn_blocking(move || answer.recv_timeout(within)).await;
        match waited {
            Ok(Ok(Ok(answer))) => ResponseBody::Facts(answer),
            Ok(Ok(Err(message))) => ResponseBody::Error { message },
            Ok(Err(mpsc::RecvTimeoutError::Disconnected)) => error(
                "the daemon was busy and did not take the request, which changed nothing: try again",
            ),
            Ok(Err(mpsc::RecvTimeoutError::Timeout)) | Err(_) => error(
                "the daemon did not answer in time: run `cortex facts list` to see whether it was done",
            ),
        }
    }

    /// Runs `work` with the service, the graph as it stands and the uncommitted diff's file nodes
    /// as last read (none before the first look) on the blocking pool, so that no connection's
    /// worker waits on the store; none before the service is installed, or when the work panicked,
    /// which is kept among the recent errors.
    async fn with_service<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Service, Option<&Arc<Graph<NodeId>>>, &[NodeId]) -> T + Send + 'static,
    ) -> Option<T> {
        let graph = self.snapshot.graph();
        let uncommitted = self.worktree.uncommitted();
        let service = Arc::clone(&self.service);
        let done = tokio::task::spawn_blocking(move || {
            let nodes = uncommitted.as_deref().map_or(&[][..], Vec::as_slice);
            service
                .get()
                .map(|service| work(service, graph.as_ref(), nodes))
        })
        .await;
        match done {
            Ok(answered) => answered,
            Err(failure) => {
                tracing::warn!(error = %failure, "an answer's work failed");
                self.inbox.note_error("an answer's work failed", now_ms());
                None
            }
        }
    }

    /// Keeps what broke a connection among the recent errors, and logs it.
    fn note(&self, broken: &Broken) {
        let message = broken.message();
        tracing::warn!(error = %message, "a connection broke");
        self.inbox.note_error(&message, now_ms());
    }

    /// The daemon's state.
    fn report(&self) -> StatusReport {
        // The times are copied out, so that connections noting theirs wait for the copy only, not
        // for the sorting, and the inbox is never locked while the times are.
        let latencies = self.latencies().clone();
        let counts = self.inbox.counts();
        StatusReport {
            cortex_version: CORTEX_VERSION.to_owned(),
            protocol: Protocol::CURRENT,
            pid: std::process::id(),
            uptime_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            events_waiting: u64::try_from(counts.events_waiting).unwrap_or(u64::MAX),
            changes_waiting: u64::try_from(counts.changes_waiting).unwrap_or(u64::MAX),
            events_dropped: counts.events_dropped,
            failed_batches: counts.failed_batches,
            failed_checkpoints: counts.failed_checkpoints,
            probes_answered: self.probes_answered.load(atomic::Ordering::Relaxed),
            is_indexed: counts.is_indexed,
            latencies: latencies.report(),
            recent_errors: self.inbox.recent_errors(),
            git: Some(self.inbox.git()),
            graph: self.snapshot.report(),
            worktree: self.worktree.report(),
        }
    }

    /// The hooks' handling times, locked. A panic while they were held leaves them consistent
    /// (each time is noted in one push), so a poisoned lock is taken as it is.
    fn latencies(&self) -> MutexGuard<'_, Latencies> {
        self.latencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// An answer saying a request could not be served.
fn error(message: &str) -> ResponseBody {
    ResponseBody::Error {
        message: message.to_owned(),
    }
}

/// Reads one request, whose whole frame must arrive within [`FRAME_TIMEOUT`]. A client that closes
/// the connection, says nothing for that long, or whose connection fails before a frame begins is
/// done, and that is no error: `None`.
///
/// # Errors
///
/// A frame cut off or late, one too long, and one that is not a request.
async fn read_request(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Option<Received>, FrameError> {
    let deadline = tokio::time::Instant::now() + FRAME_TIMEOUT;
    let Ok(Ok(first)) = tokio::time::timeout_at(deadline, reader.read_u8()).await else {
        return Ok(None);
    };
    let began = Instant::now();
    let rest = async {
        let mut others = [0u8; LENGTH_PREFIX_BYTES - 1];
        reader.read_exact(&mut others).await?;
        let [second, third, fourth] = others;
        let mut body = vec![0u8; body_len([first, second, third, fourth])?];
        reader.read_exact(&mut body).await?;
        decode(&body)
    };
    let request = tokio::time::timeout_at(deadline, rest)
        .await
        .map_err(|_| FrameError::Io(io::ErrorKind::TimedOut.into()))??;
    Ok(Some(Received { request, began }))
}

/// Writes the response to request `request_id`.
async fn write_response(
    writer: &mut (impl AsyncWrite + Unpin),
    request_id: u64,
    body: ResponseBody,
) -> Result<(), FrameError> {
    writer
        .write_all(&encode(&Response { request_id, body })?)
        .await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::DuplexStream;

    use super::*;
    use crate::inbox::Work;
    use crate::troubles::Step;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Runs `read_request` on the reading end of a pipe after `send` has used the other end, on a
    /// runtime whose clock moves on whenever nothing else can, so that a wait of 10 s takes none.
    fn read_after(
        send: impl AsyncFnOnce(&mut DuplexStream) -> io::Result<()>,
    ) -> Result<Result<Option<Received>, FrameError>, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()?;
        runtime.block_on(async {
            let (mut client, mut daemon) = tokio::io::duplex(64);
            send(&mut client).await?;
            let read = read_request(&mut daemon).await;
            drop(client);
            Ok(read)
        })
    }

    /// A client that closes, or that says nothing for 10 s, before a frame begins is done: no
    /// request and no error. A whole frame is a request.
    #[test]
    fn a_frame_not_begun_is_no_error() -> TestResult {
        let closed = read_after(async |client| client.shutdown().await)?;
        assert!(matches!(closed, Ok(None)), "{closed:?}");
        let silent = read_after(async |_| Ok(()))?;
        assert!(matches!(silent, Ok(None)), "{silent:?}");
        let status = encode(&Request {
            request_id: 3,
            body: RequestBody::Status,
        })?;
        let whole = read_after(async |client| client.write_all(&status).await)?;
        let request = whole?.map(|received| received.request);
        assert_eq!(
            request,
            Some(Request {
                request_id: 3,
                body: RequestBody::Status
            })
        );
        Ok(())
    }

    /// Two bytes of a prefix and then a close is a frame cut off; two bytes and then nothing for
    /// 10 s is a late one. Each is kept in words of its own.
    #[test]
    fn a_frame_begun_and_not_finished_is_an_error() -> TestResult {
        let cut = read_after(async |client| {
            client.write_all(&[5, 0]).await?;
            client.shutdown().await
        })?;
        let late = read_after(async |client| client.write_all(&[5, 0]).await)?;
        let messages: Vec<String> = [cut, late]
            .into_iter()
            .map(|read| match read {
                Err(error) => Broken::Unreadable(error).message(),
                Ok(_) => "read".to_owned(),
            })
            .collect();
        assert_eq!(
            messages,
            [
                "a request was cut off: the connection closed in the middle of its frame",
                "a request was cut off: the rest of its frame did not arrive within 10 s",
            ]
        );
        Ok(())
    }

    /// A stop holds though its acknowledgement cannot be written, the client having gone: the
    /// answer is a broken pipe, and the writer is told to stop all the same.
    #[test]
    fn a_stop_holds_when_its_answer_cannot_be_written() -> TestResult {
        let inbox = Arc::new(Inbox::default());
        let server = Server::new(Arc::clone(&inbox), Default::default(), Arc::default());
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let answered = runtime.block_on(async {
            let (client, mut daemon) = tokio::io::duplex(64);
            drop(client);
            let stop = Received {
                request: Request {
                    request_id: 2,
                    body: RequestBody::Shutdown,
                },
                began: Instant::now(),
            };
            server.answer(stop, &mut daemon).await
        });
        assert!(
            matches!(answered, Err(Broken::Unwritable(FrameError::Io(_)))),
            "{answered:?}"
        );
        assert!(matches!(inbox.wait(Some(Instant::now())), Work::Stop));
        Ok(())
    }

    /// A start's reply that holds no text, only failures to tell the user, gives them back when
    /// it cannot be written, the client having gone: the next start takes them again.
    #[test]
    fn failures_in_a_reply_that_never_arrived_are_kept() -> TestResult {
        let inbox = Arc::new(Inbox::default());
        inbox.note_trouble(Step::Mining, "reading the history: bad object", now_ms());
        let server = Server::new(Arc::clone(&inbox), Default::default(), Arc::default());
        let start = cortex_core::Event {
            hook: HookKind::SessionStart,
            ..stop_event()?
        };
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let answered = runtime.block_on(async {
            let (client, mut daemon) = tokio::io::duplex(64);
            drop(client);
            let asked = Received {
                request: Request {
                    request_id: 3,
                    body: RequestBody::Primer {
                        event: Box::new(start),
                        answer_by_ms: None,
                    },
                },
                began: Instant::now(),
            };
            server.answer(asked, &mut daemon).await
        });
        assert!(answered.is_err(), "{answered:?}");
        assert!(inbox.take_troubles(now_ms()).is_some());
        Ok(())
    }

    /// A hook that stops reading at 100 ms has not stopped at 100 ms, has at 101; one that waits
    /// as long as it takes never has.
    #[test]
    fn a_hook_stops_reading_after_its_time() {
        assert!(!is_past(Some(100), 100));
        assert!(is_past(Some(100), 101));
        assert!(!is_past(None, i64::MAX));
    }

    /// Text for a hook that stops reading at 100 ms is sent when ready by 90 ms, 10 ms before, and
    /// not when ready at 91; for one that waits as long as it takes, whenever it is ready.
    #[test]
    fn text_is_sent_ten_milliseconds_before_its_hook_stops_reading() {
        assert!(!is_too_late_to_send(Some(100), 90));
        assert!(is_too_late_to_send(Some(100), 91));
        assert!(!is_too_late_to_send(None, i64::MAX));
    }

    /// A stop event of session `s1`.
    fn stop_event() -> Result<cortex_core::Event, Box<dyn std::error::Error>> {
        Ok(cortex_core::Event {
            event_id: cortex_core::Ulid::from_seed(15, b"stop"),
            ts_ms: 15,
            agent: cortex_core::Agent::Claude,
            agent_version: None,
            session_id: cortex_core::Identifier::new("s1")?,
            parent_session_id: None,
            cwd: std::path::PathBuf::from("/repo"),
            repo_root: std::path::PathBuf::from("/repo"),
            hook: HookKind::Stop,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        })
    }

    /// An event that arrives once the writer has stopped and closed the inbox is answered with an
    /// error, not taken: the writer records nothing after, so an acknowledgement would be a lie.
    /// Nothing is put in the inbox.
    #[test]
    fn a_closed_inbox_takes_no_events() -> TestResult {
        let inbox = Arc::new(Inbox::default());
        inbox.close();
        let server = Server::new(Arc::clone(&inbox), Default::default(), Arc::default());
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let event = Received {
            request: Request {
                request_id: 7,
                body: RequestBody::Event {
                    event: Box::new(stop_event()?),
                },
            },
            began: Instant::now(),
        };
        let written = runtime.block_on(async {
            let (mut client, mut daemon) = tokio::io::duplex(4096);
            let answered = server.answer(event, &mut daemon).await;
            drop(daemon);
            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).await?;
            answered.map_err(|broken| broken.message())?;
            Ok::<_, Box<dyn std::error::Error>>(bytes)
        })?;
        let response: Response = decode(written.get(LENGTH_PREFIX_BYTES..).unwrap_or_default())?;
        assert_eq!(
            response,
            Response {
                request_id: 7,
                body: error("the daemon is stopping and records no more events"),
            }
        );
        assert_eq!(inbox.counts().events_waiting, 0);
        Ok(())
    }

    /// A frame that is not a request is kept as such, never with the JSON error, which quotes the
    /// client's text: here the text a request id cannot be.
    #[test]
    fn an_unreadable_request_keeps_none_of_the_client_s_text() -> TestResult {
        let Err(error) = decode::<Request>(br#"{"request_id":"private words","body":{}}"#) else {
            return Err("the frame was read".into());
        };
        assert_eq!(
            Broken::Unreadable(error).message(),
            "a request could not be read: a frame body is not the message expected"
        );
        let too_long = Broken::Unreadable(FrameError::TooLarge {
            len_bytes: 2_000_000,
        });
        assert_eq!(
            too_long.message(),
            "a request could not be read: a frame body of 2000000 bytes is longer than 1048576"
        );
        let gone = Broken::Unwritable(FrameError::Io(io::ErrorKind::BrokenPipe.into()));
        assert_eq!(
            gone.message(),
            "an answer could not be written: broken pipe"
        );
        Ok(())
    }
}
