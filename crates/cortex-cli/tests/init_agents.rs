//! `cortex init --agents` on temporary repositories with an empty home folder and a PATH of the
//! test's own: Codex's hooks in the user's `~/.codex/hooks.json`, each a command line, with what
//! the user must do themselves said; an agent Cortex does not set up refused before anything is
//! written; and `cortex uninstall --scope user` taking the file init made away again: the user's
//! hooks serve every project, so uninstalling one repository leaves them.

mod common;

use std::process::Command;

use common::init::run;
use common::{StopsDaemon, TestResult};
use serde_json::{Value, json};

/// Codex's hooks, from its adapter's table by hand: each event with its matcher, the command line
/// running this binary, and the hard timeout in whole seconds rounded up (1,500 ms is 2 s, every
/// other 1 s); in the user's file, since a repository has no Codex file git leaves out. Codex
/// itself, not on the PATH, is neither asked nor checked; the trust step is said.
#[test]
fn init_sets_up_codex_in_the_user_s_hooks_file() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "codex", "--root"])
        .arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let program = env!("CARGO_BIN_EXE_cortex");
    // The premise of the expected lines: the test binary's path needs no quoting.
    assert!(!program.contains(' '), "{program}");
    let program = program.replace('\\', "/");
    let group = |event: &str, matcher: Option<&str>, timeout: u64| {
        let handler = json!({
            "type": "command",
            "command": format!("{program} hook --agent codex --event {event}"),
            "timeout": timeout,
        });
        match matcher {
            Some(matcher) => json!([{"matcher": matcher, "hooks": [handler]}]),
            None => json!([{"hooks": [handler]}]),
        }
    };
    let expected = json!({"hooks": {
        "SessionStart": group("SessionStart", None, 2),
        "UserPromptSubmit": group("UserPromptSubmit", None, 1),
        "PreToolUse": group("PreToolUse", Some("^Bash$"), 1),
        "PostToolUse": group("PostToolUse", Some("^(Bash|apply_patch)$"), 1),
        "PreCompact": group("PreCompact", None, 1),
        "Stop": group("Stop", None, 1),
        "SubagentStart": group("SubagentStart", None, 1),
        "SubagentStop": group("SubagentStop", None, 1),
        "SessionEnd": group("SessionEnd", None, 1),
    }});
    let file = home.path().join(".codex").join("hooks.json");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    assert_eq!(written, expected);
    assert!(!repo.path().join(".claude").exists());
    assert!(done.stdout.lines().any(|line| line
        == "Codex: to do yourself: trust Cortex's hooks in Codex's `/hooks` (Codex runs no hook \
            it has not been told to trust)"));
    let written = done
        .stdout
        .lines()
        .find(|line| line.starts_with("Codex: hooks written to "))
        .ok_or_else(|| done.stdout.clone())?;
    assert!(
        written.ends_with(
            "hooks.json, which serve every repository; `cortex uninstall --scope user` takes \
             them out"
        ),
        "{written}"
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["uninstall", "--root"]).arg(repo.path());
    let undone = run(&mut command, home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(file.exists());
    let note = format!(
        "note: {} holds Cortex's Codex hooks, which run in every repository; `cortex uninstall \
         --scope user` takes them out\n",
        file.display()
    );
    assert!(undone.stdout.ends_with(&note), "{}", undone.stdout);
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["uninstall", "--scope", "user", "--root"])
        .arg(repo.path());
    let undone = run(&mut command, home.path(), &[])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(!file.exists());
    Ok(())
}

/// An agent Cortex does not set up is refused, naming those it does, before anything is written.
#[test]
fn init_refuses_an_agent_it_does_not_set_up() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "codex,nope", "--root"])
        .arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(!done.is_success);
    assert!(
        done.stderr
            .contains("`nope` is not an agent Cortex sets up; it sets up claude, codex"),
        "{}",
        done.stderr
    );
    assert!(!home.path().join(".codex").exists());
    assert!(!repo.path().join(".cortex").exists());
    Ok(())
}

