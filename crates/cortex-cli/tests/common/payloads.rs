//! Hook payloads captured from the agents, as the tests send them: read from the fixtures of one
//! agent's version, every path the capture names under its repository (`C:\repo`) moved into the
//! test's own.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::TestResult;

/// The repository root the captured payloads name.
const CAPTURED_ROOT: &str = r"C:\repo";

/// The folder of the payloads captured from `agent` at `version`.
pub fn fixtures_of(agent: &str, version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(agent)
        .join(version)
}

/// The folder of the payloads captured from Claude Code `version`.
pub fn fixtures(version: &str) -> PathBuf {
    fixtures_of("claude", version)
}

/// The payload `name` captured from Claude Code `version`, with every path under the captured
/// repository moved under `root`, and the files a search found in this platform's form.
pub fn payload(version: &str, name: &str, root: &Path) -> TestResult<Vec<u8>> {
    payload_of("claude", version, name, root)
}

/// The payload `name` captured from `agent` at `version`, with every path under the captured
/// repository moved under `root`, and the files a search found in this platform's form.
pub fn payload_of(agent: &str, version: &str, name: &str, root: &Path) -> TestResult<Vec<u8>> {
    let path = fixtures_of(agent, version).join(name);
    let mut value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    move_paths(&mut value, root);
    native_search_paths(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

/// Writes the files a search found in this platform's form. The agents name them relative to the
/// working folder, with backslashes on Windows, where the payloads were captured; elsewhere they
/// take `/`. The captured matches' text holds no backslash, so Grep's output, and a response
/// given as one text (Codex's), are rewritten whole.
fn native_search_paths(value: &mut Value) {
    if cfg!(windows) {
        return;
    }
    let Some(response) = value.get_mut("tool_response") else {
        return;
    };
    if let Value::String(text) = response {
        *text = text.replace('\\', "/");
        return;
    }
    if let Some(Value::Array(files)) = response.get_mut("filenames") {
        for file in files {
            if let Value::String(path) = file {
                *path = path.replace('\\', "/");
            }
        }
    }
    if let Some(Value::String(content)) = response.get_mut("content") {
        *content = content.replace('\\', "/");
    }
}

/// Moves every string in `value` that is a path under the captured repository to the same place
/// under `root`.
pub fn move_paths(value: &mut Value, root: &Path) {
    match value {
        Value::String(text) => {
            let rest = text.strip_prefix(CAPTURED_ROOT);
            // The root itself, or a path under it: not a folder whose name only begins the same.
            if let Some(rest) = rest.filter(|rest| rest.is_empty() || rest.starts_with('\\')) {
                let moved = rest
                    .split('\\')
                    .filter(|name| !name.is_empty())
                    .fold(root.to_path_buf(), |path, name| path.join(name));
                *text = moved.to_string_lossy().into_owned();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| move_paths(item, root)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| move_paths(field, root)),
        _ => {}
    }
}

/// The event name `payload` carries.
pub fn event_name(payload: &[u8]) -> TestResult<String> {
    let value: Value = serde_json::from_slice(payload)?;
    Ok(value
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or("no event name")?
        .to_owned())
}

/// The payload `name` captured from Claude Code `version`, its paths moved under `root`, sent as
/// session `session`'s, with `transcript` as the transcript it names: its session's, or for a
/// subagent's stop, the subagent's.
pub fn as_session(
    version: &str,
    name: &str,
    root: &Path,
    (session, transcript): (&str, &Path),
) -> TestResult<Value> {
    let mut payload: Value = serde_json::from_slice(&payload(version, name, root)?)?;
    let Value::Object(fields) = &mut payload else {
        return Err(format!("the payload {name} is not an object").into());
    };
    fields.insert("session_id".to_owned(), Value::from(session));
    let field = if fields.contains_key("agent_transcript_path") {
        "agent_transcript_path"
    } else {
        "transcript_path"
    };
    let transcript = Value::from(transcript.to_string_lossy().into_owned());
    fields.insert(field.to_owned(), transcript);
    Ok(payload)
}

/// Copies the transcript `name` captured from Claude Code `version` to `to`, making the folders
/// it needs.
pub fn copy_transcript(version: &str, name: &str, to: &Path) -> TestResult {
    let from = fixtures(version).join("transcripts").join(name);
    std::fs::create_dir_all(to.parent().ok_or("a transcript has a folder")?)?;
    std::fs::copy(from, to)?;
    Ok(())
}

/// The folder Claude Code keeps the transcripts of the test's repository in, under `home`.
pub fn transcripts_folder(home: &Path) -> PathBuf {
    home.join(".claude").join("projects").join("C--repo")
}
