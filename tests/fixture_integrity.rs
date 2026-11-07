//! The captured hook payloads and transcripts are intact, complete and scrubbed.
//!
//! Every captured fixture must parse as a JSON object whose `hook_event_name` matches its file
//! name; every event and tool the adapters handle must have a fixture for each captured agent
//! version, holding the fields its name promises (the table is in `src/required.rs`); every version
//! captured with payloads must be in that table; the fixture folder may hold nothing but payloads,
//! READMEs and, under a version's `transcripts/` folder, transcripts of one JSON object a line; and
//! no fixture, line or README may carry a real path, id, user name or email.
//!
//! A `docs-<date>` folder holds payloads written from an agent's documentation, not captured: they
//! carry the agent's own event names, and its adapter's tests read each by name, so the event
//! names and the captured versions' table do not apply to them; the scrub rules do. A
//! `plugin-<date>` folder holds what a plugin agent's generated plugin sent, one JSON call a file
//! under `calls/`, and golden copies of the generated plugin files under `generated/`: the same
//! scrub rules apply to both.

use std::error::Error;
use std::fs;
use std::path::Path;

use cortex_tests::required::{REQUIRED, unmet};
use cortex_tests::{files_under, fixtures_dir, json_files, payload_dir, scrub};

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

/// Whether the version folder `dir` holds payloads written from documentation (`docs-<date>`)
/// or a plugin's recorded calls (`plugin-<date>`), rather than captured ones.
fn is_not_captured(dir: &Path) -> bool {
    let name = file_name(dir);
    name.starts_with("docs-") || name.starts_with("plugin-")
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

/// `path` relative to the fixture folder, with `/` separators.
fn fixture_relative(path: &Path) -> String {
    let fixtures = fixtures_dir();
    let relative = path.strip_prefix(&fixtures).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every captured fixture parses as an object and names the event its file name promises.
#[test]
fn every_fixture_names_its_event() -> TestResult<()> {
    let mut checked = 0;
    for dir in version_dirs()?
        .into_iter()
        .filter(|dir| !is_not_captured(dir))
    {
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

/// Each captured agent version has every required fixture, and each holds the fields its name
/// promises.
#[test]
fn required_fixtures_exist_and_match_their_names() -> TestResult<()> {
    let mut problems = Vec::new();
    for (agent, version, required) in REQUIRED {
        for (name, expects) in *required {
            let path = payload_dir(agent, version).join(format!("{name}.json"));
            if !path.is_file() {
                problems.push(format!("missing fixture {}", path.display()));
                continue;
            }
            let payload: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            problems.extend(
                unmet(&payload, expects)
                    .into_iter()
                    .map(|problem| format!("{agent}/{version}/{name}.json: {problem}")),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    Ok(())
}

/// Every agent version captured with hook payloads has a list of required fixtures, so a new
/// version folder of payloads cannot skip the completeness check. A version folder that holds only
/// a transcript (Codex 0.166.0's rollout) has no payloads to require, and one of payloads written
/// from documentation is its adapter's tests' to check.
#[test]
fn every_version_folder_is_required() -> TestResult<()> {
    let mut with_payloads = Vec::new();
    for dir in version_dirs()?
        .into_iter()
        .filter(|dir| !is_not_captured(dir))
    {
        if !json_files(&dir)?.is_empty() {
            with_payloads.push(dir);
        }
    }
    let unlisted: Vec<String> = with_payloads
        .iter()
        .map(|dir| fixture_relative(dir))
        .filter(|relative| {
            !REQUIRED
                .iter()
                .any(|(agent, version, _)| *relative == format!("{agent}/{version}"))
        })
        .collect();
    assert!(
        unlisted.is_empty(),
        "fixture folders missing from the REQUIRED table: {unlisted:?}"
    );
    Ok(())
}

/// The fixture folder holds only `<agent>/<version>/` folders of JSON payloads and READMEs,
/// transcripts under a version's `transcripts/` folder, and in a `plugin-<date>` folder the calls
/// under `calls/` and the generated plugin files (JavaScript and JSON) under `generated/`, so
/// nothing escapes the checks by sitting elsewhere or under another extension.
#[test]
fn fixture_tree_holds_only_payloads_and_readmes() -> TestResult<()> {
    let unexpected: Vec<String> = files_under(&fixtures_dir())?
        .iter()
        .map(|path| fixture_relative(path))
        .filter(|relative| {
            let parts: Vec<&str> = relative.split('/').collect();
            let is_payload = matches!(parts.as_slice(), [_, _, name]
                if *name == "README.md" || name.ends_with(".json"));
            let is_transcript = matches!(parts.as_slice(), [_, _, "transcripts", .., name]
                if name.ends_with(".jsonl"));
            let is_call = matches!(parts.as_slice(), [_, version, "calls", name]
                if version.starts_with("plugin-") && name.ends_with(".json"));
            let is_generated = matches!(parts.as_slice(), [_, version, "generated", .., name]
                if version.starts_with("plugin-")
                    && (name.ends_with(".js") || name.ends_with(".json")));
            !is_payload && !is_transcript && !is_call && !is_generated
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected files under the fixture folder: {unexpected:?}"
    );
    Ok(())
}

/// No fixture or README carries a real path, id, opaque hash, user or machine name, email address
/// or carriage return.
#[test]
fn fixtures_are_scrubbed() -> TestResult<()> {
    let mut problems = Vec::new();
    for path in files_under(&fixtures_dir())? {
        let bytes = fs::read(&path)?;
        let mut found = Vec::new();
        if bytes.contains(&b'\r') {
            found.push("CRLF line ending".to_owned());
        }
        if file_name(&path) == "README.md" || file_name(&path).ends_with(".js") {
            found.extend(scrub::text_problems(&String::from_utf8(bytes)?));
        } else if file_name(&path).ends_with(".jsonl") {
            for line in String::from_utf8(bytes)?.lines() {
                let entry: serde_json::Value = serde_json::from_str(line)?;
                found.extend(scrub::payload_problems(&entry));
            }
        } else {
            let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
            found.extend(scrub::payload_problems(&payload));
        }
        problems.extend(
            found
                .into_iter()
                .map(|problem| format!("{}: {problem}", fixture_relative(&path))),
        );
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