/// Each agent whose settings take Claude Code's nested hooks, in its own file, typed by hand from
/// its documentation: the user's file for an agent whose repository settings git shares, the
/// repository's local file otherwise, which git is then told to leave out; the events under
/// `hooks` or at the top of the file; timeouts in seconds rounded up or in milliseconds. Every
/// event of the agent's adapter gets its matcher group and a command line running this binary.
/// Uninstall, in the scope the file is in, takes the file init made away again.
#[test]
fn init_sets_up_each_agent_with_nested_hooks() -> TestResult {
    // (agent, the file under the home folder or the repository, whether at the top of the file,
    // whether in milliseconds)
    let agents = [
        ("qwen", Place::Home(".qwen/settings.json"), false, false),
        ("gemini", Place::Home(".gemini/settings.json"), false, true),
        ("droid", Place::Home(".factory/hooks.json"), true, false),
        (
            "devin",
            Place::Repository(".devin/config.local.json"),
            false,
            false,
        ),
        (
            "augment",
            Place::Repository(".augment/settings.local.json"),
            false,
            true,
        ),
        ("junie", Place::Home(".junie/config.json"), false, false),
    ];
    let program = env!("CARGO_BIN_EXE_cortex");
    assert!(!program.contains(' '), "{program}");
    let program = program.replace('\\', "/");
    for (agent, place, is_whole_file, is_millis) in agents {
        let repo = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        common::init::git(repo.path(), &["init", "-q"], home.path())?;
        let _daemon = StopsDaemon::of(repo.path(), home.path());
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(["init", "--yes", "--agents", agent, "--root"])
            .arg(repo.path());
        let done = run(&mut command, home.path(), &["git"])?;
        assert!(done.is_success, "{agent}: {}", done.stderr);
        let adapter = cortex_hook::adapter(agent).ok_or(agent)?;
        let mut events = serde_json::Map::new();
        for entry in adapter.hooks() {
            let millis = u64::try_from(entry.hard_timeout.as_millis())?;
            let timeout = if is_millis {
                millis
            } else {
                millis.div_ceil(1_000)
            };
            let handler = json!({
                "type": "command",
                "command": format!("{program} hook --agent {agent} --event {}", entry.event_name),
                "timeout": timeout,
            });
            let group = match entry.matcher {
                Some(matcher) => json!({"matcher": matcher, "hooks": [handler]}),
                None => json!({"hooks": [handler]}),
            };
            events.insert(entry.event_name.to_owned(), json!([group]));
        }
        let expected = if is_whole_file {
            Value::Object(events)
        } else {
            json!({ "hooks": events })
        };
        let (file, scope) = match place {
            Place::Home(path) => (home.path().join(path), "user"),
            Place::Repository(path) => (repo.path().join(path), "project"),
        };
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        assert_eq!(written, expected, "{agent}");
        if let Place::Repository(path) = place {
            let ignores = std::fs::read_to_string(repo.path().join(".gitignore"))?;
            assert!(
                ignores.lines().any(|line| line == path),
                "{agent}: {ignores}"
            );
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(["uninstall", "--scope", scope, "--root"])
            .arg(repo.path());
        let undone = run(&mut command, home.path(), &["git"])?;
        assert!(undone.is_success, "{agent}: {}", undone.stderr);
        assert!(!file.exists(), "{agent}");
    }
    Ok(())
}

/// Where a test expects an agent's hooks file.
#[derive(Clone, Copy)]
enum Place {
    /// Under the home folder.
    Home(&'static str),
    /// Under the repository's root.
    Repository(&'static str),
}

/// Codex is found by its folder in the home folder, which holds its configuration, and Claude Code
/// neither by its folder (empty, as one Cortex made) nor on the PATH: init, naming no agent, sets
/// up Codex alone, writing nothing of Claude Code's.
#[test]
fn init_sets_up_only_the_agents_it_finds() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir(home.path().join(".codex"))?;
    std::fs::write(home.path().join(".codex").join("config.toml"), "")?;
    std::fs::create_dir(home.path().join(".claude"))?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--yes", "--root"]).arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(home.path().join(".codex").join("hooks.json").is_file());
    assert!(!repo.path().join(".claude").exists());
    assert_eq!(std::fs::read_dir(home.path().join(".claude"))?.count(), 0);
    let written: Vec<String> = std::fs::read_dir(home.path())?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    let mut written = written;
    written.sort_unstable();
    assert_eq!(written, [".claude", ".codex", ".cortex"]);
    Ok(())
}

/// An agent named twice is set up once; Codex, found by its folder with a hooks file that is not
/// JSON, is left out with a note while Qwen Code, found by its folder, is set up, and the hooks
/// file is left as it was; named, Codex with that file stops init.
#[test]
fn init_leaves_out_a_found_agent_it_cannot_set_up() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "codex,codex", "--root"])
        .arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let codex_hooks = home.path().join(".codex").join("hooks.json");
    std::fs::write(&codex_hooks, "not json")?;
    std::fs::write(home.path().join(".codex").join("config.toml"), "")?;
    std::fs::create_dir(home.path().join(".qwen"))?;
    std::fs::write(home.path().join(".qwen").join("installation_id"), "x")?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--yes", "--root"]).arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    let note = done
        .stdout
        .lines()
        .find(|line| line.starts_with("note: Codex was found but is not set up: "))
        .ok_or_else(|| done.stdout.clone())?;
    assert!(
        note.ends_with("; `cortex init --agents codex` sets it up once that is fixed"),
        "{note}"
    );
    assert_eq!(std::fs::read_to_string(&codex_hooks)?, "not json");
    assert!(home.path().join(".qwen").join("settings.json").is_file());
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "codex", "--root"])
        .arg(repo.path());
    let refused = run(&mut command, home.path(), &[])?;
    assert!(!refused.is_success);
    assert_eq!(std::fs::read_to_string(&codex_hooks)?, "not json");
    Ok(())
}

/// Droid reads hooks from `~/.factory/settings.json` while `~/.factory/hooks.json` is absent: with
/// hooks kept there, init asked for Droid refuses, naming the file, and makes no `hooks.json`,
/// which would hide them; the settings stay as they were. Without hooks there it is set up.
#[test]
fn init_does_not_hide_droid_s_hooks_in_its_settings() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let _daemon = StopsDaemon::of(repo.path(), home.path());
    let factory = home.path().join(".factory");
    std::fs::create_dir(&factory)?;
    let settings = factory.join("settings.json");
    let kept = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify"}]}]}}"#;
    std::fs::write(&settings, kept)?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "droid", "--root"])
        .arg(repo.path());
    let refused = run(&mut command, home.path(), &[])?;
    assert!(!refused.is_success);
    assert!(
        refused.stderr.contains(
            "settings.json holds hooks, which Droid reads only while it has no hooks file of its \
             own: Cortex's would hide them; move them into that file first"
        ),
        "{}",
        refused.stderr
    );
    assert!(!factory.join("hooks.json").exists());
    assert_eq!(std::fs::read_to_string(&settings)?, kept);
    std::fs::write(&settings, r#"{"theme": "dark"}"#)?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(["init", "--yes", "--agents", "droid", "--root"])
        .arg(repo.path());
    let done = run(&mut command, home.path(), &[])?;
    assert!(done.is_success, "{}", done.stderr);
    assert!(factory.join("hooks.json").is_file());
    assert_eq!(std::fs::read_to_string(&settings)?, r#"{"theme": "dark"}"#);
    Ok(())
}
