//! `cortex init` on temporary repositories with an empty home folder and a PATH of the test's own
//! (git, and a stand-in for Claude Code or none): Claude Code's settings get Cortex's hooks beside
//! the user's, backed up first with a manifest; a second run changes nothing; a file it cannot
//! read is refused and left; the user's scope writes the home folder's settings; the repository is
//! git's, never the home folder; a Claude Code too old is refused before anything is written; the
//! questions are asked once, their answers kept in the configuration and the ignores, and nothing
//! is written while one is unanswered.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fake_daemon::{FakeDaemon, Script, kinds};
use common::init::{claude_in, git, init, init_without_yes, run};
use common::{StopsDaemon, TestResult, stand_in_claude};
use cortex_core::ipc::CORTEX_VERSION;
use cortex_core::{ContentHash, canonical_root};
use serde_json::Value;

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
/// file as it was is in the backup folder, and the manifest names the repository and the file, its
/// kind, its copy with the copy's hash, and the hash of what was written. The user's backup folder
/// holds the same manifest, byte for byte, in a run folder of the same name.
#[test]
fn init_merges_the_hooks_and_keeps_a_backup() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    std::fs::create_dir_all(repo.path().join(".claude"))?;
    let original = r#"{"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify.sh"}]}]}}"#;
    std::fs::write(&settings, original)?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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
    // Init's own lines; the daemon's, the index's and doctor's follow.
    let lines: Vec<&str> = run.stdout.lines().take(8).collect();
    let [
        repository,
        no_git,
        claude,
        written_to,
        repository_answer,
        users_answer,
        backed_up,
        users_backed_up,
    ] = lines.as_slice()
    else {
        return Err(run.stdout.into());
    };
    assert_eq!(
        *repository,
        format!("repository: {}", canonical_root(repo.path())?.display())
    );
    // Git ends with 128 outside a repository, which Rust shows as an exit code on Windows and an
    // exit status elsewhere.
    let ended = if cfg!(windows) {
        "exit code: 128"
    } else {
        "exit status: 128"
    };
    assert_eq!(
        *no_git,
        format!(
            "note: git named no repository here (it ended with {ended}); Cortex sets up this \
             folder alone, without git history"
        )
    );
    assert_eq!(
        *claude,
        format!(
            "Claude Code 2.1.312 at {}, a version Cortex works with",
            claude_in(home.path()).display()
        )
    );
    let shown = canonical_root(repo.path())?
        .join(".claude")
        .join("settings.local.json");
    assert_eq!(
        *written_to,
        format!("Claude Code: hooks written to {}", shown.display())
    );
    let config = canonical_root(repo.path())?
        .join(".cortex")
        .join("config.toml");
    assert_eq!(
        *repository_answer,
        format!("answer kept in {}", config.display())
    );
    let users_config = home.path().join(".cortex").join("config.toml");
    assert_eq!(
        *users_answer,
        format!("answer kept in {}", users_config.display())
    );
    assert_eq!(
        *backed_up,
        format!("backup and manifest in {}", folder.display())
    );
    assert!(
        users_backed_up.starts_with(&format!(
            "backup and manifest in {}",
            home.path().join(".cortex").join("backups").display()
        )),
        "{users_backed_up}"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("1-settings.local.json"))?,
        original
    );
    let manifest_bytes = std::fs::read(folder.join("manifest.json"))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)?;
    assert_eq!(
        manifest.get("repository").and_then(Value::as_str),
        canonical_root(repo.path())?.to_str()
    );
    let entry = manifest.pointer("/files/0").ok_or("an entry")?;
    assert_eq!(
        entry.get("kind").and_then(Value::as_str),
        Some("claude_settings")
    );
    assert_eq!(
        entry.pointer("/backup/name").and_then(Value::as_str),
        Some("1-settings.local.json")
    );
    assert_eq!(
        entry.pointer("/backup/blake3").and_then(Value::as_str),
        Some(ContentHash::of(original.as_bytes()).to_string().as_str())
    );
    let run = folder.file_name().ok_or("a run folder")?;
    let users_copy = home
        .path()
        .join(".cortex")
        .join("backups")
        .join(run)
        .join("repository-manifest.json");
    assert_eq!(std::fs::read(users_copy)?, manifest_bytes);
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
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    assert!(init(repo.path(), home.path(), &[])?.is_success);
    let settings = repo.path().join(".claude").join("settings.local.json");
    let first = std::fs::read(&settings)?;
    let again = init(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert!(
        again
            .stdout
            .lines()
            .any(|line| line.starts_with("Claude Code: the hooks in ")
                && line.ends_with(" are current")),
        "{}",
        again.stdout
    );
    assert_eq!(std::fs::read(&settings)?, first);
    assert_eq!(backup_folders(repo.path())?.len(), 1);
    Ok(())
}

