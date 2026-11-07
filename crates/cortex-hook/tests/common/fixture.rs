//! The captured payloads every adapter's tests read, in this platform's form, and what those
//! tests share: the repository every fixture runs in, the stamp every event gets, the rules every
//! event keeps to, and payloads of any shape for the proptests.

use std::path::{Path, PathBuf};

use cortex_core::{Event, HookKind, Identifier, RepoPath, Ulid};
use cortex_hook::{Located, Stamp};
use proptest::prelude::*;
use serde_json::Value;

use super::TestResult;

/// The folder of the payloads captured from `agent` at `version`.
pub fn fixtures_of(agent: &str, version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(agent)
        .join(version)
}

/// A path the fixtures write in Windows form, in this platform's form: as captured on Windows;
/// elsewhere the home folder `C:\Users\dev` is `/home/dev`, and any other `C:\x` is `/x`.
pub fn native(windows: &str) -> String {
    if cfg!(windows) {
        return windows.to_owned();
    }
    let unix = windows
        .replace(r"C:\Users\dev", "/home/dev")
        .replace('\\', "/");
    unix.strip_prefix("C:")
        .map_or_else(|| unix.clone(), str::to_owned)
}

/// The payload `name` captured from `agent` at `version`, its paths in this platform's form: the
/// working folder, the transcripts, the paths a tool was given, and the files a search found,
/// which the response names relative to the working folder, with backslashes on Windows. The
/// captured responses' text holds no other backslash, so a response's text is rewritten whole.
/// Any other string that is an absolute path on `C:`, wherever an agent's own payload holds it,
/// is rewritten too.
pub fn read_fixture(agent: &str, version: &str, name: &str) -> TestResult<Vec<u8>> {
    let text = std::fs::read(fixtures_of(agent, version).join(name))?;
    if cfg!(windows) {
        return Ok(text);
    }
    let mut value: Value = serde_json::from_slice(&text)?;
    let rewrite = |field: Option<&mut Value>| {
        if let Some(Value::String(path)) = field {
            *path = native(path);
        }
    };
    for key in [
        "cwd",
        "working_dir",
        "transcript_path",
        "agent_transcript_path",
    ] {
        rewrite(value.get_mut(key));
    }
    if let Some(input) = value.get_mut("tool_input") {
        rewrite(input.get_mut("file_path"));
        rewrite(input.get_mut("path"));
    }
    match value.get_mut("tool_response") {
        Some(response @ Value::String(_)) => rewrite(Some(response)),
        Some(response) => {
            if let Some(Value::Array(files)) = response.get_mut("filenames") {
                for file in files {
                    rewrite(Some(file));
                }
            }
            rewrite(response.get_mut("content"));
        }
        None => {}
    }
    rewrite_absolute(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

/// Rewrites every string in `value` that is an absolute path on `C:` to this platform's form.
fn rewrite_absolute(value: &mut Value) {
    match value {
        Value::String(text) if text.starts_with(r"C:\") => *text = native(text),
        Value::Array(elements) => elements.iter_mut().for_each(rewrite_absolute),
        Value::Object(fields) => fields.values_mut().for_each(rewrite_absolute),
        _ => {}
    }
}

/// The fixture's event name, as the installer passes it to `cortex hook --event`: its
/// `hook_event_name`, or OpenHands' `event_type`, or Cline's `hookName`.
pub fn event_name(payload: &[u8]) -> TestResult<String> {
    let value: Value = serde_json::from_slice(payload)?;
    Ok(value
        .get("hook_event_name")
        .or_else(|| value.get("event_type"))
        .or_else(|| value.get("hookName"))
        .and_then(Value::as_str)
        .ok_or("no event name")?
        .to_owned())
}

/// The names of the captured payloads in `folder`, sorted.
pub fn fixture_names(folder: &Path) -> TestResult<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(folder)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.retain(|name| name.ends_with(".json"));
    names.sort_unstable();
    Ok(names)
}

/// The repository folder, already canonical, which every fixture runs in.
pub fn root() -> PathBuf {
    PathBuf::from(native(r"C:\repo"))
}

/// Finds every fixture's working folder in the repository.
pub fn locate(_: &Path) -> Option<Located> {
    Some(Located {
        cwd: root(),
        root: root(),
        agent_root: root(),
    })
}

/// The id and time every event in these tests gets.
pub fn stamp() -> Stamp {
    Stamp {
        event_id: Ulid::from_seed(1_700_000_000_000, b"fixture"),
        ts_ms: 1_700_000_000_000,
    }
}

/// A name.
pub fn id(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// Asserts that `event`, made at `hook`, keeps to that hook's fields and to every cap: a tool only
/// at a tool's two moments, a result only after it, a prompt only at a prompt and within 128 KiB,
/// a start's source only at a start, a pattern within 1,000 characters, ranges that are ranges,
/// and a search's hits only after it, at most 50 and 4,096 bytes of them, each once, never the
/// repository's top folder.
pub fn assert_shape(hook: HookKind, event: &Event) {
    let is_tool_event = matches!(hook, HookKind::PreTool | HookKind::PostTool);
    assert_eq!(event.hook, hook);
    assert!(is_tool_event || event.tool.is_none() && event.tool_target.is_none());
    assert!(hook == HookKind::PostTool || event.tool_result.is_none());
    assert!(hook == HookKind::Prompt || event.prompt_text.is_none());
    assert!(hook == HookKind::SessionStart || event.source.is_none());
    assert!(
        event
            .prompt_text
            .as_ref()
            .is_none_or(|prompt| prompt.len() <= 131_072)
    );
    if let Some(target) = &event.tool_target {
        assert!(
            target
                .pattern
                .as_ref()
                .is_none_or(|pattern| pattern.chars().count() <= 1_000)
        );
        assert!(
            target
                .ranges
                .iter()
                .all(|range| range.start_line() >= 1 && range.start_line() <= range.end_line())
        );
        let hits = &target.hits;
        assert!(hook == HookKind::PostTool || hits.is_empty());
        assert!(hits.len() <= 50);
        assert!(hits.iter().map(|path| path.as_str().len()).sum::<usize>() <= 4_096);
        assert!(!hits.contains(&RepoPath::root()));
        assert_eq!(
            hits.iter().collect::<std::collections::BTreeSet<_>>().len(),
            hits.len()
        );
    }
}

/// Any JSON value a few levels deep, its object keys drawn from `fields` and its strings from
/// `texts` or of any shape.
pub fn json_value(
    fields: &'static [&'static str],
    texts: &'static [&'static str],
) -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        prop::num::f64::NORMAL.prop_map(Value::from),
        prop::sample::select(texts).prop_map(Value::from),
        ".{0,40}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 32, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
            prop::collection::btree_map(prop::sample::select(fields), inner, 0..8).prop_map(
                |fields| {
                    Value::Object(
                        fields
                            .into_iter()
                            .map(|(key, value)| (key.to_owned(), value))
                            .collect(),
                    )
                }
            ),
        ]
    })
}
