//! Hook payloads captured from Claude Code, as the tests send them: read from the fixtures of one
//! Claude Code version, every path the capture names under its repository (`C:\repo`) moved into
//! the test's own.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::TestResult;

/// The repository root the captured payloads name.
const CAPTURED_ROOT: &str = r"C:\repo";

/// The folder of the payloads captured from Claude Code `version`.
pub fn fixtures(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/claude")
        .join(version)
}

/// The payload `name` captured from Claude Code `version`, with every path under the captured
/// repository moved under `root`.
pub fn payload(version: &str, name: &str, root: &Path) -> TestResult<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(&std::fs::read(fixtures(version).join(name))?)?;
    move_paths(&mut value, root);
    Ok(serde_json::to_vec(&value)?)
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
