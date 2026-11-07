//! A shell command's exit code, read from the end of Codex's rollout file: Codex tells its hooks
//! no exit code, but by the time it runs a command's `PostToolUse` hook it has written the
//! command's `item_completed` line, whose item (`CommandExecution`) has the hook's `tool_use_id`
//! as its `id` and the command's `exit_code`. Only the file's last mebibyte is read, from a
//! regular file named `rollout-*.jsonl` under a Codex `sessions` folder; the line is searched for
//! by the id's bytes and only it is parsed, for its exit code alone. When the line is not there
//! (an older Codex, a line not yet written, a command whose output pushed it further back) the
//! exit code is unknown.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How much of the rollout's end is read: the command's line comes last or nearly so, and Codex
/// cuts a command's output in the rollout to tens of kilobytes.
const TAIL_BYTES: u64 = 1024 * 1024;

/// The longest command id looked for; Codex's are `exec-` and a UUID.
const MAX_CALL_BYTES: usize = 128;

/// The variable that moves Codex's home folder, `~/.codex`, and its rollout files with it.
const HOME_VARIABLE: &str = "CODEX_HOME";

/// The exit code of the command `call` from the rollout file at `transcript`, as Codex wrote it.
pub(super) fn exit_code(transcript: &str, call: &str) -> Option<i32> {
    if call.is_empty()
        || call.len() > MAX_CALL_BYTES
        || !call
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    let path = rollout_path(transcript)?;
    let tail = read_tail(&path)?;
    tail.split(|byte| *byte == b'\n')
        .rev()
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter(|line| line.contains(call) && line.contains("\"CommandExecution\""))
        .find_map(|line| {
            let line: Line = serde_json::from_str(line).ok()?;
            let item = line.payload?.item?;
            (item.kind.as_deref() == Some("CommandExecution") && item.id.as_deref() == Some(call))
                .then_some(item.exit_code)
                .flatten()
        })
}

/// The rollout file `transcript` names, when it is one: an absolute path to a file named
/// `rollout-*.jsonl` inside a Codex `sessions` folder (`~/.codex/sessions`, or `sessions` under
/// `CODEX_HOME`), the Windows verbatim prefix taken off.
fn rollout_path(transcript: &str) -> Option<PathBuf> {
    let path = PathBuf::from(transcript.strip_prefix(r"\\?\").unwrap_or(transcript));
    let name = path.file_name()?.to_str()?;
    let is_jsonl = path
        .extension()
        .is_some_and(|extension| extension == "jsonl");
    if !path.is_absolute() || !name.starts_with("rollout-") || !is_jsonl {
        return None;
    }
    let moved = std::env::var_os(HOME_VARIABLE)
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join("sessions"));
    let is_in_sessions = path.ancestors().any(|folder| {
        folder.ends_with(Path::new(".codex").join("sessions"))
            || moved.as_deref().is_some_and(|moved| folder == moved)
    });
    is_in_sessions.then_some(path)
}

/// The last [`TAIL_BYTES`] of the regular file at `path`, which is not followed when it is a
/// link.
fn read_tail(path: &Path) -> Option<Vec<u8>> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut tail).ok()?;
    Some(tail)
}

/// A rollout line, as far as a command's result goes.
#[derive(Deserialize)]
struct Line {
    /// What the line holds.
    payload: Option<Payload>,
}

/// What an `item_completed` line holds.
#[derive(Deserialize)]
struct Payload {
    /// The item that completed.
    item: Option<Item>,
}

/// A completed item, as far as a command's result goes.
#[derive(Deserialize)]
struct Item {
    /// Its type: `CommandExecution` for a shell command.
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Its id, the hook's `tool_use_id`.
    id: Option<String>,
    /// The command's exit code.
    exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rollout's `item_completed` line for the command `id` that exited with `code`.
    fn finished(id: &str, code: i32) -> String {
        format!(
            r#"{{"timestamp":"2026-11-06T19:50:15.000Z","type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"CommandExecution","id":"{id}","status":"failed","aggregated_output":"exec-0 CommandExecution","exit_code":{code}}}}}}}"#
        )
    }