/// From a folder inside a git repository below the home folder, with no `--root`, init sets up the
/// repository's top: it says so first, and the hooks go to the top's `.claude/settings.local.json`,
/// none to the folder.
#[test]
fn init_sets_up_git_s_repository_from_a_folder_inside_it() -> TestResult {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir(&repo)?;
    git(&repo, &["init", "-q"], home.path())?;
    let inner = repo.join("src").join("inner");
    std::fs::create_dir_all(&inner)?;
    stand_in_claude(home.path(), "2.1.312")?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--yes"]).current_dir(&inner);
    let _daemon = StopsDaemon::of(&repo, home.path());
    let done = run(&mut command, home.path(), &["git"])?;
    assert!(done.is_success, "{}", done.stderr);
    let top = canonical_root(&repo)?;
    assert_eq!(
        done.stdout.lines().next(),
        Some(format!("repository: {}", top.display()).as_str())
    );
    assert!(top.join(".claude").join("settings.local.json").is_file());
    assert!(!inner.join(".claude").exists());
    Ok(())
}

/// A repository whose `core.worktree` names a folder that does not hold it gets nothing written
/// there: init sets up the folder it starts from alone, and says why.
#[test]
fn init_never_follows_git_to_a_folder_outside() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let worktree = elsewhere.path().display().to_string();
    git(
        repo.path(),
        &["config", "core.worktree", worktree.as_str()],
        home.path(),
    )?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(
        done.stdout.lines().nth(1),
        Some(
            "note: git names a repository top that does not hold this folder; Cortex sets up \
             this folder alone, without git history"
        )
    );
    assert!(
        repo.path()
            .join(".claude")
            .join("settings.local.json")
            .is_file()
    );
    assert!(!elsewhere.path().join(".claude").exists());
    Ok(())
}

/// A folder inside a home folder that is itself a git repository, as when dotfiles are kept in
/// git, is set up alone: git's repository is the home folder, whose `.cortex` is the user's own.
/// The home folder itself is refused, and nothing is written there.
#[test]
fn init_never_sets_up_a_home_folder_repository() -> TestResult {
    let home = tempfile::tempdir()?;
    git(home.path(), &["init", "-q"], home.path())?;
    let project = home.path().join("project");
    std::fs::create_dir(&project)?;
    let _daemon = StopsDaemon::of(&project, home.path());
    let done = init(&project, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let lines: Vec<&str> = done.stdout.lines().take(2).collect();
    assert_eq!(
        lines,
        [
            format!("repository: {}", canonical_root(&project)?.display()).as_str(),
            "note: git's repository here is the home folder or holds it, which Cortex never sets \
             up; Cortex sets up this folder alone, without git history"
        ]
    );
    assert!(
        project
            .join(".claude")
            .join("settings.local.json")
            .is_file()
    );
    let refused = init(home.path(), home.path(), &[])?;
    assert!(!refused.is_success);
    assert_eq!(
        refused.stderr.trim_end(),
        format!(
            "cortex: error: Cortex never sets up the home folder or a folder holding it ({}); run \
             `cortex init` in a repository",
            canonical_root(home.path())?.display()
        )
    );
    assert!(!home.path().join(".claude").exists());
    Ok(())
}

/// With neither git nor Claude Code on the PATH, init says both, sets up the folder alone and
/// writes the hooks.
#[test]
fn init_without_git_or_claude_code_says_so() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--yes", "--root"]).arg(repo.path());
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let lines: Vec<&str> = done.stdout.lines().skip(1).take(2).collect();
    assert_eq!(
        lines,
        [
            "note: git is not on the PATH; Cortex sets up this folder alone, without git history",
            "note: Claude Code is not on the PATH, so its version cannot be asked"
        ]
    );
    assert!(
        repo.path()
            .join(".claude")
            .join("settings.local.json")
            .is_file()
    );
    Ok(())
}

