//! The captured hook payloads are intact, complete and scrubbed.
//!
//! Every fixture must parse as a JSON object whose `hook_event_name` matches its file name, every
//! event the adapters handle must have a fixture for each captured agent version, and no fixture may
//! carry a real path or id anywhere in its values.

use std::error::Error;
use std::fs;
use std::path::Path;

use cortex_tests::{fixtures_dir, json_files, payload_dir};

/// Result type for these tests: any I/O or parse failure fails the test.
type TestResult<T> = Result<T, Box<dyn Error>>;

/// File-name prefixes and the event each names, longest prefix first so that
/// `post-tool-use-failure` is not read as `post-tool-use`.
const EVENT_PREFIXES: &[(&str, &str)] = &[
    ("post-tool-use-failure", "PostToolUseFailure"),
    ("post-tool-use", "PostToolUse"),
    ("pre-tool-use", "PreToolUse"),
    ("user-prompt-submit", "UserPromptSubmit"),
    ("session-start", "SessionStart"),
    ("session-end", "SessionEnd"),
    ("subagent-start", "SubagentStart"),
    ("subagent-stop", "SubagentStop"),
    ("pre-compact", "PreCompact"),
    ("post-compact", "PostCompact"),
    ("stop", "Stop"),
];

/// The fixtures each captured agent version must have: every event and tool the adapters handle.
const REQUIRED: &[(&str, &str, &[&str])] = &[
    (
        "claude",
        "2.1.312",
        &[
            "session-start-startup",
            "session-start-resume",
            "session-start-compact",
            "user-prompt-submit",
            "pre-tool-use-read",
            "pre-tool-use-grep",
            "pre-tool-use-glob",
            "pre-tool-use-edit",
            "pre-tool-use-write",
            "pre-tool-use-bash",
            "pre-tool-use-agent",
            "pre-tool-use-grep-subagent",
            "post-tool-use-read",
            "post-tool-use-grep",
            "post-tool-use-glob",
            "post-tool-use-edit",
            "post-tool-use-write",
            "post-tool-use-bash",
            "post-tool-use-agent",
            "post-tool-use-grep-subagent",
            "post-tool-use-failure-bash",
            "pre-compact-manual",
            "post-compact-manual",
            "subagent-start",
            "subagent-stop",
            "subagent-stop-internal",
            "stop",
            "session-end",
        ],
    ),
    (
        "codex",
        "0.165.0",
        &[
            "session-start-startup",
            "session-start-compact",
            "user-prompt-submit",
            "pre-tool-use-bash-grep",
            "pre-tool-use-bash-read",
            "pre-tool-use-bash-test",
            "pre-tool-use-bash-subagent",
            "pre-tool-use-apply-patch",
            "pre-tool-use-spawn-agent",
            "pre-tool-use-wait-agent",
            "post-tool-use-bash-grep",
            "post-tool-use-bash-read",
            "post-tool-use-bash-test-failing",
            "post-tool-use-bash-test-passing",
            "post-tool-use-bash-nonzero-exit",
            "post-tool-use-bash-subagent",
            "post-tool-use-apply-patch",
            "post-tool-use-apply-patch-add",
            "post-tool-use-spawn-agent",
            "post-tool-use-wait-agent",
            "pre-compact-auto",
            "post-compact-auto",
            "subagent-start",
            "subagent-stop",
            "stop",
            "session-end",
        ],
    ),
];

/// Folder prefixes a scrubbed `cwd` may start with.
const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with.
const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "/home/dev/"];

/// Path pieces that are followed by a user name, on Windows and Unix.
const HOME_MARKERS: &[&str] = &["\\Users\\", "/Users/", "/home/"];

/// The only user name a scrubbed home folder may show.
const SCRUBBED_USER: &str = "dev";

/// Prefix of every scrubbed UUID-style id.
const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// Length of a UUID in its hyphenated text form: `8-4-4-4-12` hexadecimal digits.
const UUID_LEN: usize = 36;

/// The event a fixture file name says it holds.
fn expected_event(file_name: &str) -> Option<&'static str> {
    let stem = file_name.strip_suffix(".json")?;
    EVENT_PREFIXES
        .iter()
        .find(|(prefix, _)| {
            stem == *prefix
                || stem
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('-'))
        })
        .map(|(_, event)| *event)
}

