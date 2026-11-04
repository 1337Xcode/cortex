//! The co-change miner: a thread of its own, started once the daemon's first index is done, that
//! looks at HEAD every [`HEAD_EVERY`] and mines the history when a mining is due (nothing mined
//! yet, HEAD moved, or the last full mining a week old). A HEAD found due is mined at a later look,
//! once the index has taken every change that had reached the inbox by then, so that the files a
//! checkout wrote as HEAD moved count. What the mining found goes to the writer through the inbox,
//! and the miner waits for the write before it looks again, so that the mark it reads next is the
//! one written. A look that finds HEAD moved also tells the work tree's reader, which lets go of
//! the diff it read and looks again: a commit changes which files are uncommitted without writing
//! a file the watcher would report.
//!
//! `gix` reads the history, opened afresh for each read ([`source`]); the `git` program reads it
//! once `gix` cannot, and then the miner looks every [`PROGRAM_HEAD_EVERY`] while nothing is due,
//! since each look runs git. A failure, a panic in a look among them, holds the next try back
//! ([`retry`]). Where the miner is goes to the inbox, for the daemon's status.
//!
//! Told to stop, the miner ends at its next look. A read of the history under way is not cut
//! short: the miner writes nothing itself, and the daemon's process ends without waiting for it;
//! a `git` it runs then ends at its next write to the closed pipe, or at its time limit.

mod retry;
mod source;
#[cfg(test)]
mod tests;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use cortex_core::Config;
use cortex_core::ipc::{GitReport, GitState};
use cortex_git::mine::{self, Mark};
use cortex_store::Reader;

use self::retry::Retry;
use self::source::{Program, read_history};
use crate::host::now_ms;
use crate::inbox::Inbox;
use crate::worktree::Worktree;

/// How often the miner looks whether HEAD moved: reading HEAD's commit with `gix` takes about 2 ms,
/// and a commit is mined within seconds of being made. One look apart is also time enough for the
/// watcher to report the files a move of HEAD wrote: it reports a file a fraction of a second
/// after it changes.
const HEAD_EVERY: Duration = Duration::from_secs(2);

/// How often the miner looks at HEAD while the `git` program reads the history and no mining waits
/// for the index: each look then runs git, about 50 ms on Windows, so a look every 30 s costs under
/// 0.2% of a core, and a commit is still mined within the minute.
const PROGRAM_HEAD_EVERY: Duration = Duration::from_secs(30);

/// The miner's thread, told to stop when this is dropped.
pub(crate) struct Miner {
    /// Dropped to tell the thread to stop.
    _stop: Sender<()>,
}

impl Miner {
    /// Starts mining the history of the repository at `root` under `config`, the store read from
    /// its `.cortex` folder and what is found handed to `inbox`, telling `worktree` whenever HEAD
    /// moves. A thread that cannot be started is logged, kept among the
    /// daemon's recent errors and reported as a failure: the daemon goes on without co-change.
    pub(crate) fn start(
        root: &Path,
        config: &Config,
        inbox: &Arc<Inbox>,
        worktree: &Arc<Worktree>,
    ) -> Option<Self> {
        let (stop, stopped) = mpsc::channel();
        let mut looker = Looker::new(
            root.to_path_buf(),
            config.clone(),
            Arc::clone(inbox),
            Arc::clone(worktree),
        );
        let started = std::thread::Builder::new()
            .name("cortex-miner".to_owned())
            .spawn(move || looker.run(&stopped));
        match started {
            Ok(_) => Some(Self { _stop: stop }),
            Err(error) => {
                let why = format!("the co-change miner could not start: {error}");
                tracing::warn!(error = %why, "no history is mined");
                inbox.note_error(&why, now_ms());
                inbox.set_git(GitReport {
                    state: GitState::Failed,
                    detail: Some(why),
                });
                None
            }
        }
    }
}

/// A HEAD due whose mining waits for the index.
struct Settling {
    /// HEAD's commit, empty when its branch has none.
    head: String,
    /// How many changes had reached the inbox at the look after the one that found HEAD due: the
    /// index takes them before the mining reads which files count. Unset until that look.
    changes: Option<u64>,
}

/// A mining handed to the writer.
struct Handed {
    /// HEAD's commit when it was mined.
    head: String,
    /// Where the writer says whether it stored the mining, or why not.
    answer: Receiver<Result<(), String>>,
}

/// What a look leads to.
enum Next {
    /// Look again later.
    Wait,
    /// Wait for the writer to store a mining, then look again later.
    Handed(Handed),
    /// Stop: the writer takes no more.
    Stop,
}

