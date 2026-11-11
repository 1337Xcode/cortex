//! Turning Cortex on and off around adoption, through the real binary: `cortex off` silences a
//! repository and `cortex on` undoes it; `adopt.enabled` is the switch's file, shown and set by
//! `cortex config`; `cortex uninstall --global` gives every agent's file back byte for byte; and
//! `cortex init --global` refuses while a repository holds Cortex's hooks in its own settings, so
//! that no repository runs Cortex from both scopes.

mod common;

use common::adopting::{FAMILIAR, Machine, calc_repository, git_status, words_of};
use common::calc::{captured, prompt};
use common::{StopsDaemon, TestResult};
use cortex_core::{RepoId, canonical_root};
use cortex_daemon::is_running;

/// A user's own hooks, as Claude Code, Codex and Qwen Code read them: one at the stop.
const OWN_HOOKS: &str = "{\n  \"hooks\": {\n    \"Stop\": [\n      {\"hooks\": [{\"type\": \"command\", \"command\": \
     \"echo mine\"}]}\n    ]\n  },\n  \"theme\": \"dark\"\n}\n";

/// With adoption off, a hook from a git repository not set up starts nothing. `cortex off` in a
/// repository set up writes `.cortex/off` and stops its daemon; hooks there then give nothing and
/// start nothing, adoption on or not; `cortex on` takes `off` away. In a git repository never set
/// up, `cortex off` makes a `.cortex` git leaves out, and `cortex on` takes it away again.
#[test]
fn cortex_off_silences_a_repository_and_on_undoes_it() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let before = machine.hook(&captured("session-start-startup.json", &repo)?)?;
    assert!(before.is_silent_success(), "{:?}", before.output);
    assert!(!machine.start_marker(&repo)?.exists());
    let root = repo.to_string_lossy().into_owned();
    let done = machine.cortex(&["init", "--yes", "--root", &root])?;
    assert!(done.is_success, "{}", done.stderr);
    let user = machine.user();
    let id = RepoId::of(&canonical_root(&repo)?);
    assert!(is_running(&user, &id)?);
    std::fs::write(user.adopt_path(), "")?;

    let off = machine.cortex(&["off", "--root", &root])?;
    assert!(off.is_success, "{}", off.stderr);
    let lines: Vec<&str> = off.stdout.lines().collect();
    let said = format!(
        "cortex: off in {}; `cortex on` turns it on again",
        repo.display()
    );
    assert_eq!(lines.first().copied(), Some(said.as_str()));
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with("daemon: stopped")),
        "{}",
        off.stdout
    );
    assert!(repo.join(".cortex").join(cortex_core::OFF_FILE).is_file());
    assert!(!is_running(&user, &id)?);
    let long_ago = machine.age_start_marker(&repo)?;
    for payload in [
        captured("session-start-startup.json", &repo)?,
        prompt(&repo, FAMILIAR)?,
    ] {
        let call = machine.hook(&payload)?;
        assert!(call.is_silent_success(), "{:?}", call.output);
    }
    assert_eq!(
        std::fs::metadata(machine.start_marker(&repo)?)?.modified()?,
        long_ago
    );
    assert!(!is_running(&user, &id)?);

    let on = machine.cortex(&["on", "--root", &root])?;
    assert!(on.is_success, "{}", on.stderr);
    assert_eq!(on.stdout, format!("cortex: on in {}\n", repo.display()));
    assert!(!repo.join(".cortex").join(cortex_core::OFF_FILE).exists());
    assert!(repo.join(".cortex").is_dir());

    let fresh = home.join("code").join("fresh");
    calc_repository(&fresh, home)?;
    let fresh_root = fresh.to_string_lossy().into_owned();
    let off = machine.cortex(&["off", "--root", &fresh_root])?;
    assert!(off.is_success, "{}", off.stderr);
    assert!(fresh.join(".cortex").join(cortex_core::OFF_FILE).is_file());
    assert_eq!(git_status(&fresh, home)?, "");
    let call = machine.hook(&captured("session-start-startup.json", &fresh)?)?;
    assert!(call.is_silent_success(), "{:?}", call.output);
    assert!(!machine.start_marker(&fresh)?.exists());
    let on = machine.cortex(&["on", "--root", &fresh_root])?;
    assert!(on.is_success, "{}", on.stderr);
    assert!(!fresh.join(".cortex").exists());
    Ok(())
}

/// `adopt.enabled` is the switch's file: `cortex config` shows it off by default, `cortex config
/// set adopt.enabled true` writes the file and shows it on from the user, `false` removes it; a
/// value other than true or false is refused and changes nothing.
#[test]
fn adopt_enabled_is_the_switch_s_file() -> TestResult {
    let machine = Machine::new()?;
    let repo = machine.home.join("code").join("calc");
    calc_repository(&repo, &machine.home)?;
    let root = repo.to_string_lossy().into_owned();
    let switch = machine.user().adopt_path();
    let shown = |expected: &str| -> TestResult {
        let listed = machine.cortex(&["config", "--root", &root])?;
        assert!(listed.is_success, "{}", listed.stderr);
        assert!(
            listed.stdout.lines().any(|line| line == expected),
            "{}",
            listed.stdout
        );
        Ok(())
    };
    shown("adopt.enabled = false (default)")?;
    let on = machine.cortex(&["config", "set", "adopt.enabled", "true", "--root", &root])?;
    assert!(on.is_success, "{}", on.stderr);
    assert!(switch.is_file());
    shown("adopt.enabled = true (user)")?;
    let odd = machine.cortex(&["config", "set", "adopt.enabled", "yes", "--root", &root])?;
    assert!(!odd.is_success);
    assert!(
        odd.stderr.contains("`adopt.enabled` takes true or false"),
        "{}",
        odd.stderr
    );
    assert!(switch.is_file());
    let off = machine.cortex(&["config", "set", "adopt.enabled", "false", "--root", &root])?;
    assert!(off.is_success, "{}", off.stderr);
    assert!(!switch.exists());
    shown("adopt.enabled = false (default)")?;
    Ok(())
}

