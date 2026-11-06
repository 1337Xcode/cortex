//! `cortex hook --agent <agent> --event <event>`: what an agent runs at each of its hooks. The hook
//! reads the payload, turns it into an event, hands the event to its repository's daemon before the
//! event's deadline (starting a daemon in the background when none runs, unless the event is
//! `cortex doctor`'s probe, which starts and stops none), and ends. At a hook where Cortex gives the
//! agent text (a session's start, a prompt) it asks the daemon for that text with the event, and
//! writes it in the agent's hook protocol to the output it was given, when the whole answer came in
//! time. Whatever else happens it writes nothing and the hook exits 0; what went wrong goes to the
//! hook log.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cortex_core::ipc::{RequestBody, ResponseBody, Role, is_older_than_this_build};
use cortex_core::{Event, HookKind, RepoId, RunFiles, Ulid, UserFolder, unix_millis};

use crate::adapter::{Adapter, HookEntry, NormalizeError, Stamp, adapter, shortest_hard_timeout};
use crate::client::{Answer, ClientError, ask};
use crate::fail_open::{self, Outcome};
use crate::stdin::{self, StdinError};
use crate::{log, root, spawn, stop};

/// How long the hook waits for its payload to end, counted from its start.
const STDIN_TIMEOUT: Duration = Duration::from_millis(50);

/// What the hook keeps of an event's hard timeout for itself: the daemon's answer is due this long
/// before the hard timeout, so the hook exits before the agent gives up on it.
const EXIT_MARGIN: Duration = Duration::from_millis(15);

/// How long before an event's hard timeout the work is given up: after the daemon's answer was
/// due ([`EXIT_MARGIN`] before it), the work has the rest to note why it failed.
const WORK_MARGIN: Duration = Duration::from_millis(10);

/// How long before an event's hard timeout a note the hook's own thread writes is given up, once
/// the work was: the process keeps that long to exit.
const NOTE_MARGIN: Duration = Duration::from_millis(5);

