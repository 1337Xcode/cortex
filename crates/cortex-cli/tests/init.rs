//! `cortex init` on temporary repositories with an empty home folder: Claude Code's settings get
//! Cortex's hooks beside the user's, backed up first with a manifest; a second run changes
//! nothing; a file it cannot read is refused and left; the user's scope writes the home folder's
//! settings.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{TestResult, isolate};
use cortex_core::ContentHash;
use serde_json::Value;

/// What one run of `cortex init` printed and how it ended.
struct Run {
    /// Whether it succeeded.
    is_success: bool,
    /// Its stdout.
    stdout: String,
    /// Its stderr.
    stderr: String,
}

/// Runs `cortex init --root <repo>` with `extra` arguments and `home` as the home folder.
fn init(repo: &Path, home: &Path, extra: &[&str]) -> TestResult<Run> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--root"]).arg(repo).args(extra);
    isolate(&mut command, home);
    let output = command.output()?;
    Ok(Run {
        is_success: output.status.success(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// The backup folders `cortex init` made in `repo`.
fn backup_folders(repo: &Path) -> TestResult<Vec<PathBuf>> {
    let backups = repo.join(".cortex").join("backups");
    if !backups.exists() {
        return Ok(Vec::new());
    }
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(backups)? {
        let path = entry?.path();
        if path.is_dir() {
            folders.push(path);
        }
    }
    Ok(folders)
}

/// The Cortex hook `group` holds: its command and arguments, its timeout, and its matcher.
fn cortex_hook(group: &Value) -> Option<(String, Vec<String>, u64, Option<String>)> {
    let handler = group.get("hooks")?.as_array()?.first()?;
    let args = handler
        .get("args")?
        .as_array()?
        .iter()
        .filter_map(|arg| arg.as_str().map(str::to_owned))
        .collect();
    Some((
        handler.get("command")?.as_str()?.to_owned(),
        args,
        handler.get("timeout")?.as_u64()?,
        group
            .get("matcher")
            .and_then(Value::as_str)
            .map(str::to_owned),
    ))
}

/// A settings file holding the user's model and their own Stop hook gets one Cortex hook for each
/// of the ten events Cortex registers, after the user's, the user's key first and their event
/// where it was; the others follow in the adapter's order. Each runs this binary with `hook
/// --agent claude --event <event>`, with the event's timeout in seconds (2 for SessionStart,
/// 1 for the rest) and its matcher (`Read` before a tool runs, the recorded tools after it). The
/// file as it was is in the backup folder, and the manifest names it with the hash of what was
/// written.
#[test]
fn init_merges_the_hooks_and_keeps_a_backup() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    let original = r#"{"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify.sh"}]}]}}"#;
    std::fs::write(&settings, original)?;
    let run = init(repo.path(), home.path(), &[])?;
    assert!(run.is_success, "{}", run.stderr);
    let written = std::fs::read_to_string(&settings)?;
    let value: Value = serde_json::from_str(&written)?;
    let keys: Vec<&str> = value
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(keys, ["model", "hooks"]);
    let hooks = value
        .get("hooks")
        .and_then(Value::as_object)
        .ok_or("hooks")?;
    let events: Vec<&str> = hooks.keys().map(String::as_str).collect();
    let expected_events = [
        "Stop",
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PreCompact",
        "SubagentStart",
        "SubagentStop",
        "SessionEnd",
    ];
    assert_eq!(events, expected_events);
    let program = std::fs::canonicalize(env!("CARGO_BIN_EXE_cortex"))?;
    let tools = "Read|Grep|Glob|Edit|Write|MultiEdit|Bash|PowerShell";
    for event in expected_events {
        let groups = hooks.get(event).and_then(Value::as_array).ok_or(event)?;
        let ours = groups.last().and_then(cortex_hook).ok_or(event)?;
        let (command, args, timeout_s, matcher) = ours;
        assert_eq!(std::fs::canonicalize(&command)?, program, "{event}");
        assert_eq!(args, ["hook", "--agent", "claude", "--event", event]);
        let expected_timeout = if event == "SessionStart" { 2 } else { 1 };
        assert_eq!(timeout_s, expected_timeout, "{event}");
        let expected_matcher = match event {
            "PreToolUse" => Some("Read"),
            "PostToolUse" | "PostToolUseFailure" => Some(tools),
            _ => None,
        };
        assert_eq!(matcher.as_deref(), expected_matcher, "{event}");
        let expected_groups = if event == "Stop" { 2 } else { 1 };
        assert_eq!(groups.len(), expected_groups, "{event}");
    }
    let user_stop = hooks
        .get("Stop")
        .and_then(|groups| groups.get(0))
        .and_then(|group| group.pointer("/hooks/0/command"))
        .and_then(Value::as_str);
    assert_eq!(user_stop, Some("notify.sh"));
    let [folder] = backup_folders(repo.path())?
        .try_into()
        .map_err(|_| "one backup")?;
    let resolved = std::fs::canonicalize(&settings)?;
    let shown = run.stdout.trim_end();
    assert!(
        shown.starts_with("Claude Code: hooks written to ") && shown.ends_with(')'),
        "{shown}"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("1-settings.local.json"))?,
        original
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(folder.join("manifest.json"))?)?;
    let entry = manifest.pointer("/files/0").ok_or("an entry")?;
    assert_eq!(
        entry.get("backup").and_then(Value::as_str),
        Some("1-settings.local.json")
    );
    let named = entry.get("path").and_then(Value::as_str).ok_or("a path")?;
    assert_eq!(std::fs::canonicalize(named)?, resolved);
    assert_eq!(
        entry.get("written_blake3").and_then(Value::as_str),
        Some(ContentHash::of(written.as_bytes()).to_string().as_str())
    );
    Ok(())
}

/// A second run finds the hooks current: it says so, writes nothing and backs nothing up.
#[test]
fn init_again_changes_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    assert!(init(repo.path(), home.path(), &[])?.is_success);
    let settings = repo.path().join(".claude").join("settings.local.json");
    let first = std::fs::read(&settings)?;
    let again = init(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert!(
        again.stdout.starts_with("Claude Code: the hooks in ")
            && again.stdout.trim_end().ends_with(" are current"),
        "{}",
        again.stdout
    );
    assert_eq!(std::fs::read(&settings)?, first);
    assert_eq!(backup_folders(repo.path())?.len(), 1);
    Ok(())
}

/// With no settings file, one is made, and the manifest records that there was none.
#[test]
fn init_creates_the_settings_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let run = init(repo.path(), home.path(), &[])?;
    assert!(run.is_success, "{}", run.stderr);
    let settings = repo.path().join(".claude").join("settings.local.json");
    let value: Value = serde_json::from_slice(&std::fs::read(settings)?)?;
    assert_eq!(
        value
            .get("hooks")
            .and_then(Value::as_object)
            .map(serde_json::Map::len),
        Some(10)
    );
    let [folder] = backup_folders(repo.path())?
        .try_into()
        .map_err(|_| "one backup")?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(folder.join("manifest.json"))?)?;
    assert_eq!(manifest.pointer("/files/0/backup"), Some(&Value::Null));
    Ok(())
}