/// A look that failed.
struct Failure {
    /// Why, in words.
    why: String,
    /// HEAD's commit, when it was read before the failure.
    head: Option<String>,
}

impl Failure {
    /// A failure before HEAD was read, for `why`.
    fn blind(why: String) -> Self {
        Self { why, head: None }
    }
}

/// The miner's state between looks.
struct Looker {
    /// The repository's root folder.
    root: PathBuf,
    /// The settings in effect.
    config: Config,
    /// Where minings go, and where the miner's state is reported.
    inbox: Arc<Inbox>,
    /// The store, read-only, once opened.
    reader: Option<Reader>,
    /// The `git` program, once `gix` could not read the history.
    program: Option<Program>,
    /// A HEAD due whose mining waits for the index.
    settling: Option<Settling>,
    /// The failures in a row.
    retry: Retry,
    /// The work tree's reader, told whenever HEAD moves.
    worktree: Arc<Worktree>,
    /// HEAD's commit at the latest look that read it, empty when its branch has none.
    head_seen: Option<String>,
}

impl Looker {
    /// A miner of the repository at `root` under `config`, handing what it finds to `inbox` and
    /// telling `worktree` whenever HEAD moves, that has not looked yet.
    fn new(root: PathBuf, config: Config, inbox: Arc<Inbox>, worktree: Arc<Worktree>) -> Self {
        Self {
            root,
            config,
            inbox,
            reader: None,
            program: None,
            settling: None,
            retry: Retry::default(),
            worktree,
            head_seen: None,
        }
    }