/// Why a hook call recorded nothing. Each message names what failed, never the payload's text.
#[derive(Debug, thiserror::Error)]
enum Failure {
    /// The arguments are not the two the installer writes.
    #[error("the arguments are not `--agent <agent> --event <event>`")]
    Arguments,
    /// No adapter has the agent's name.
    #[error("Cortex has no adapter for that agent")]
    UnknownAgent,
    /// The agent's adapter registers no hook of that name.
    #[error("Cortex registers no hook for that event")]
    UnknownEvent,
    /// No payload was read.
    #[error(transparent)]
    Stdin(#[from] StdinError),
    /// The payload made no event.
    #[error(transparent)]
    Payload(#[from] NormalizeError),
    /// The operating system reports no home folder, so the run files cannot be found.
    #[error("the operating system reports no home folder")]
    NoHome,
    /// The daemon could not be asked.
    #[error(transparent)]
    Daemon(#[from] ClientError),
    /// The daemon answered with something other than taking the event, or the text asked for.
    #[error("the daemon did not take the event")]
    NotTaken,
    /// The daemon's text is not one the agent takes as it is: longer than it takes whole, or
    /// holding text it rewrites.
    #[error("the daemon's text is not one the agent takes as it is")]
    Unfit,
    /// No daemon was running, and none could be started.
    #[error("starting the daemon")]
    StartDaemon(#[source] std::io::Error),
    /// A probe, `cortex doctor`'s, found no daemon, and a probe starts none.
    #[error("a probe found no daemon, and probes start none")]
    ProbeWithoutDaemon,
}

/// Runs the hook with `args`, the arguments that follow `hook` on the command line, the payload on
/// `input` (standard input) and `home` the user's home folder, writing to `output` (standard
/// output) the agent's hook output holding the daemon's text when the daemon answered with text
/// for the agent in time, and nothing otherwise. It never fails and never panics past this call,
/// and it returns by the event's hard timeout unless writing to the agent waits on the agent.
pub fn run(
    args: &[OsString],
    input: impl Read + Send + 'static,
    home: Option<&Path>,
    output: &mut dyn Write,
) {
    let hook = Hook {
        started: Instant::now(),
        now: SystemTime::now(),
        home: home.map(Path::to_path_buf),
        names: None,
        hard_timeout: shortest_hard_timeout(),
    };
    let finished = fail_open::guard(AssertUnwindSafe(|| hook.clone().call(args, input, output)));
    if !finished {
        // Noting the panic runs guarded too: nothing may escape to the agent.
        fail_open::guard(AssertUnwindSafe(|| hook.note_in_time("the hook panicked")));
    }
}

/// One hook call: when it started, where the user's home is, and the agent and event once known.
#[derive(Debug, Clone)]
struct Hook {
    /// When the hook started, for its deadlines.
    started: Instant,
    /// The clock at the start, for the event's time and id.
    now: SystemTime,
    /// The user's home folder, if the operating system reports one.
    home: Option<PathBuf>,
    /// The agent's and the event's names, once the arguments name ones Cortex knows.
    names: Option<(&'static str, &'static str)>,
    /// How long the call may take: the event's hard timeout once the arguments name a hook, the
    /// shortest of any hook's until then.
    hard_timeout: Duration,
}

impl Hook {
    /// Reads the arguments, then does the rest on a thread of its own, waited for no longer than
    /// the event's hard timeout, and writes what it gives the agent to `output`.
    fn call(
        mut self,
        args: &[OsString],
        input: impl Read + Send + 'static,
        output: &mut dyn Write,
    ) {
        let (adapter, entry) = match hook_of(args) {
            Ok(found) => found,
            Err(failure) => return self.note_in_time(&describe(&failure)),
        };
        self.names = Some((adapter.agent().name(), entry.event_name));
        self.hard_timeout = entry.hard_timeout;
        let worker = self.clone();
        // The whole of the work, file system calls included, ends by the event's hard timeout,
        // less the moment kept to note why when it does not finish.
        let work_until = self.started + entry.hard_timeout.saturating_sub(WORK_MARGIN);
        let outcome = fail_open::within(work_until, move || {
            worker
                .handle(adapter, entry, input)
                .unwrap_or_else(|failure| {
                    worker.note(&describe(&failure));
                    None
                })
        });
        match outcome {
            Outcome::Finished(Some(given)) => self.give(&given, output),
            Outcome::Finished(None) => {}
            Outcome::Panicked => self.note_in_time("the hook panicked"),
            Outcome::OutOfTime => self.note_in_time("the hook ran out of time"),
        }
    }

    /// Writes `given`, the agent's hook output, to `output`, then a line break. It is written from
    /// the hook's own thread once the work is done, never by work given up at its deadline: a write
    /// cut short by the process's exit would leave the agent part of an object, which it would
    /// read as plain text. A write that fails is noted.
    fn give(&self, given: &str, output: &mut dyn Write) {
        if let Err(error) = writeln!(output, "{given}").and_then(|()| output.flush()) {
            self.note_in_time(&format!("writing to the agent failed: {error}"));
        }
    }

    /// Reads the payload from `input`, normalizes it, and hands the event to the daemon, asking
    /// for its text where Cortex gives the agent some; returns the agent's hook output holding the
    /// text the daemon answered with.
    fn handle(
        &self,
        adapter: &'static dyn Adapter,
        entry: &'static HookEntry,
        input: impl Read + Send + 'static,
    ) -> Result<Option<String>, Failure> {
        let payload = stdin::read_payload(input, self.started + STDIN_TIMEOUT)?;
        let home = self.home.as_deref();
        let locate = |cwd: &Path| root::locate(cwd, home);
        let Some(event) =
            adapter.normalize(entry.event_name, &payload, &locate, stamp(self.now))?
        else {
            return Ok(None);
        };
        let user = UserFolder::in_home(home.ok_or(Failure::NoHome)?);
        let files = user.run_files(&RepoId::of(&event.repo_root));
        let root = event.repo_root.clone();
        let is_probe = event.is_probe;
        let body = request_of(entry, event);
        let asks_for_text = !matches!(body, RequestBody::Event { .. });
        let deadline = deadline(self.started, entry);
        let answer = ask(&files, Role::Hook, body, deadline);
        if is_probe {
            return probed(answer, asks_for_text).map(|()| None);
        }
        match answer {
            Ok(answer) => {
                let text = text_of(answer.body, asks_for_text)?;
                // The answer came: a stop that fails is noted, and the text given all the same.
                if let Err(failure) = stop_older(&files, &answer.welcome.cortex_version, deadline) {
                    self.note(&describe(&failure));
                }
                // INVARIANT: silent-when-unsure. An empty text gives the agent nothing, not an
                // empty context.
                text.filter(|text| !text.is_empty())
                    .map(|text| adapter.context_output(entry, &text).ok_or(Failure::Unfit))
                    .transpose()
            }
            // No daemon: start one and go on without it. The event that found none is not kept;
            // the next hook finds the daemon listening.
            Err(ClientError::NotRunning) => {
                let program = std::env::current_exe().map_err(Failure::StartDaemon)?;
                // A start made by another hook moments ago is under way: nothing to do.
                spawn::start_daemon(&program, &root, &files, self.now)
                    .map(|_| None)
                    .map_err(Failure::StartDaemon)
            }
            // A daemon of another protocol took nothing; an older one is still told to stop.
            Err(ClientError::Mismatch(theirs)) => {
                stop_older(&files, &theirs.cortex_version, deadline)?;
                Err(ClientError::Mismatch(theirs).into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Writes `what` to the hook log as [`Hook::note`] does, from the hook's own thread, which must
    /// return by the hard timeout: the note is written on a thread of its own and given up
    /// [`NOTE_MARGIN`] before it, left to the process's exit like stalled work, since a file
    /// system that stalled the work may stall the log too.
    fn note_in_time(&self, what: &str) {
        let (hook, what) = (self.clone(), what.to_owned());
        let note_until = self.started + self.hard_timeout.saturating_sub(NOTE_MARGIN);
        // INVARIANT: fail-open. The log never holds the agent past the hook's hard timeout.
        let _ = fail_open::within(note_until, move || hook.note(&what));
    }

    /// Writes `what` to the hook log with the call's time, agent and event. A log that cannot be
    /// written is left unwritten: nothing else can be told. It may wait on the file system as long
    /// as that takes: the work calls it within its own deadline, and the hook's own thread through
    /// [`Hook::note_in_time`].
    fn note(&self, what: &str) {
        let Some(home) = self.home.as_deref() else {
            return;
        };
        let (agent, event) = self.names.unwrap_or(("-", "-"));
        let line = format!("{} {agent} {event}: {what}", unix_millis(self.now));
        let folder = UserFolder::in_home(home).logs_folder();
        drop(log::write(&folder, SystemTime::now(), &line));
    }
}

/// Asks the daemon whose run files are `files`, which answered as Cortex `version`, to stop when
/// that is older than this hook, before `deadline`: the next hook then finds none and starts one
/// of this build. The stop goes on a connection of its own whose handshake is checked again, so a
/// daemon that took the older one's place meanwhile is left running.
fn stop_older(files: &RunFiles, version: &str, deadline: Instant) -> Result<(), Failure> {
    if !is_older_than_this_build(version) {
        return Ok(());
    }
    stop::stop_if_older(files, Role::Hook, deadline)?;
    Ok(())
}

/// What became of a probe, `cortex doctor`'s, by the daemon's `answer` to a request that asked
/// for text or not. A probe measures the daemon that runs and changes nothing: it never asks an
/// older one to stop, nor starts one when none runs, and the text it gets is not given.
fn probed(answer: Result<Answer, ClientError>, asks_for_text: bool) -> Result<(), Failure> {
    match answer {
        Ok(answer) => text_of(answer.body, asks_for_text).map(drop),
        Err(ClientError::NotRunning) => Err(Failure::ProbeWithoutDaemon),
        Err(error) => Err(error.into()),
    }
}

/// The request the hook makes of the daemon with `event` at the hook `entry`: the text Cortex
/// gives the agent there, where it gives some (the primer at a session's start, the work order at
/// a prompt, the warnings after an edit), saying when the hook stops reading the answer; the event
/// alone otherwise.
fn request_of(entry: &HookEntry, event: Event) -> RequestBody {
    // The event's time is the clock at the hook's start, from which its deadline counts.
    let wait = entry.hard_timeout.saturating_sub(EXIT_MARGIN);
    let answer_by_ms = Some(
        event
            .ts_ms
            .saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX)),
    );
    let event = Box::new(event);
    match entry.hook {
        HookKind::SessionStart if entry.gives_context => RequestBody::Primer {
            event,
            answer_by_ms,
        },
        HookKind::Prompt if entry.gives_context => RequestBody::Retrieve {
            event,
            answer_by_ms,
        },
        HookKind::PostTool if entry.gives_context && event.edited_file().is_some() => {
            RequestBody::PostEdit {
                event,
                answer_by_ms,
            }
        }
        _ => RequestBody::Event { event },
    }
}

/// The text in the daemon's answer `body` to a request that asked for text or not: none for the
/// event taken, or when the daemon has no text to give.
///
/// # Errors
///
/// The answer is not the one the request asks for: an error, or another kind of answer.
fn text_of(body: ResponseBody, asks_for_text: bool) -> Result<Option<String>, Failure> {
    match (body, asks_for_text) {
        (ResponseBody::Ack, false) => Ok(None),
        (ResponseBody::Reply { text }, true) => Ok(text),
        _ => Err(Failure::NotTaken),
    }
}

/// The adapter and the hook the arguments name.
fn hook_of(args: &[OsString]) -> Result<(&'static dyn Adapter, &'static HookEntry), Failure> {
    let (agent, event_name) = parse(args)?;
    let adapter = adapter(agent).ok_or(Failure::UnknownAgent)?;
    let entry = adapter
        .hooks()
        .iter()
        .find(|entry| entry.event_name == event_name)
        .ok_or(Failure::UnknownEvent)?;
    Ok((adapter, entry))
}

/// The agent's and the event's names from `--agent <agent> --event <event>`, in either order.
fn parse(args: &[OsString]) -> Result<(&str, &str), Failure> {
    let (mut agent, mut event) = (None, None);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .and_then(|value| value.to_str())
            .ok_or(Failure::Arguments)?;
        match flag.to_str() {
            Some("--agent") if agent.is_none() => agent = Some(value),
            Some("--event") if event.is_none() => event = Some(value),
            _ => return Err(Failure::Arguments),
        }
    }
    agent.zip(event).ok_or(Failure::Arguments)
}

/// When the daemon's answer is due: the event's hard timeout after the hook's start, less
/// [`EXIT_MARGIN`].
fn deadline(started: Instant, entry: &HookEntry) -> Instant {
    started + entry.hard_timeout.saturating_sub(EXIT_MARGIN)
}

/// The event's id and time: the clock at the start, and an id whose random part is BLAKE3 of the
/// process id and the clock's nanoseconds, so that hooks started in the same millisecond differ.
fn stamp(now: SystemTime) -> Stamp {
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let mut seed = std::process::id().to_le_bytes().to_vec();
    seed.extend_from_slice(&since_epoch.as_nanos().to_le_bytes());
    let ts_ms = unix_millis(now);
    Stamp {
        event_id: Ulid::from_seed(u64::try_from(ts_ms).unwrap_or(0), &seed),
        ts_ms,
    }
}

/// What went wrong, with the operating system's reason for a failed read or connection. A
/// payload's parse error is not followed: its message may quote the payload.
fn describe(failure: &Failure) -> String {
    let mut text = failure.to_string();
    let io_source = match failure {
        Failure::Stdin(StdinError::Io(error))
        | Failure::Daemon(ClientError::Io(error))
        | Failure::StartDaemon(error) => Some(error),
        _ => None,
    };
    if let Some(error) = io_source {
        text.push_str(": ");
        text.push_str(&error.to_string());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments.
    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// The two flags are read in either order; anything else, missing, repeated or extra, is
    /// refused.
    #[test]
    fn only_the_two_flags_are_read() {
        let named = |list: &[&str]| {
            parse(&args(list))
                .ok()
                .map(|(a, e)| (a.to_owned(), e.to_owned()))
        };
        let expected = Some(("claude".to_owned(), "Stop".to_owned()));
        assert_eq!(named(&["--agent", "claude", "--event", "Stop"]), expected);
        assert_eq!(named(&["--event", "Stop", "--agent", "claude"]), expected);
        for refused in [
            &["--agent", "claude"][..],
            &["--agent", "claude", "--event"],
            &["--agent", "claude", "--event", "Stop", "--extra", "x"],
            &["--agent", "claude", "--agent", "codex", "--event", "Stop"],
            &["--help"],
            &[],
        ] {
            assert_eq!(named(refused), None, "{refused:?}");
        }
    }

    /// The daemon's answer is due 15 ms before the hard timeout: 485 ms after the start for a
    /// tool's result, 285 ms before a read, 1,485 ms at a session's start.
    #[test]
    fn the_answer_is_due_before_the_hard_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let started = Instant::now();
        for (event, due_ms) in [
            ("PostToolUse", 485),
            ("PreToolUse", 285),
            ("SessionStart", 1_485),
        ] {
            let (_, entry) = hook_of(&args(&["--agent", "claude", "--event", event]))
                .map_err(|failure| failure.to_string())?;
            assert_eq!(
                deadline(started, entry),
                started + Duration::from_millis(due_ms),
                "{event}"
            );
        }
        Ok(())
    }

    /// A tool's result at 1,000 ms that `tool` gave, succeeding or not, on `calc/ops.py`.
    fn tool_result(
        tool: cortex_core::ToolKind,
        is_success: bool,
    ) -> Result<Event, Box<dyn std::error::Error>> {
        Ok(Event {
            event_id: Ulid::from_seed(1_000, b"tool"),
            ts_ms: 1_000,
            agent: cortex_core::Agent::Claude,
            agent_version: None,
            session_id: cortex_core::Identifier::new("s1")?,
            parent_session_id: None,
            cwd: PathBuf::from("/repo"),
            repo_root: PathBuf::from("/repo"),
            hook: HookKind::PostTool,
            source: None,
            tool: Some(tool),
            tool_target: Some(cortex_core::ToolTarget {
                path: Some(cortex_core::RepoPath::new("calc/ops.py")?),
                ..cortex_core::ToolTarget::default()
            }),
            tool_result: Some(cortex_core::ToolResult {
                exit_code: None,
                is_success,
                is_interrupted: false,
                output_bytes: None,
            }),
            prompt_text: None,
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        })
    }

    /// After a successful edit Claude Code's `PostToolUse` asks for the edit's warnings, to be read
    /// by 1,485 ms (485 ms after the event's 1,000); after a read it reports the event alone, and so
    /// does `PostToolUseFailure`, whose edit failed.
    #[test]
    fn an_edit_asks_for_its_warnings() -> Result<(), Box<dyn std::error::Error>> {
        use cortex_core::ToolKind;
        let entry = |event: &str| -> Result<&'static HookEntry, Box<dyn std::error::Error>> {
            Ok(hook_of(&args(&["--agent", "claude", "--event", event]))
                .map_err(|failure| failure.to_string())?
                .1)
        };
        let edit = tool_result(ToolKind::Edit, true)?;
        assert_eq!(
            request_of(entry("PostToolUse")?, edit.clone()),
            RequestBody::PostEdit {
                event: Box::new(edit),
                answer_by_ms: Some(1_485),
            }
        );
        let read = tool_result(ToolKind::Read, true)?;
        assert_eq!(
            request_of(entry("PostToolUse")?, read.clone()),
            RequestBody::Event {
                event: Box::new(read)
            }
        );
        let failed = tool_result(ToolKind::Edit, false)?;
        assert_eq!(
            request_of(entry("PostToolUseFailure")?, failed.clone()),
            RequestBody::Event {
                event: Box::new(failed)
            }
        );
        Ok(())
    }

    /// An event's time is the start's, in milliseconds; two hooks started in the same millisecond
    /// get different ids.
    #[test]
    fn the_stamp_is_the_start_time() {
        let now = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let a_little_later = now + Duration::from_micros(250);
        assert_eq!(stamp(now).ts_ms, 1_700_000_000_123);
        assert_eq!(stamp(a_little_later).ts_ms, 1_700_000_000_123);
        assert_ne!(stamp(now).event_id, stamp(a_little_later).event_id);
    }
}