/// A folder whose name holds a control character is printed with the character escaped, so a
/// repository's name cannot drive the terminal.
// Only Unix allows control characters in a file name; Windows refuses them.
#[cfg(unix)]
#[test]
fn init_prints_control_characters_escaped() -> TestResult {
    let base = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let repo = base.path().join("repo\u{1b}[31m");
    std::fs::create_dir(&repo)?;
    let _daemon = StopsDaemon::of(&repo, home.path());
    let done = init(&repo, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let shown =
        format!("repository: {}", canonical_root(&repo)?.display()).replace('\u{1b}', "\\u{1b}");
    assert_eq!(done.stdout.lines().next(), Some(shown.as_str()));
    assert!(!done.stdout.contains('\u{1b}'), "{}", done.stdout);
    Ok(())
}

/// A Claude Code older than the oldest version Cortex's hooks work with is refused before anything
/// is written, the error naming it, where it is and the update; one of a version Cortex works with
/// is named and the hooks are written.
#[test]
fn init_refuses_an_older_claude_code() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let claude = stand_in_claude(home.path(), "2.1.200")?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = init(repo.path(), home.path(), &[])?;
    assert!(!refused.is_success);
    let expected = format!(
        "cortex: error: Claude Code 2.1.200 at {} is older than 2.1.295, the oldest Cortex's hooks \
         work with; update Claude Code to 2.1.295 or later, then run `cortex init` again",
        claude.display()
    );
    assert_eq!(refused.stderr.trim_end(), expected);
    assert!(!repo.path().join(".claude").exists());
    assert!(!repo.path().join(".cortex").exists());
    stand_in_claude(home.path(), "2.1.300")?;
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let named = format!(
        "Claude Code 2.1.300 at {}, a version Cortex works with",
        claude.display()
    );
    assert!(
        done.stdout.lines().any(|line| line == named),
        "{}",
        done.stdout
    );
    assert!(
        repo.path()
            .join(".claude")
            .join("settings.local.json")
            .is_file()
    );
    Ok(())
}

/// In a git repository holding two Python files, init ends with the repository's daemon running,
/// its first index ready, and doctor's checks. The index, counted from the fixture: the two files
/// and the `.gitignore` init wrote are stored, the two Python files parsed, and Claude Code's local
/// settings left out, as the `.gitignore` says. Doctor's daemon check names the process init
/// started; its latency checks may fail on a busy machine, which init reports without failing.
#[test]
fn init_starts_the_daemon_and_runs_doctor() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    git(repo.path(), &["init", "-q"], home.path())?;
    std::fs::write(repo.path().join("app.py"), "def app():\n    pass\n")?;
    std::fs::write(repo.path().join("lib.py"), "def lib():\n    pass\n")?;
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let lines: Vec<&str> = done.stdout.lines().collect();
    let running = lines
        .iter()
        .find_map(|line| line.strip_prefix("daemon: running (process "))
        .ok_or_else(|| done.stdout.clone())?;
    let (pid, version) = running
        .strip_suffix(')')
        .and_then(|rest| rest.split_once(", Cortex "))
        .ok_or_else(|| done.stdout.clone())?;
    assert_eq!(version, CORTEX_VERSION);
    let index = lines
        .iter()
        .position(|line| *line == "index: ready")
        .ok_or_else(|| done.stdout.clone())?;
    assert_eq!(
        lines.get(index..index + 3),
        Some(&["index: ready", "mode: tiny", "files: 3 stored, 2 parsed"][..]),
        "{}",
        done.stdout
    );
    let checks: Vec<&str> = lines
        .iter()
        .skip(index + 3)
        // Doctor's own lines: init's closing note, when a check failed, is not one of them.
        .filter(|line| {
            ["ok    ", "FAIL  ", "note  "]
                .iter()
                .any(|kind| line.starts_with(kind))
        })
        .filter(|line| !line.contains("latency: "))
        .filter_map(|line| line.split_once(':').map(|(head, _)| head))
        .collect();
    assert_eq!(
        checks,
        [
            "ok    hooks",
            "ok    silenced",
            "ok    binary",
            "ok    agent",
            "ok    daemon"
        ],
        "{}",
        done.stdout
    );
    assert!(
        lines.contains(
            &format!("ok    daemon: running (process {pid}, Cortex {CORTEX_VERSION})").as_str()
        ),
        "{}",
        done.stdout
    );
    let probed = lines
        .iter()
        .filter(|line| line.contains("latency: "))
        .count();
    assert_eq!(probed, 5, "{}", done.stdout);
    Ok(())
}

