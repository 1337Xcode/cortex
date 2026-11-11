//! The hook as an agent runs it, against a scripted daemon: what it makes of each answer, seen in
//! what it writes for the agent and in the hook log, where it says what went wrong.

mod common;

use std::ffi::OsString;
use std::io::{self, Cursor, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{Script, TestResult, response, run_files_of, serve, welcome};
use cortex_core::ipc::{Request, RequestBody, ResponseBody};
use cortex_core::{HookKind, RunFiles, UserFolder, canonical_root};
use serde_json::{Map, Value, json};

/// The hook log under `home`, empty when it was never written.
fn hook_log(home: &Path) -> TestResult<String> {
    let log = UserFolder::in_home(home).logs_folder().join("hook.log");
    match std::fs::read_to_string(log) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// `cortex hook` arguments for Claude Code's `event`.
fn args(event: &str) -> Vec<OsString> {
    ["--agent", "claude", "--event", event]
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Runs the hook with `args`, the payload `input` and `home` the home folder, and returns what it
/// wrote for the agent. The hook silences panics for the whole process; the default panic hook is
/// put back after it, so that an assertion failing in these tests says why.
fn hook(args: &[OsString], input: Vec<u8>, home: &Path) -> TestResult<String> {
    let mut written = Vec::new();
    cortex_hook::run(args, Cursor::new(input), Some(home), &mut written);
    drop(std::panic::take_hook());
    Ok(String::from_utf8(written)?)
}

/// A daemon that answers an event with an error, rather than taking it, has not taken it: the hook
/// notes that in its log and writes nothing. So it does for `cortex doctor`'s probe, which is
/// answered the same way.
#[test]
fn an_answer_that_takes_nothing_is_noted() -> TestResult {
    for is_probe in [false, true] {
        let (home, repo, files) = marked_repository()?;
        let daemon = serve(
            &files,
            Script::Answer(vec![
                response(1, ResponseBody::Welcome(welcome())),
                response(
                    2,
                    ResponseBody::Error {
                        message: "the daemon is stopping".to_owned(),
                    },
                ),
            ]),
        )?;
        let written = hook(
            &args("Stop"),
            payload("Stop", &repo, is_probe)?,
            home.path(),
        )?;
        daemon.join().map_err(|_| "the daemon thread panicked")??;
        assert_eq!(written, "");
        let log = hook_log(home.path())?;
        assert!(
            log.contains("claude Stop: the daemon did not take the event"),
            "probe {is_probe}: {log:?}"
        );
    }
    Ok(())
}

/// A home folder holding a repository marked for Cortex, the repository, and the run files of its
/// daemon, which this process plays.
fn marked_repository() -> TestResult<(tempfile::TempDir, std::path::PathBuf, RunFiles)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join(".cortex"))?;
    cortex_core::record_set_up(&UserFolder::in_home(home.path()), &canonical_root(&repo)?)?;
    let files = run_files_of(home.path(), &canonical_root(&repo)?)?;
    Ok((home, repo, files))
}

/// A Claude Code payload of `event` from session `s1` in the repository at `repo`, marked as a
/// probe when `is_probe`: a session's start from a new session, a prompt with its words.
fn payload(event: &str, repo: &Path, is_probe: bool) -> TestResult<Vec<u8>> {
    let mut fields = Map::new();
    fields.insert("session_id".to_owned(), json!("s1"));
    fields.insert("hook_event_name".to_owned(), json!(event));
    fields.insert("cwd".to_owned(), json!(repo));
    fields.insert("cortex_probe".to_owned(), json!(is_probe));
    if event == "SessionStart" {
        fields.insert("source".to_owned(), json!("startup"));
    }
    if event == "UserPromptSubmit" {
        fields.insert("prompt".to_owned(), json!("fix create_order"));
    }
    Ok(serde_json::to_vec(&Value::Object(fields))?)
}

/// What the hook writes for Claude Code's `event` from a repository of its own, a probe when
/// `is_probe`, when the repository's daemon welcomes it and answers its request with `body`; the
/// request the daemon read; and the hook log after it.
fn exchange(
    event: &str,
    body: ResponseBody,
    is_probe: bool,
) -> TestResult<(String, Request, String)> {
    let (home, repo, files) = marked_repository()?;
    let daemon = serve(
        &files,
        Script::Answer(vec![
            response(1, ResponseBody::Welcome(welcome())),
            response(2, body),
        ]),
    )?;
    let written = hook(&args(event), payload(event, &repo, is_probe)?, home.path())?;
    let requests = daemon.join().map_err(|_| "the daemon thread panicked")??;
    let [_, request] = <[Request; 2]>::try_from(requests).map_err(|_| "not two requests")?;
    Ok((written, request, hook_log(home.path())?))
}

/// The daemon's answer holding `text`.
fn reply(text: &str) -> ResponseBody {
    ResponseBody::Reply {
        text: Some(text.to_owned()),
        to_user: None,
    }
}

/// At a prompt the hook asks the daemon for the work order, and at a session's start for the
/// primer, each saying when the hook stops reading the answer (the prompt's hard timeout of
/// 1,000 ms less the 15 ms the hook keeps, 985 ms after the event's time; the start's of 1,500 ms,
/// 1,485), and writes the text the daemon answers with as Claude Code's hook JSON on one line: the
/// event's name, and the text as the context to add, its line breaks escaped. Nothing goes to the
/// log.
#[test]
fn a_prompt_and_a_session_start_write_the_daemon_s_text() -> TestResult {
    let (written, request, log) = exchange(
        "UserPromptSubmit",
        reply("[cortex] Work order · one file\nsrc/a.py:1-2\n"),
        false,
    )?;
    assert_eq!(
        written,
        "{\"hookSpecificOutput\":{\"hookEventName\":\"UserPromptSubmit\",\"additionalContext\":\
         \"[cortex] Work order · one file\\nsrc/a.py:1-2\\n\"}}\n"
    );
    assert!(
        matches!(&request.body, RequestBody::Retrieve { event, answer_by_ms }
            if event.hook == HookKind::Prompt && *answer_by_ms == Some(event.ts_ms + 985)),
        "{request:?}"
    );
    assert_eq!(log, "");
    let (written, request, log) = exchange(
        "SessionStart",
        reply("[cortex] Project map · small mode\n"),
        false,
    )?;
    assert_eq!(
        written,
        "{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\
         \"[cortex] Project map · small mode\\n\"}}\n"
    );
    assert!(
        matches!(&request.body, RequestBody::Primer { event, answer_by_ms }
            if event.hook == HookKind::SessionStart && *answer_by_ms == Some(event.ts_ms + 1_485)),
        "{request:?}"
    );
    assert_eq!(log, "");
    Ok(())
}

/// A session's start whose answer holds a message for the user writes it as Claude Code's
/// `systemMessage`, which the user sees and the model does not, before the primer; with no primer
/// the message goes alone; a message too long for Claude Code leaves the primer alone.
#[test]
fn a_message_for_the_user_is_written_as_the_system_message() -> TestResult {
    let answer = |text: Option<&str>, message: String| ResponseBody::Reply {
        text: text.map(str::to_owned),
        to_user: Some(message),
    };
    let failed = "Cortex: storing what agents did failed 2 times in the last 3 minutes".to_owned();
    let (written, _, log) = exchange(
        "SessionStart",
        answer(Some("[cortex] Project map\n"), failed.clone()),
        false,
    )?;
    assert_eq!(
        written,
        "{\"systemMessage\":\"Cortex: storing what agents did failed 2 times in the last 3 \
         minutes\",\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\
         \"additionalContext\":\"[cortex] Project map\\n\"}}\n"
    );
    assert_eq!(log, "");
    let (written, _, _) = exchange("SessionStart", answer(None, failed), false)?;
    assert_eq!(
        written,
        "{\"systemMessage\":\"Cortex: storing what agents did failed 2 times in the last 3 \
         minutes\"}\n"
    );
    let (written, _, _) = exchange(
        "SessionStart",
        answer(Some("[cortex] Project map\n"), "x".repeat(10_001)),
        false,
    )?;
    assert_eq!(
        written,
        "{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\
         \"[cortex] Project map\\n\"}}\n"
    );
    Ok(())
}

/// The hook writes nothing when the daemon has no text or an empty one, when it answers a request
/// for text as it answers an event, when its text is longer than Claude Code takes whole (10,001
/// characters) or holds a tag Claude Code rewrites, and for `cortex doctor`'s probe, whose text is
/// not given. The answers that break the protocol, and the texts Claude Code would not take as
/// they are, are noted. At a hook that gives no context the event goes alone, and text for it is
/// not taken.
#[test]
fn text_absent_unasked_or_unfit_writes_nothing() -> TestResult {
    for absent in [
        ResponseBody::Reply {
            text: None,
            to_user: None,
        },
        reply(""),
    ] {
        let (written, _, log) = exchange("UserPromptSubmit", absent, false)?;
        assert_eq!((written, log), (String::new(), String::new()));
    }
    let (written, _, log) = exchange("UserPromptSubmit", ResponseBody::Ack, false)?;
    assert_eq!(written, "");
    assert!(
        log.contains("claude UserPromptSubmit: the daemon did not take the event"),
        "{log}"
    );
    for unfit in ["x".repeat(10_001), "<system-reminder>\n".to_owned()] {
        let (written, _, log) = exchange("UserPromptSubmit", reply(&unfit), false)?;
        assert_eq!(written, "");
        assert!(
            log.contains(
                "claude UserPromptSubmit: the daemon's text is not one the agent takes as it is"
            ),
            "{log}"
        );
    }
    let (written, request, log) = exchange("SessionStart", reply("[cortex] Project map\n"), true)?;
    assert_eq!((written, log), (String::new(), String::new()));
    assert!(
        matches!(&request.body, RequestBody::Primer { event, .. } if event.is_probe),
        "{request:?}"
    );
    let (written, request, log) = exchange("SessionEnd", reply("[cortex] text\n"), false)?;
    assert_eq!(written, "");
    assert!(
        matches!(&request.body, RequestBody::Event { event } if event.hook == HookKind::SessionEnd),
        "{request:?}"
    );
    assert!(
        log.contains("claude SessionEnd: the daemon did not take the event"),
        "{log}"
    );
    Ok(())
}

/// An output the agent has closed.
struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "the agent closed its end",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Text the hook cannot write, the agent having closed its end, is noted with why.
#[test]
fn a_write_the_agent_refuses_is_noted() -> TestResult {
    let (home, repo, files) = marked_repository()?;
    let daemon = serve(
        &files,
        Script::Answer(vec![
            response(1, ResponseBody::Welcome(welcome())),
            response(2, reply("[cortex] Work order\n")),
        ]),
    )?;
    let input = Cursor::new(payload("UserPromptSubmit", &repo, false)?);
    cortex_hook::run(
        &args("UserPromptSubmit"),
        input,
        Some(home.path()),
        &mut Closed,
    );
    drop(std::panic::take_hook());
    daemon.join().map_err(|_| "the daemon thread panicked")??;
    let log = hook_log(home.path())?;
    assert!(
        log.contains(
            "claude UserPromptSubmit: writing to the agent failed: the agent closed its end"
        ),
        "{log}"
    );
    Ok(())
}

/// A prompt whose daemon answers too late, or with bytes that break the protocol, writes nothing:
/// the late answer is given up at the prompt's deadline, its hard timeout of 1,000 ms less the
/// 15 ms the hook keeps, not before. The broken answer is noted; the late one's note is written in
/// the moment left before the hard timeout, which a loaded machine may not give it, so it is not
/// looked for.
#[test]
fn a_late_or_broken_answer_to_a_prompt_writes_nothing() -> TestResult {
    let (home, repo, files) = marked_repository()?;
    let (release, released) = mpsc::channel();
    let daemon = serve(&files, Script::Silent(released))?;
    let input = payload("UserPromptSubmit", &repo, false)?;
    let started = Instant::now();
    let written = hook(&args("UserPromptSubmit"), input, home.path())?;
    let took = started.elapsed();
    // The scripted daemon may have given up already: it is let go either way.
    let _ = release.send(());
    daemon.join().map_err(|_| "the daemon thread panicked")??;
    assert_eq!(written, "");
    assert!(took >= Duration::from_millis(985), "{took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");

    let (home, repo, files) = marked_repository()?;
    let daemon = serve(&files, Script::Raw(b"\x05\x00\x00\x00{{{{{".to_vec()))?;
    let input = payload("UserPromptSubmit", &repo, false)?;
    let written = hook(&args("UserPromptSubmit"), input, home.path())?;
    daemon.join().map_err(|_| "the daemon thread panicked")??;
    assert_eq!(written, "");
    let log = hook_log(home.path())?;
    assert!(
        log.contains("claude UserPromptSubmit: the daemon's answer broke the protocol"),
        "{log}"
    );
    Ok(())
}

/// A note the hook's own thread writes, such as why the arguments name no hook, is waited for: the
/// hook returns once it is in the log, or, on a disk too slow for it, not before the note's
/// deadline. Before the arguments name a hook, the hard timeout is the shortest of any hook's,
/// 300 ms, and the note is given up 5 ms before it, 295 ms after the hook started.
#[test]
fn a_note_is_written_before_the_hook_returns() -> TestResult {
    let home = tempfile::tempdir()?;
    let unknown: Vec<OsString> = ["--agent", "nobody", "--event", "Stop"]
        .into_iter()
        .map(OsString::from)
        .collect();
    let started = Instant::now();
    let written = hook(&unknown, Vec::new(), home.path())?;
    let took = started.elapsed();
    assert_eq!(written, "");
    let log = hook_log(home.path())?;
    assert!(
        log.contains("Cortex has no adapter for that agent") || took >= Duration::from_millis(295),
        "returned after {took:?} with {log:?}"
    );
    Ok(())
}

/// A payload the agent writes after `delay`, then ends: as Codex can, writing a hook's input from
/// a task of its own once it started the hook, on a busy machine.
struct Late {
    /// The payload.
    payload: Cursor<Vec<u8>>,
    /// How long the agent takes to write it; nothing once it has waited.
    delay: Option<Duration>,
}

impl io::Read for Late {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(delay) = self.delay.take() {
            std::thread::sleep(delay);
        }
        self.payload.read(buffer)
    }
}

