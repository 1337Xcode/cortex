//! `cortex hook --agent <agent> --event <event>`: what an agent runs at each of its hooks. The hook
//! reads the payload, turns it into an event, hands the event to its repository's daemon before the
//! event's deadline (starting a daemon in the background when none runs), and ends. Whatever
//! happens it prints nothing and exits 0; what went wrong goes to the hook log.

use std::ffi::OsString;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cortex_core::ipc::{RequestBody, ResponseBody, Role, is_older_than_this_build};
use cortex_core::{RepoId, RunFiles, Ulid, UserFolder, unix_millis};

use crate::adapter::{Adapter, HookEntry, NormalizeError, Stamp, adapter};
use crate::client::{ClientError, ask};
use crate::fail_open::{self, Outcome};
use crate::stdin::{self, StdinError};
use crate::{log, root, spawn, stop};

/// How long the hook waits for its payload to end, counted from its start.
const STDIN_TIMEOUT: Duration = Duration::from_millis(50);

/// What the hook keeps of an event's hard timeout for itself: the daemon's answer is due this long
/// before the hard timeout, so the hook exits before the agent gives up on it.
const EXIT_MARGIN: Duration = Duration::from_millis(15);

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
    /// The daemon answered with something other than taking the event.
    #[error("the daemon did not take the event")]
    NotTaken,
    /// No daemon was running, and none could be started.
    #[error("starting the daemon")]
    StartDaemon(#[source] std::io::Error),
}

/// Runs the hook with `args`, the arguments that follow `hook` on the command line, the payload on
/// `input` (standard input) and `home` the user's home folder. It never fails, never panics past
/// this call, never prints, and returns by the event's hard timeout.
pub fn run(args: &[OsString], input: impl Read + Send + 'static, home: Option<&Path>) {
    let hook = Hook {
        started: Instant::now(),
        now: SystemTime::now(),
        home: home.map(Path::to_path_buf),
        names: None,
    };
    let finished = fail_open::guard(AssertUnwindSafe(|| hook.clone().call(args, input)));
    if !finished {
        // Noting the panic runs guarded too: nothing may escape to the agent.
        fail_open::guard(AssertUnwindSafe(|| hook.note("the hook panicked")));
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
}

impl Hook {
    /// Reads the arguments, then does the rest on a thread of its own, waited for no longer than
    /// the event's hard timeout.
    fn call(mut self, args: &[OsString], input: impl Read + Send + 'static) {
        let (adapter, entry) = match hook_of(args) {
            Ok(found) => found,
            Err(failure) => return self.note(&describe(&failure)),
        };
        self.names = Some((adapter.agent().name(), entry.event_name));
        let worker = self.clone();
        // The whole of the work, file system calls included, ends by the event's hard timeout.
        let outcome = fail_open::within(self.started + entry.hard_timeout, move || {
            if let Err(failure) = worker.handle(adapter, entry, input) {
                worker.note(&describe(&failure));
            }
        });
        match outcome {
            Outcome::Finished => {}
            Outcome::Panicked => self.note("the hook panicked"),
            Outcome::OutOfTime => self.note("the hook ran out of time"),
        }
    }

    /// Reads the payload from `input`, normalizes it, and hands the event to the daemon.
    fn handle(
        &self,
        adapter: &'static dyn Adapter,
        entry: &'static HookEntry,
        input: impl Read + Send + 'static,
    ) -> Result<(), Failure> {
        let payload = stdin::read_payload(input, self.started + STDIN_TIMEOUT)?;
        let home = self.home.as_deref();
        let locate = |cwd: &Path| root::locate(cwd, home);
        let Some(event) =
            adapter.normalize(entry.event_name, &payload, &locate, stamp(self.now))?
        else {
            return Ok(());
        };
        let user = UserFolder::in_home(home.ok_or(Failure::NoHome)?);
        let files = user.run_files(&RepoId::of(&event.repo_root));
        let root = event.repo_root.clone();
        let body = RequestBody::Event {
            event: Box::new(event),
        };
        let deadline = deadline(self.started, entry);
        match ask(&files, Role::Hook, body, deadline) {
            Ok(answer) if answer.body == ResponseBody::Ack => {
                stop_older(&files, &answer.welcome.cortex_version, deadline)
            }
            Ok(_) => Err(Failure::NotTaken),
            // No daemon: start one and go on without it. The event that found none is not kept;
            // the next hook finds the daemon listening.
            Err(ClientError::NotRunning) => {
                let program = std::env::current_exe().map_err(Failure::StartDaemon)?;
                // A start made by another hook moments ago is under way: nothing to do.
                spawn::start_daemon(&program, &root, &files, self.now)
                    .map(drop)
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

    /// Writes `what` to the hook log with the call's time, agent and event. A log that cannot be
    /// written is left unwritten: nothing else can be told.
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