/// Run again once its daemon has stopped, init waits for the new daemon's own first index, not for
/// what the earlier one left in the store: a file added meanwhile is counted (from the fixture:
/// `app.py` and the `.gitignore` first, then `lib.py` too).
#[test]
fn init_waits_for_the_new_daemon_s_index() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let daemon = StopsDaemon::of(repo.path(), home.path());
    git(repo.path(), &["init", "-q"], home.path())?;
    std::fs::write(repo.path().join("app.py"), "def app():\n    pass\n")?;
    let first = init(repo.path(), home.path(), &[])?;
    assert!(first.is_success, "{}", first.stderr);
    assert!(
        first.stdout.contains("\nfiles: 2 stored, 1 parsed\n"),
        "{}",
        first.stdout
    );
    daemon.stop()?;
    std::fs::write(repo.path().join("lib.py"), "def lib():\n    pass\n")?;
    let again = init(repo.path(), home.path(), &[])?;
    assert!(again.is_success, "{}", again.stderr);
    assert!(
        again.stdout.contains("\nfiles: 3 stored, 2 parsed\n"),
        "{}",
        again.stdout
    );
    Ok(())
}

/// A daemon that stops before its first index is done, here on a store that is no database, fails
/// init within seconds, naming the daemon, rather than waiting out the index's 5 minutes.
#[test]
fn init_fails_when_the_daemon_stops() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::fs::write(
        repo.path().join(".cortex").join("cortex.db"),
        "not a database",
    )?;
    let started = std::time::Instant::now();
    let done = init(repo.path(), home.path(), &[])?;
    assert!(!done.is_success, "{}", done.stdout);
    assert!(done.stderr.contains("daemon"), "{}", done.stderr);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(120),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// A daemon that answers while it indexes and then stops, before its first index is done, fails
/// init at once, naming the daemon's log: a fake daemon of this build answers two statuses, then
/// reads the third connection's handshake and stops without answering, as one that crashes.
#[test]
fn init_fails_when_the_daemon_stops_while_it_indexes() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let fake = FakeDaemon::start(repo.path(), home.path(), Script::Indexing(2))?;
    let done = init(repo.path(), home.path(), &[])?;
    assert!(!done.is_success, "{}", done.stdout);
    assert!(
        done.stderr
            .contains("the daemon stopped before its first index was done; its log says why: "),
        "{}",
        done.stderr
    );
    assert_eq!(
        kinds(&fake.requests()?),
        ["hello", "status", "hello", "status", "hello"]
    );
    Ok(())
}

/// Doctor's failures are reported and init still succeeds, its files written: with hooks turned off
/// in the user's settings, the silenced check fails and init says init is done.
#[test]
fn init_reports_doctor_s_failures_without_failing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    std::fs::create_dir(home.path().join(".claude"))?;
    std::fs::write(
        home.path().join(".claude").join("settings.json"),
        r#"{"disableAllHooks": true}"#,
    )?;
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(
        done.stdout
            .lines()
            .any(|line| line.starts_with("FAIL  silenced: ")),
        "{}",
        done.stdout
    );
    let note = done
        .stdout
        .lines()
        .find(|line| line.starts_with("note: init is done, and "))
        .ok_or_else(|| done.stdout.clone())?;
    assert!(
        note.contains(" of doctor's checks failed; each says what to do"),
        "{note}"
    );
    Ok(())
}

/// The answer files `--yes` writes: the repository's holdout, at one task in ten.
const REPOSITORY_ANSWER: &str =
    "# The answer to a question `cortex init` asked; change it here.\n[holdout]\nshare = 0.1\n";

/// The user's answer `--yes` writes: no upkeep by a language model.
const USERS_ANSWER: &str = "# The answer to a question `cortex init` asked; change it here.\n[upkeep.llm]\nenabled = false\n";