/// With the user's own hooks in Claude Code's, Codex's and Qwen Code's files, `init --global` adds
/// Cortex's beside them and keeps the holdout answer in a user configuration it makes; `uninstall
/// --global` then gives each file back byte for byte, removes the configuration it made, and turns
/// adoption off.
#[test]
fn uninstall_global_gives_every_agent_file_back() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    let files = [
        (home.join(".claude").join("settings.json"), "claude"),
        (home.join(".codex").join("hooks.json"), "codex"),
        (home.join(".qwen").join("settings.json"), "qwen"),
    ];
    for (file, _) in &files {
        std::fs::create_dir_all(file.parent().ok_or("no folder")?)?;
        std::fs::write(file, OWN_HOOKS)?;
    }
    // What makes each agent found besides the file Cortex writes hooks to.
    std::fs::write(home.join(".codex").join("config.toml"), "model = \"o4\"\n")?;
    std::fs::write(home.join(".qwen").join("installation_id"), "x")?;
    let done = machine.cortex(&["init", "--global", "--yes"])?;
    assert!(done.is_success, "{}", done.stderr);
    for (file, agent) in &files {
        let written = std::fs::read_to_string(file)?;
        let words = words_of(&serde_json::from_str(&written)?);
        assert!(words.contains("echo mine"), "{written}");
        assert!(
            words.contains(&format!("hook --agent {agent} --event")),
            "{written}"
        );
    }
    let user = machine.user();
    assert_eq!(
        std::fs::read_to_string(user.config_path())?,
        "# The answer to a question `cortex init` asked; change it here.\n[holdout]\nshare = 0.1\n"
    );
    assert!(user.adopt_path().is_file());

    let undone = machine.cortex(&["uninstall", "--global"])?;
    assert!(undone.is_success, "{}", undone.stderr);
    assert!(
        undone
            .stdout
            .ends_with("adoption: off; Cortex sets up no repository on its own\n"),
        "{}",
        undone.stdout
    );
    for (file, _) in &files {
        assert_eq!(
            std::fs::read(file)?,
            OWN_HOOKS.as_bytes(),
            "{}",
            file.display()
        );
    }
    assert!(!user.config_path().exists());
    assert!(!user.adopt_path().exists());
    Ok(())
}

/// A repository `cortex init` set Claude Code up in, in its own `.claude/settings.local.json`:
/// `init --global` refuses, naming that file and the uninstall that takes the hooks out, and
/// writes nothing (the user's settings are as they were, adoption stays off). Once the
/// repository's hooks are taken out, `init --global` sets Claude Code up for the user. Then
/// `cortex init` in the repository leaves Claude Code out the other way round, as it did before,
/// and says why.
#[test]
fn no_repository_runs_cortex_from_both_scopes() -> TestResult {
    let machine = Machine::new()?;
    let home = machine.home.as_path();
    let repo = home.join("code").join("calc");
    calc_repository(&repo, home)?;
    let _daemon = StopsDaemon::of(&repo, home);
    let root = repo.to_string_lossy().into_owned();
    let user_settings = home.join(".claude").join("settings.json");
    std::fs::create_dir_all(home.join(".claude"))?;
    std::fs::write(&user_settings, OWN_HOOKS)?;
    let project = machine.cortex(&["init", "--yes", "--root", &root])?;
    assert!(project.is_success, "{}", project.stderr);
    let local = repo.join(".claude").join("settings.local.json");
    assert!(local.is_file());

    let global = machine.cortex(&["init", "--global", "--yes"])?;
    assert!(!global.is_success);
    let named = format!(
        "{} holds Claude Code's; `cortex uninstall --root {}` takes them out",
        local.display(),
        repo.display()
    );
    assert!(global.stderr.contains(&named), "{}", global.stderr);
    assert!(
        global
            .stderr
            .contains("each event there would run Cortex twice"),
        "{}",
        global.stderr
    );
    assert_eq!(std::fs::read(&user_settings)?, OWN_HOOKS.as_bytes());
    assert!(!machine.user().adopt_path().exists());

    let undone = machine.cortex(&["uninstall", "--root", &root])?;
    assert!(undone.is_success, "{}", undone.stderr);
    let global = machine.cortex(&["init", "--global", "--yes"])?;
    assert!(global.is_success, "{}", global.stderr);
    let written = std::fs::read_to_string(&user_settings)?;
    assert!(
        words_of(&serde_json::from_str(&written)?).contains("hook --agent claude --event"),
        "{written}"
    );
    let again = machine.cortex(&["init", "--yes", "--root", &root])?;
    assert!(again.is_success, "{}", again.stderr);
    assert!(
        again
            .stdout
            .contains("note: Claude Code was found but is not set up")
            && again.stdout.contains("each event would run Cortex twice"),
        "{}",
        again.stdout
    );
    assert!(!local.exists());
    Ok(())
}