/// Every `fixtures/<agent>/<version>` folder, sorted.
fn version_dirs() -> TestResult<Vec<std::path::PathBuf>> {
    let mut dirs = Vec::new();
    for agent in fs::read_dir(fixtures_dir())? {
        let agent = agent?.path();
        if agent.is_dir() {
            for version in fs::read_dir(&agent)? {
                let version = version?.path();
                if version.is_dir() {
                    dirs.push(version);
                }
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// The file name of `path` as text.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every string value inside `value`, depth first.
fn strings_in(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(text) => vec![text.as_str()],
        serde_json::Value::Array(items) => items.iter().flat_map(strings_in).collect(),
        serde_json::Value::Object(fields) => fields.values().flat_map(strings_in).collect(),
        _ => Vec::new(),
    }
}

/// Whether `bytes` is a UUID in its hyphenated `8-4-4-4-12` hexadecimal form.
fn is_uuid(bytes: &[u8]) -> bool {
    bytes.len() == UUID_LEN
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Every UUID-shaped run of characters in `text`, in order.
fn uuids_in(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    (0..=bytes.len().saturating_sub(UUID_LEN))
        .filter(|&start| bytes.get(start..start + UUID_LEN).is_some_and(is_uuid))
        .filter_map(|start| text.get(start..start + UUID_LEN))
        .collect()
}

/// Whether the text after a home marker names the scrubbed user, followed by a separator or nothing.
fn is_scrubbed_user(rest: &str) -> bool {
    rest.strip_prefix(SCRUBBED_USER)
        .is_some_and(|after| after.is_empty() || after.starts_with(['\\', '/']))
}

/// The ids and home folders in one string that were not replaced by fixed values.
fn text_problems(text: &str) -> Vec<String> {
    let ids = uuids_in(text)
        .into_iter()
        .filter(|id| !id.starts_with(SCRUBBED_UUID_PREFIX))
        .map(|id| format!("id {id} is not a fixed id"));
    let homes = HOME_MARKERS
        .iter()
        .flat_map(|marker| {
            text.match_indices(marker)
                .filter_map(move |(index, _)| text.get(index + marker.len()..))
        })
        .filter(|rest| !is_scrubbed_user(rest))
        .map(|rest| {
            let user: String = rest
                .chars()
                .take_while(|character| !matches!(character, '\\' | '/'))
                .collect();
            format!("home folder of user {user:?} is not scrubbed")
        });
    ids.chain(homes).collect()
}

/// What in one fixture still looks captured rather than scrubbed.
fn scrub_problems(bytes: &[u8], payload: &serde_json::Value) -> Vec<String> {
    let mut problems = Vec::new();
    if bytes.contains(&b'\r') {
        problems.push("CRLF line ending".to_owned());
    }
    let field = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
    if let Some(cwd) = field("cwd")
        && !SCRUBBED_REPO_ROOTS.iter().any(|root| cwd.starts_with(root))
    {
        problems.push(format!("cwd {cwd:?} is not a scrubbed repository path"));
    }
    if let Some(transcript) = field("transcript_path")
        && !SCRUBBED_HOMES
            .iter()
            .any(|home| transcript.starts_with(home))
    {
        problems.push(format!(
            "transcript path {transcript:?} is not under a scrubbed home"
        ));
    }
    if let Some(session) = field("session_id")
        && !session.starts_with(SCRUBBED_UUID_PREFIX)
    {
        problems.push(format!("session id {session:?} is not a fixed id"));
    }
    problems.extend(strings_in(payload).into_iter().flat_map(text_problems));
    problems
}

/// Every fixture parses as an object and names the event its file name promises.
#[test]
fn every_fixture_names_its_event() -> TestResult<()> {
    let mut checked = 0;
    for dir in version_dirs()? {
        for path in json_files(&dir)? {
            let name = file_name(&path);
            let event = expected_event(&name).ok_or(format!("{name}: no known event prefix"))?;
            let payload: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            assert_eq!(
                payload
                    .get("hook_event_name")
                    .and_then(|value| value.as_str()),
                Some(event),
                "{}",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "no fixtures found under {}",
        fixtures_dir().display()
    );
    Ok(())
}

/// Each captured agent version has a fixture for every event and tool the adapters handle.
#[test]
fn required_fixtures_exist() -> TestResult<()> {
    for (agent, version, names) in REQUIRED {
        let dir = payload_dir(agent, version);
        for name in *names {
            let path = dir.join(format!("{name}.json"));
            assert!(path.is_file(), "missing fixture {}", path.display());
        }
    }
    Ok(())
}

/// No fixture carries a real path, a real id or a carriage return, in any of its values.
#[test]
fn fixtures_are_scrubbed() -> TestResult<()> {
    let mut problems = Vec::new();
    for dir in version_dirs()? {
        for path in json_files(&dir)? {
            let bytes = fs::read(&path)?;
            let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
            problems.extend(
                scrub_problems(&bytes, &payload)
                    .into_iter()
                    .map(|problem| format!("{}: {problem}", path.display())),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    Ok(())
}

/// The prefix table maps names to events, including the failure and subagent variants.
#[test]
fn file_names_map_to_events() {
    assert_eq!(
        expected_event("post-tool-use-failure-bash.json"),
        Some("PostToolUseFailure")
    );
    assert_eq!(
        expected_event("post-tool-use-bash.json"),
        Some("PostToolUse")
    );
    assert_eq!(
        expected_event("subagent-stop-internal.json"),
        Some("SubagentStop")
    );
    assert_eq!(expected_event("stop.json"), Some("Stop"));
    assert_eq!(expected_event("stopped.json"), None);
    assert_eq!(expected_event("notes.txt"), None);
}

/// The text scan flags ids and home folders that were not replaced, and nothing else.
#[test]
fn text_scan_flags_unscrubbed_ids_and_homes() {
    assert!(
        text_problems("C:\\Users\\dev\\.codex\\rollout-00000000-0000-4000-8000-000000000001.jsonl")
            .is_empty()
    );
    assert!(text_problems("/home/dev/project and /c/Users/dev").is_empty());
    assert!(text_problems("short-0000-4000 and 2026-10-29T06-03-56").is_empty());
    assert_eq!(
        text_problems("rollout-01a18642-06c8-7022-ac9c-fca4853cfe2c.jsonl").len(),
        1
    );
    assert_eq!(
        text_problems("exec-5aa55f07-61d9-4d37-baed-65ba37f4185f").len(),
        1
    );
    assert_eq!(text_problems("/home/alice/project").len(), 1);
    assert_eq!(text_problems("C:\\Users\\devon\\notes").len(), 1);
}