/// A settings file that is not JSON is refused with where it breaks and none of its text, and is
/// left as it was, with nothing backed up.
#[test]
fn init_refuses_a_file_it_cannot_read() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    std::fs::write(&settings, "{\"token\": oops")?;
    let run = init(repo.path(), home.path(), &[])?;
    assert!(!run.is_success);
    assert!(
        run.stderr.contains("it is not JSON (line 1, column ") && !run.stderr.contains("oops"),
        "{}",
        run.stderr
    );
    assert_eq!(std::fs::read_to_string(&settings)?, "{\"token\": oops");
    assert!(backup_folders(repo.path())?.is_empty());
    Ok(())
}

/// The user's scope writes `~/.claude/settings.json`, backs it up in `~/.cortex/backups`, and
/// makes nothing in the repository: neither `.claude` nor `.cortex`, which would make the folder a
/// repository for every hook below it.
#[test]
fn init_for_the_user_writes_the_home_settings() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let run = init(repo.path(), home.path(), &["--scope", "user"])?;
    assert!(run.is_success, "{}", run.stderr);
    let value: Value = serde_json::from_slice(&std::fs::read(
        home.path().join(".claude").join("settings.json"),
    )?)?;
    assert!(
        value
            .pointer("/hooks/SessionStart/0/hooks/0/args")
            .is_some()
    );
    let backups = home.path().join(".cortex").join("backups");
    assert_eq!(std::fs::read_dir(backups)?.count(), 1);
    assert!(!repo.path().join(".claude").exists());
    assert!(!repo.path().join(".cortex").exists());
    Ok(())
}

/// A settings file the user made read-only is left as it is: init says so and backs nothing up.
#[test]
fn init_leaves_a_read_only_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    std::fs::write(&settings, "{}")?;
    let mut permissions = std::fs::metadata(&settings)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&settings, permissions)?;
    let run = init(repo.path(), home.path(), &[])?;
    assert!(!run.is_success);
    assert!(run.stderr.contains("is read-only"), "{}", run.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, "{}");
    assert!(backup_folders(repo.path())?.is_empty());
    Ok(())
}

/// A repository whose `.claude` or `.cortex` folder is a link to a folder outside it is refused,
/// and the folder it leads to stays empty: nothing is written through it.
#[cfg(unix)]
#[test]
fn init_refuses_folders_that_lead_outside() -> TestResult {
    for name in [".claude", ".cortex"] {
        let repo = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        let elsewhere = tempfile::tempdir()?;
        std::os::unix::fs::symlink(elsewhere.path(), repo.path().join(name))?;
        let run = init(repo.path(), home.path(), &[])?;
        assert!(!run.is_success, "{name}");
        assert!(run.stderr.contains("leads elsewhere"), "{}", run.stderr);
        assert_eq!(std::fs::read_dir(elsewhere.path())?.count(), 0, "{name}");
    }
    Ok(())
}

/// On Windows the same holds of a junction, which a user may make without privileges: a `.cortex`
/// junction to a folder outside the repository is refused and nothing is written there.
#[cfg(windows)]
#[test]
fn init_refuses_a_junction_that_leads_outside() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let junction = repo.path().join(".cortex");
    let made = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(elsewhere.path())
        .output()?;
    assert!(made.status.success(), "{made:?}");
    let run = init(repo.path(), home.path(), &[])?;
    assert!(!run.is_success);
    assert!(run.stderr.contains("leads elsewhere"), "{}", run.stderr);
    assert_eq!(std::fs::read_dir(elsewhere.path())?.count(), 0);
    Ok(())
}

/// A settings file that is a link to a file that does not exist is left as it is: writing would
/// replace the link, losing where it led.
#[cfg(unix)]
#[test]
fn init_leaves_a_link_to_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::os::unix::fs::symlink(repo.path().join("gone.json"), &settings)?;
    let run = init(repo.path(), home.path(), &[])?;
    assert!(!run.is_success);
    assert!(
        run.stderr
            .contains("is a link to a file that does not exist"),
        "{}",
        run.stderr
    );
    assert!(
        std::fs::symlink_metadata(&settings)?
            .file_type()
            .is_symlink()
    );
    Ok(())
}