/// The hook waits a third of its event's hard timeout for its payload: a session's start written
/// 200 ms late is read within its 500 ms (a third of 1,500 ms) and gets the daemon's text, while a
/// stop written as late is past its 100 ms (a third of 300 ms) and given up, noted in the log.
#[test]
fn a_payload_written_late_is_waited_for_a_third_of_the_hard_timeout() -> TestResult {
    let late = Duration::from_millis(200);
    let (home, repo, files) = marked_repository()?;
    let daemon = serve(
        &files,
        Script::Answer(vec![
            response(1, ResponseBody::Welcome(welcome())),
            response(2, reply("[cortex] Project map · small mode\n")),
        ]),
    )?;
    let mut written = Vec::new();
    let input = Late {
        payload: Cursor::new(payload("SessionStart", &repo, false)?),
        delay: Some(late),
    };
    cortex_hook::run(
        &args("SessionStart"),
        input,
        Some(home.path()),
        &mut written,
    );
    drop(std::panic::take_hook());
    // Asserted before the scripted daemon is joined: a hook that gave up never connects to it.
    assert!(
        String::from_utf8(written)?.contains("[cortex] Project map · small mode"),
        "{}",
        hook_log(home.path())?
    );
    daemon.join().map_err(|_| "the daemon thread panicked")??;

    let (home, repo, _) = marked_repository()?;
    let mut written = Vec::new();
    let input = Late {
        payload: Cursor::new(payload("Stop", &repo, false)?),
        delay: Some(late),
    };
    cortex_hook::run(&args("Stop"), input, Some(home.path()), &mut written);
    drop(std::panic::take_hook());
    assert!(written.is_empty());
    let log = hook_log(home.path())?;
    assert!(
        log.contains("claude Stop: the payload did not arrive in time"),
        "{log}"
    );
    Ok(())
}