    /// Looks until told to stop by `stopped`, or until the writer takes no more.
    fn run(&mut self, stopped: &Receiver<()>) {
        loop {
            let mut next = self.guarded(Instant::now(), now_ms(), Self::look);
            if let Next::Handed(handed) = next {
                let answer = handed.answer.recv();
                next = self.written(handed.head, answer, Instant::now(), now_ms());
            }
            if let Next::Stop = next {
                return;
            }
            match stopped.recv_timeout(self.look_every()) {
                Err(RecvTimeoutError::Timeout) => {}
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// How long until the next look: [`PROGRAM_HEAD_EVERY`] while the `git` program reads the
    /// history and no HEAD waits for the index, else [`HEAD_EVERY`].
    fn look_every(&self) -> Duration {
        if self.program.is_some() && self.settling.is_none() {
            PROGRAM_HEAD_EVERY
        } else {
            HEAD_EVERY
        }
    }

    /// Runs `look` at `now` (for the pauses) and `wall_ms` (UTC milliseconds, for the week and the
    /// errors' times). A panic in it, as a library reading the repository's untrusted content may
    /// meet, is a failure like any other rather than the end of the miner; the daemon's panic hook
    /// logs where it happened.
    fn guarded(
        &mut self,
        now: Instant,
        wall_ms: i64,
        look: impl FnOnce(&mut Self, Instant, i64) -> Next,
    ) -> Next {
        let looked = std::panic::catch_unwind(AssertUnwindSafe(|| look(self, now, wall_ms)));
        looked.unwrap_or_else(|_| {
            // The look stopped part way: a HEAD waiting for the index waits from the start again.
            self.settling = None;
            let why = "a look stopped on an internal error, logged as a panic".to_owned();
            self.fail(Failure::blind(why), now, wall_ms);
            Next::Wait
        })
    }

    /// One look at `now` and `wall_ms`: at HEAD, and at the history when a mining is due. A
    /// failure is reported, logged and kept among the daemon's recent errors, and holds the next
    /// try back.
    fn look(&mut self, now: Instant, wall_ms: i64) -> Next {
        self.try_look(now, wall_ms).unwrap_or_else(|failure| {
            self.fail(failure, now, wall_ms);
            Next::Wait
        })
    }

    /// One look at `now` and `wall_ms`, or why it failed.
    fn try_look(&mut self, now: Instant, wall_ms: i64) -> Result<Next, Failure> {
        if self.retry.holds_back(None, now) {
            return Ok(Next::Wait);
        }
        let head = read_history(&self.root, &mut self.program, |history| history.head())
            .map_err(Failure::blind)?;
        let Some(head) = head else {
            self.settling = None;
            self.retry = Retry::default();
            self.report(GitState::NoRepository, None);
            return Ok(Next::Wait);
        };
        let head = head.unwrap_or_default();
        self.see_head(&head);
        let mark = self.mark().map_err(Failure::blind)?;
        if !mine::is_due(mark.as_ref(), &head, wall_ms) {
            self.settling = None;
            self.retry = Retry::default();
            self.report(GitState::Ready, self.detail());
            return Ok(Next::Wait);
        }
        if self.retry.holds_back(Some(&head), now) {
            return Ok(Next::Wait);
        }
        self.report(GitState::Mining, self.detail());
        if !self.is_settled(&head) {
            return Ok(Next::Wait);
        }
        self.settling = None;
        let failed = |why: String| Failure {
            why,
            head: Some(head.clone()),
        };
        let files = self
            .reader()
            .and_then(|reader| {
                reader
                    .queries()
                    .cochange_files()
                    .map_err(|error| format!("reading the store: {error}"))
            })
            .map_err(failed)?;
        let (root, config) = (&self.root, &self.config);
        let mined = read_history(root, &mut self.program, |history| {
            mine::mine(history, mark.as_ref(), &files, config, wall_ms)
        })
        .map_err(failed)?;
        let Some(mining) = mined else {
            // The repository went away since HEAD was read: the next look says so.
            return Ok(Next::Wait);
        };
        Ok(match self.inbox.push_mining(mining) {
            Some(answer) => Next::Handed(Handed { head, answer }),
            None => Next::Stop,
        })
    }

    /// Tells the work tree's reader that HEAD moved when HEAD's commit, `head`, is not the one the
    /// latest look read, the first look's among them, since the work tree may have been read at
    /// another HEAD before the miner began.
    fn see_head(&mut self, head: &str) {
        if self.head_seen.as_deref() != Some(head) {
            self.head_seen = Some(head.to_owned());
            self.worktree.head_moved();
        }
    }

    /// Whether the index has taken the files a move of HEAD to `head` wrote, so that a mining now
    /// counts the files the move created: HEAD was `head` at the look before, and every change
    /// that had reached the inbox by the first look after that one is indexed. A file the watcher
    /// reports later than that look is not waited for; the next full mining counts it.
    fn is_settled(&mut self, head: &str) -> bool {
        let Some(settling) = self
            .settling
            .as_mut()
            .filter(|settling| settling.head == head)
        else {
            self.settling = Some(Settling {
                head: head.to_owned(),
                changes: None,
            });
            return false;
        };
        let inbox = &self.inbox;
        let changes = *settling.changes.get_or_insert_with(|| inbox.changes_in());
        inbox.has_indexed(changes)
    }

    /// Takes the writer's `answer` on the mining of `head` it was handed, come at `now` and
    /// `wall_ms`, and reports how the write went; no answer means the writer stopped and dropped
    /// the mining, and the miner stops too.
    fn written(
        &mut self,
        head: String,
        answer: Result<Result<(), String>, RecvError>,
        now: Instant,
        wall_ms: i64,
    ) -> Next {
        match answer {
            Ok(Ok(())) => {
                self.retry = Retry::default();
                self.report(GitState::Ready, self.detail());
                Next::Wait
            }
            Ok(Err(why)) => {
                let failure = Failure {
                    why: format!("writing what was mined: {why}"),
                    head: Some(head),
                };
                self.fail(failure, now, wall_ms);
                Next::Wait
            }
            Err(RecvError) => Next::Stop,
        }
    }

    /// Reports `failure`, met at `now` and `wall_ms`, logs it, keeps it among the daemon's recent
    /// errors, and holds the next try back.
    fn fail(&mut self, failure: Failure, now: Instant, wall_ms: i64) {
        tracing::warn!(error = %failure.why, "mining the git history failed");
        self.inbox.note_error(
            &format!("mining the git history failed: {}", failure.why),
            wall_ms,
        );
        self.retry.failed(failure.head, now);
        self.report(GitState::Failed, Some(failure.why));
    }

    /// The mark the last mining left in the store.
    fn mark(&mut self) -> Result<Option<Mark>, String> {
        Mark::read(self.reader()?.queries()).map_err(|error| format!("reading the store: {error}"))
    }

    /// The store, opened read-only on first use.
    fn reader(&mut self) -> Result<&Reader, String> {
        if self.reader.is_none() {
            let folder = self.root.join(".cortex");
            let reader = Reader::open(&folder, self.config.mmap_bytes)
                .map_err(|error| format!("reading the store: {error}"))?;
            self.reader = Some(reader);
        }
        self.reader
            .as_ref()
            .ok_or_else(|| "no store is open".to_owned())
    }

    /// More about how the history is read, when the `git` program reads it.
    fn detail(&self) -> Option<String> {
        self.program.as_ref().map(Program::detail)
    }

    /// Reports the miner's `state` and `detail` for the daemon's status.
    fn report(&self, state: GitState, detail: Option<String>) {
        self.inbox.set_git(GitReport { state, detail });
    }
}
