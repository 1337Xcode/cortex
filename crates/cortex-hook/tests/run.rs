//! The hook as an agent runs it, against a scripted daemon: what it makes of each answer, seen in
//! the hook log, the one place a hook says anything.

mod common;

use std::ffi::OsString;
use std::io::Cursor;
use std::path::Path;

use common::{Script, TestResult, response, run_files_of, serve, welcome};
use cortex_core::ipc::ResponseBody;
use cortex_core::{UserFolder, canonical_root};

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

/// A daemon that answers an event with an error, rather than taking it, has not taken it: the hook
/// notes that in its log. So it does for `cortex doctor`'s probe, which is answered the same way.
#[test]
fn an_answer_that_takes_nothing_is_noted() -> TestResult {
    for is_probe in [false, true] {
        let home = tempfile::tempdir()?;
        let repo = home.path().join("repo");
        std::fs::create_dir_all(repo.join(".cortex"))?;
        let files = run_files_of(home.path(), &canonical_root(&repo)?)?;
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
        let payload = serde_json::to_vec(&serde_json::json!({
            "session_id": "s1",
            "hook_event_name": "Stop",
            "cwd": repo,
            "cortex_probe": is_probe,
        }))?;
        cortex_hook::run(&args("Stop"), Cursor::new(payload), Some(home.path()));
        daemon.join().map_err(|_| "the daemon thread panicked")??;
        let log = hook_log(home.path())?;
        assert!(
            log.contains("claude Stop: the daemon did not take the event"),
            "probe {is_probe}: {log:?}"
        );
    }
    Ok(())
}

/// A note the hook's own thread writes, such as why the arguments name no hook, is in the log when
/// the hook returns: the hook waits for it, up to just before its hard timeout.
#[test]
fn a_note_is_written_before_the_hook_returns() -> TestResult {
    let home = tempfile::tempdir()?;
    let unknown: Vec<OsString> = ["--agent", "nobody", "--event", "Stop"]
        .into_iter()
        .map(OsString::from)
        .collect();
    cortex_hook::run(&unknown, Cursor::new(Vec::new()), Some(home.path()));
    let log = hook_log(home.path())?;
    assert!(
        log.contains("Cortex has no adapter for that agent"),
        "{log:?}"
    );
    Ok(())
}
