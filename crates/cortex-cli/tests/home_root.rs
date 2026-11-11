//! Cortex never serves the home folder as a repository, through the real binary with a home
//! folder of the test's own: `cortex mcp` started there, as an editor starts it from no repository
//! in particular, answers each tool call with a tool error the agent passes on, and starts no
//! daemon; `cortex where` run there refuses at once instead of waiting for a daemon; and the
//! daemon itself refuses a root that is the home folder. Each command is bounded, so that one that
//! went on to serve the home folder fails the test rather than hanging it.

mod common;

use std::io::Write as _;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::{TestResult, daemon_command, isolate};
use cortex_core::canonical_root;

/// How long a command here may run: each ends at once when it works.
const ENDS_WITHIN: Duration = Duration::from_secs(60);

/// Runs `command`, writes `lines` to its input and closes it, and gives what it printed, within
/// [`ENDS_WITHIN`].
fn talk(mut command: Command, lines: &[&str]) -> TestResult<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("no stdin")?;
    for line in lines {
        writeln!(input, "{line}")?;
    }
    drop(input);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(child.wait_with_output()));
    Ok(receiver.recv_timeout(ENDS_WITHIN)??)
}

/// What `cortex mcp` started in `home` answered to one `cortex_where` call, from a client that
/// names no roots: whether it is an error, and its text.
fn answered_in_home(home: &std::path::Path) -> TestResult<(bool, String)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.arg("mcp").current_dir(home);
    isolate(&mut command, home);
    let output = talk(
        command,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"cortex_where","arguments":{"query":"where are orders summed"}}}"#,
        ],
    )?;
    if !output.status.success() {
        return Err(format!("{output:?}").into());
    }
    let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let answer = replies
        .iter()
        .find(|reply| reply.get("id") == Some(&serde_json::Value::from(2)))
        .ok_or("no answer")?;
    let is_error = answer.pointer("/result/isError") == Some(&serde_json::Value::Bool(true));
    let text = answer
        .pointer("/result/content/0/text")
        .and_then(serde_json::Value::as_str)
        .ok_or("no text")?;
    Ok((is_error, text.to_owned()))
}

/// `cortex mcp` started in the home folder answers `cortex_where` with a tool error, word for word,
/// saying the home folder is never served and what the user is to do; it records no repository
/// as set up and starts no daemon.
#[test]
fn mcp_in_the_home_folder_says_it_is_never_served() -> TestResult {
    let home = tempfile::tempdir()?;
    let (is_error, text) = answered_in_home(home.path())?;
    assert!(is_error, "{text}");
    assert_eq!(
        text,
        format!(
            "Cortex: {} is the home folder, a folder holding it, or a file system's root, which \
             Cortex never serves; tell the user to start the agent in a repository set up with \
             `cortex init`",
            canonical_root(home.path())?.display()
        )
    );
    assert!(!home.path().join(".cortex").join("repositories").exists());
    assert!(!home.path().join(".cortex").join("run").exists());
    Ok(())
}

/// `cortex where` run in the home folder refuses at once, saying why, and starts no daemon.
#[test]
fn where_in_the_home_folder_refuses() -> TestResult {
    let home = tempfile::tempdir()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["where", "where are orders summed"])
        .current_dir(home.path());
    isolate(&mut command, home.path());
    let output = talk(command, &[])?;
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "cortex: error: {} is the home folder, a folder holding it, or a file system's root, \
             which Cortex never serves as a repository: run the command in a repository, or name \
             one with --root\n",
            canonical_root(home.path())?.display()
        )
    );
    assert!(!home.path().join(".cortex").join("repositories").exists());
    Ok(())
}

/// The daemon asked to serve the home folder refuses before it records or holds anything, saying
/// why.
#[test]
fn the_daemon_refuses_the_home_folder() -> TestResult {
    let home = tempfile::tempdir()?;
    let output = talk(daemon_command(home.path(), home.path()), &[])?;
    assert!(!output.status.success(), "{output:?}");
    let said = String::from_utf8(output.stderr)?;
    assert!(
        said.contains(&format!(
            "{} is the home folder, a folder holding it, or a file system's root, which Cortex \
             never serves as a repository",
            canonical_root(home.path())?.display()
        )),
        "{said}"
    );
    assert!(!home.path().join(".cortex").join("repositories").exists());
    Ok(())
}