/// With `--yes`, the defaults: a holdout of one task in ten in the repository's configuration, no
/// upkeep by a language model in the user's own, no sharing, so git leaves out all of `.cortex/`
/// and the local settings init made.
#[test]
fn init_with_yes_takes_the_defaults() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(!done.stderr.contains('?'), "{}", done.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".cortex").join("config.toml"))?,
        REPOSITORY_ANSWER
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join(".cortex").join("config.toml"))?,
        USERS_ANSWER
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore"))?,
        "# Cortex\n.cortex/\n.claude/settings.local.json\n"
    );
    // The ignores are written first, so that a run stopped partway never leaves the local settings
    // for git to commit.
    let [folder] = backup_folders(repo.path())?
        .try_into()
        .map_err(|_| "one backup")?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(folder.join("manifest.json"))?)?;
    let first = manifest
        .pointer("/files/0/path")
        .and_then(Value::as_str)
        .ok_or("a first file")?;
    assert!(first.ends_with(".gitignore"), "{first}");
    let kinds: Vec<&str> = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or("files")?
        .iter()
        .filter_map(|file| file.get("kind").and_then(Value::as_str))
        .collect();
    assert_eq!(kinds, ["gitignore", "claude_settings", "cortex_config"]);
    let run = folder.file_name().ok_or("a run folder")?;
    let users_manifest = home
        .path()
        .join(".cortex")
        .join("backups")
        .join(run)
        .join("manifest.json");
    let users: Value = serde_json::from_slice(&std::fs::read(users_manifest)?)?;
    assert_eq!(
        users.pointer("/files/0/kind").and_then(Value::as_str),
        Some("cortex_config")
    );
    Ok(())
}

/// Once answered, nothing is asked again, even with no terminal and no `--yes`: the second run
/// changes nothing and backs nothing up.
#[test]
fn init_asks_nothing_once_answered() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let first = init(repo.path(), home.path(), &[])?;
    assert!(first.is_success, "{}", first.stderr);
    let ignores = std::fs::read_to_string(repo.path().join(".gitignore"))?;
    let again = init_without_yes(repo.path(), home.path())?;
    assert!(again.is_success, "{}", again.stderr);
    assert!(!again.stderr.contains('?'), "{}", again.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore"))?,
        ignores
    );
    assert_eq!(backup_folders(repo.path())?.len(), 1);
    Ok(())
}

/// With a question to ask, no terminal and no `--yes`, init names the question and writes nothing
/// anywhere. With the answers kept by hand (the holdout in the repository's configuration, the
/// upkeep in the user's own) it asks nothing; outside git, sharing is not a question.
#[test]
fn init_without_a_terminal_writes_nothing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = init_without_yes(repo.path(), home.path())?;
    assert!(!refused.is_success);
    assert_eq!(
        refused.stderr.trim_end(),
        "cortex: error: init asks \"Measure Cortex's effect: leave one task in ten without its \
         help, to compare with?\" in a terminal, and has none: run it in one, or pass --yes to \
         take the defaults"
    );
    assert!(!repo.path().join(".claude").exists());
    assert!(!repo.path().join(".cortex").exists());
    assert!(!home.path().join(".cortex").exists());
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::fs::write(
        repo.path().join(".cortex").join("config.toml"),
        "holdout.share = 0\n",
    )?;
    std::fs::create_dir(home.path().join(".cortex"))?;
    std::fs::write(
        home.path().join(".cortex").join("config.toml"),
        "upkeep.llm.enabled = false\n",
    )?;
    let answered = init_without_yes(repo.path(), home.path())?;
    assert!(answered.is_success, "{}", answered.stderr);
    assert!(
        repo.path()
            .join(".claude")
            .join("settings.local.json")
            .is_file()
    );
    Ok(())
}

/// A team that shares facts has committed ignores that leave `.cortex/shared/` in: init keeps that
/// answer, asks nothing about it, and adds only its local settings.
#[test]
fn init_keeps_the_team_s_sharing() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    std::fs::write(
        repo.path().join(".gitignore"),
        ".cortex/*\n!.cortex/shared/\n",
    )?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore"))?,
        ".cortex/*\n!.cortex/shared/\n# Cortex\n.claude/settings.local.json\n"
    );
    Ok(())
}

/// A repository's own configuration cannot answer the question on upkeep by a language model,
/// which only the user's may: with the holdout kept there and upkeep turned on there, init still
/// asks about upkeep, and with no terminal refuses naming it.
#[test]
fn init_never_takes_a_repository_s_upkeep_for_an_answer() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::fs::write(
        repo.path().join(".cortex").join("config.toml"),
        "holdout.share = 0.1\n[upkeep.llm]\nenabled = true\n",
    )?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let refused = init_without_yes(repo.path(), home.path())?;
    assert!(!refused.is_success);
    assert!(
        refused
            .stderr
            .contains("init asks \"Allow optional upkeep by a language model"),
        "{}",
        refused.stderr
    );
    Ok(())
}