    /// Writes `lines` as the rollout `name` under `folder`'s `.codex/sessions/2026/11/06`, and
    /// returns its path.
    fn rollout(folder: &Path, name: &str, lines: &[String]) -> std::io::Result<PathBuf> {
        let day = folder
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("11")
            .join("06");
        std::fs::create_dir_all(&day)?;
        let path = day.join(name);
        std::fs::write(&path, lines.join("\n") + "\n")?;
        Ok(path)
    }

    /// The command's own line gives its exit code, wherever it stands among the last lines;
    /// another command's line, a line that only names the id in its text, and an item of another
    /// type give none.
    #[test]
    fn a_command_s_exit_code_is_read_from_its_line() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let other = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","id":"exec-2","exit_code":5}}}"#;
        let path = rollout(
            folder.path(),
            "rollout-2026-11-06T19-50-00-x.jsonl",
            &[
                finished("exec-1", 3),
                finished("exec-0", 0),
                other.to_owned(),
            ],
        )?;
        let text = path.to_string_lossy();
        assert_eq!(exit_code(&text, "exec-1"), Some(3));
        assert_eq!(exit_code(&text, "exec-0"), Some(0));
        assert_eq!(exit_code(&text, "exec-2"), None);
        assert_eq!(exit_code(&text, "exec-9"), None);
        // Every line here names `exec-0` in its output, none is its own.
        let named = rollout(
            folder.path(),
            "rollout-2026-11-06T19-51-00-y.jsonl",
            &[finished("exec-1", 3)],
        )?;
        assert_eq!(exit_code(&named.to_string_lossy(), "exec-0"), None);
        Ok(())
    }

    /// Only a rollout file in a Codex sessions folder is read: not one named otherwise, not one
    /// elsewhere, not a relative path, not a folder; and an id that is no id is not looked for.
    #[test]
    fn only_a_rollout_in_a_sessions_folder_is_read() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let lines = [finished("exec-1", 3)];
        let named = rollout(folder.path(), "notes.jsonl", &lines)?;
        assert_eq!(exit_code(&named.to_string_lossy(), "exec-1"), None);
        let elsewhere = folder.path().join("rollout-a.jsonl");
        std::fs::write(&elsewhere, finished("exec-1", 3))?;
        assert_eq!(exit_code(&elsewhere.to_string_lossy(), "exec-1"), None);
        assert_eq!(exit_code("rollout-a.jsonl", "exec-1"), None);
        let path = rollout(folder.path(), "rollout-b.jsonl", &lines)?;
        let text = path.to_string_lossy();
        assert_eq!(exit_code(&text, "exec-1"), Some(3));
        for odd in ["", "exec 1", "exec-1\"", &"x".repeat(129)] {
            assert_eq!(exit_code(&text, odd), None, "{odd}");
        }
        let day = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
        std::fs::create_dir(day.join("rollout-c.jsonl"))?;
        assert_eq!(
            exit_code(&day.join("rollout-c.jsonl").to_string_lossy(), "exec-1"),
            None
        );
        Ok(())
    }

    /// The command's line is found in the rollout's last mebibyte: after two mebibytes of other
    /// lines it is read; followed by more than a mebibyte of them it is not.
    #[test]
    fn only_the_rollout_s_last_mebibyte_is_read() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let filler: Vec<String> = (0..2_048)
            .map(|line| {
                format!(
                    r#"{{"type":"response_item","payload":{{"n":{line},"pad":"{}"}}}}"#,
                    "x".repeat(1_000)
                )
            })
            .collect();
        let mut late = filler.clone();
        late.push(finished("exec-1", 3));
        let path = rollout(folder.path(), "rollout-late.jsonl", &late)?;
        assert_eq!(exit_code(&path.to_string_lossy(), "exec-1"), Some(3));
        let mut early = vec![finished("exec-1", 3)];
        early.extend(filler);
        let path = rollout(folder.path(), "rollout-early.jsonl", &early)?;
        assert_eq!(exit_code(&path.to_string_lossy(), "exec-1"), None);
        Ok(())
    }
}
