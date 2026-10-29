//! The captured hook payloads are intact, complete and scrubbed.
//!
//! Every fixture must parse as a JSON object whose `hook_event_name` matches its file name, every
//! event the adapters handle must have a fixture for each captured agent version, and no fixture may
//! carry a real path or session id.

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
const REQUIRED: &[(&str, &str, &[&str])] = &[(
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
)];

/// Folder prefixes a scrubbed `cwd` may start with.
const SCRUBBED_REPO_ROOTS: &[&str] = &["C:\\repo", "/repo"];

/// Folder prefixes a scrubbed transcript path may start with.
const SCRUBBED_HOMES: &[&str] = &["C:\\Users\\dev\\", "/home/dev/"];

/// Prefix of every scrubbed UUID-style session id.
const SCRUBBED_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

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

/// No fixture carries a real path, a real session id or a carriage return.
#[test]
fn fixtures_are_scrubbed() -> TestResult<()> {
    for dir in version_dirs()? {
        for path in json_files(&dir)? {
            let bytes = fs::read(&path)?;
            assert!(
                !bytes.contains(&b'\r'),
                "{}: CRLF line ending",
                path.display()
            );
            let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
            let field = |key: &str| payload.get(key).and_then(|value| value.as_str());
            if let Some(cwd) = field("cwd") {
                assert!(
                    SCRUBBED_REPO_ROOTS.iter().any(|root| cwd.starts_with(root)),
                    "{}: cwd {cwd:?} is not a scrubbed repository path",
                    path.display()
                );
            }
            if let Some(transcript) = field("transcript_path") {
                assert!(
                    SCRUBBED_HOMES
                        .iter()
                        .any(|home| transcript.starts_with(home)),
                    "{}: transcript path {transcript:?} is not under a scrubbed home",
                    path.display()
                );
            }
            if let Some(session) = field("session_id") {
                assert!(
                    session.starts_with(SCRUBBED_UUID_PREFIX),
                    "{}: session id {session:?} is not a fixed id",
                    path.display()
                );
            }
        }
    }
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
