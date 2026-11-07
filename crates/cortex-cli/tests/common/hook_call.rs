//! Running `cortex hook` as an agent does: a payload on its standard input, its output and how
//! long it took kept for the test to judge, and the context it gave Claude Code read back.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{TestResult, isolate};

/// How a hook call went: its output and how long it took.
pub struct Call {
    /// What the process left behind.
    pub output: Output,
    /// From its start to its exit.
    pub took: Duration,
    /// Whether the whole input could be written: not when the hook had stopped reading and closed
    /// its end first.
    pub is_input_taken: bool,
}

impl Call {
    /// Whether the call ended as an agent requires: exit code 0, nothing printed.
    pub fn is_silent_success(&self) -> bool {
        self.output.status.code() == Some(0)
            && self.output.stdout.is_empty()
            && self.output.stderr.is_empty()
    }

    /// What the call gave Claude Code to add to its context at `event`: none when it printed
    /// nothing. The call must have ended as an agent requires: exit code 0, nothing on stderr, and
    /// on stdout nothing, or one line holding Claude Code's hook JSON for `event` and nothing else.
    pub fn context(&self, event: &str) -> TestResult<Option<String>> {
        let output = &self.output;
        if output.status.code() != Some(0) || !output.stderr.is_empty() {
            return Err(format!("{event}: {output:?}").into());
        }
        if output.stdout.is_empty() {
            return Ok(None);
        }
        let printed = std::str::from_utf8(&output.stdout)?;
        let line = printed
            .strip_suffix('\n')
            .filter(|line| !line.contains('\n'))
            .ok_or_else(|| format!("{event}: not one line: {printed:?}"))?;
        let value: Value = serde_json::from_str(line)?;
        let context = value
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{event}: no context: {line}"))?;
        let expected = json!({
            "hookSpecificOutput": {"hookEventName": event, "additionalContext": context}
        });
        if value != expected {
            return Err(format!("{event}: not Claude Code's hook JSON for it: {line}").into());
        }
        Ok(Some(context.to_owned()))
    }
}

/// Runs `cortex hook` with `args`, writing `input` to its standard input after `delay` from a
/// thread of its own (a hook that stops reading may close the pipe first), with `home` as the
/// user's home folder.
pub fn hook(args: &[&str], input: Vec<u8>, delay: Duration, home: &Path) -> TestResult<Call> {
    hook_with(args, input, (delay, home), &[])
}

/// [`hook`], with each of `variables` set in the hook's environment, as an agent sets some for
/// its hooks.
pub fn hook_with(
    args: &[&str],
    input: Vec<u8>,
    (delay, home): (Duration, &Path),
    variables: &[(&str, &Path)],
) -> TestResult<Call> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .arg("hook")
        .args(args)
        .envs(variables.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut command, home);
    let started = Instant::now();
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin is not piped")?;
    let writer = thread::spawn(move || {
        thread::sleep(delay);
        // The hook may have stopped reading and closed its end: that is its right.
        stdin.write_all(&input).is_ok()
    });
    let output = child.wait_with_output()?;
    let took = started.elapsed();
    let is_input_taken = writer.join().map_err(|_| "the writer panicked")?;
    Ok(Call {
        output,
        took,
        is_input_taken,
    })
}

/// Runs `cortex hook` for the Claude Code `payload` as Claude Code would, with `home` as the
/// user's home folder. The call must end as an agent requires of a hook: silently, or, at a
/// session's start and at a prompt, with Claude Code's hook JSON for its event.
pub fn send(payload: &serde_json::Value, home: &Path) -> TestResult {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let call = hook(
        &["--agent", "claude", "--event", &event],
        serde_json::to_vec(payload)?,
        Duration::ZERO,
        home,
    )?;
    let context = call.context(&event)?;
    let may_give = matches!(event.as_str(), "SessionStart" | "UserPromptSubmit");
    assert!(context.is_none() || may_give, "{event}: {context:?}");
    Ok(())
}