/// A `.gitignore` the user made read-only is refused before anything is asked or written.
#[test]
fn init_leaves_a_read_only_gitignore() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let gitignore = repo.path().join(".gitignore");
    std::fs::write(&gitignore, "target/\n")?;
    let mut permissions = std::fs::metadata(&gitignore)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&gitignore, permissions.clone())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the test gives the file back its write permission so it can be removed"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(&gitignore, permissions)?;
    assert!(!done.is_success);
    assert!(done.stderr.contains("is read-only"), "{}", done.stderr);
    assert!(!repo.path().join(".claude").exists());
    assert!(!repo.path().join(".cortex").exists());
    assert_eq!(std::fs::read_to_string(&gitignore)?, "target/\n");
    Ok(())
}

/// A `.gitignore` that is a link is refused before anything is asked: git reads no ignores through
/// one, so lines written there would never count.
// Making a link to a file takes privileges on Windows; the check under test is the same there.
#[cfg(unix)]
#[test]
fn init_refuses_a_linked_gitignore() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    std::fs::write(repo.path().join("ignores"), "target/\n")?;
    std::os::unix::fs::symlink("ignores", repo.path().join(".gitignore"))?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(!done.is_success);
    assert!(
        done.stderr
            .contains("is a link, which git does not read through"),
        "{}",
        done.stderr
    );
    assert!(!repo.path().join(".claude").exists());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("ignores"))?,
        "target/\n"
    );
    Ok(())
}

/// A configuration file that is a link leading outside the repository is refused before anything
/// is asked, and the file it leads to is left as it was.
// Making a link to a file takes privileges on Windows; the check under test is the same there.
#[cfg(unix)]
#[test]
fn init_refuses_a_file_that_leads_outside() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let target = elsewhere.path().join("config.toml");
    std::fs::write(&target, "")?;
    std::fs::create_dir(repo.path().join(".cortex"))?;
    std::os::unix::fs::symlink(&target, repo.path().join(".cortex").join("config.toml"))?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let done = init(repo.path(), home.path(), &[])?;
    assert!(!done.is_success);
    assert!(
        done.stderr.contains("leads outside the repository"),
        "{}",
        done.stderr
    );
    assert!(!repo.path().join(".claude").exists());
    assert_eq!(std::fs::read_to_string(&target)?, "");
    Ok(())
}

/// With no settings file, one is made, and the manifest records that there was none.
#[test]
fn init_creates_the_settings_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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
    assert_eq!(manifest.pointer("/files/1/backup"), Some(&Value::Null));
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
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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

/// The user's scope writes `~/.claude/settings.json` and backs it up in `~/.cortex/backups`; the
/// repository is set up all the same: its answer in its `.cortex/config.toml` with a backup of its
/// own, git leaving out `.cortex/`, and no `.claude` of its own.
#[test]
fn init_for_the_user_writes_the_home_settings() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    git(repo.path(), &["init", "-q"], home.path())?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".cortex").join("config.toml"))?,
        REPOSITORY_ANSWER
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore"))?,
        "# Cortex\n.cortex/\n"
    );
    assert_eq!(backup_folders(repo.path())?.len(), 1);
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
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let run = init(repo.path(), home.path(), &[])?;
    assert!(!run.is_success);
    assert!(run.stderr.contains("is read-only"), "{}", run.stderr);
    assert_eq!(std::fs::read_to_string(&settings)?, "{}");
    assert!(backup_folders(repo.path())?.is_empty());
    Ok(())
}

/// A repository whose `.claude` or `.cortex` folder is a link is refused, whether it leads to a
/// folder outside it or to one beside it inside (here `docs`, which git would commit), and the
/// folder it leads to stays empty: nothing is written through it.
#[cfg(unix)]
#[test]
fn init_refuses_folders_that_lead_outside() -> TestResult {
    for name in [".claude", ".cortex"] {
        let repo = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        let elsewhere = tempfile::tempdir()?;
        let docs = repo.path().join("docs");
        std::fs::create_dir(&docs)?;
        for target in [elsewhere.path(), docs.as_path()] {
            let link = repo.path().join(name);
            std::os::unix::fs::symlink(target, &link)?;
            let _daemon = StopsDaemon::of(repo.path(), home.path());
            let run = init(repo.path(), home.path(), &[])?;
            assert!(!run.is_success, "{name}");
            assert!(run.stderr.contains("leads elsewhere"), "{}", run.stderr);
            assert_eq!(std::fs::read_dir(target)?.count(), 0, "{name}");
            std::fs::remove_file(&link)?;
        }
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
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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
    let _daemon = StopsDaemon::of(repo.path(), home.path());
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
