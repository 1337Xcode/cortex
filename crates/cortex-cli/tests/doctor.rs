//! `cortex doctor` on temporary repositories with an empty home folder and a PATH of the test's
//! own, holding a stand-in for Claude Code or nothing: green once `cortex init` wrote the hooks
//! and a daemon runs; each check failing once, with its fix, and the command failing then.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, TestResult, isolate, stand_in_claude, test_path};
use cortex_core::ipc::CORTEX_VERSION;

/// What one run printed and how it ended.
struct Run {
    /// Whether it succeeded.
    is_success: bool,
    /// Its stdout's lines.
    lines: Vec<String>,
    /// Its stderr.
    stderr: String,
}

/// Runs `cortex <command> --root <repo>` with `home` as the home folder and a PATH holding only
/// `home`'s `bin` folder (and on Windows the system's own folder, where the command shell is).
fn cortex(command: &str, repo: &Path, home: &Path) -> TestResult<Run> {
    let mut process = Command::new(env!("CARGO_BIN_EXE_cortex"));
    process.args([command, "--root"]).arg(repo);
    isolate(&mut process, home);
    process.env("PATH", test_path(home, &[])?);
    let output = process.output()?;
    Ok(Run {
        is_success: output.status.success(),
        lines: String::from_utf8(output.stdout)?
            .lines()
            .map(str::to_owned)
            .collect(),
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// The lines of `run` that begin with `prefix`.
fn starting(run: &Run, prefix: &str) -> Vec<String> {
    run.lines
        .iter()
        .filter(|line| line.starts_with(prefix))
        .cloned()
        .collect()
}

/// A repository holding one Python file, and an empty home folder.
fn repository() -> TestResult<(tempfile::TempDir, tempfile::TempDir)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def kept():\n    pass\n")?;
    Ok((repo, home))
}

/// After `cortex init` and with the daemon up, every check passes: the hooks are current in the
/// repository's local settings, nothing turns hooks off, they run this binary, the daemon of this
/// build answers, and twenty probes of each of five events reach it, each event's 95th percentile
/// printed against its budget (over it on a loaded machine is a failure, and the command's result
/// follows). With no Claude Code on the PATH, its version is a note. What doctor cannot see is
/// said last.
#[test]
fn doctor_is_green_after_init_with_a_daemon() -> TestResult {
    let (repo, home) = repository()?;
    assert!(cortex("init", repo.path(), home.path())?.is_success);
    let daemon = Daemon::start(repo.path(), home.path())?;
    let ready = daemon.first_line()?;
    assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
    let run = cortex("doctor", repo.path(), home.path())?;
    let program = env!("CARGO_BIN_EXE_cortex");
    let names: Vec<&str> = run
        .lines
        .iter()
        .filter(|line| !line.starts_with(' ') && !line.contains("latency: "))
        .filter_map(|line| line.split_once(':').map(|(head, _)| head))
        .collect();
    assert_eq!(
        names,
        [
            "ok    hooks",
            "ok    silenced",
            "ok    binary",
            "note  agent",
            "ok    daemon"
        ]
    );
    let latency: Vec<(&str, &str)> = run
        .lines
        .iter()
        .filter_map(|line| line.split_once("latency: "))
        .filter_map(|(head, rest)| {
            rest.split_once(": p95 ")
                .map(|(what, _)| (head.trim_end(), what))
        })
        .collect();
    let probed: Vec<&str> = latency.iter().map(|(_, what)| *what).collect();
    assert_eq!(
        probed,
        [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse on a read",
            "PostToolUse on an edit",
            "Stop"
        ],
        "{:?}",
        run.lines
    );
    assert!(
        run.lines
            .iter()
            .all(|line| !line.contains("without reaching the daemon")),
        "{:?}",
        run.lines
    );
    let is_any_over = latency.iter().any(|(head, _)| *head == "FAIL");
    assert_eq!(
        run.is_success, !is_any_over,
        "{:?}\n{}",
        run.lines, run.stderr
    );
    assert_eq!(
        starting(&run, "note  agent"),
        ["note  agent: Claude Code is not on the PATH, so its version cannot be asked"]
    );
    assert_eq!(
        run.lines.last().map(String::as_str),
        Some(
            "note  doctor cannot see whether Claude Code runs with --safe-mode, with --settings \
             that turn hooks off, under settings an organization delivers by its server, MDM or \
             registry, or in a folder it does not trust; each of these keeps every hook from \
             running"
        )
    );
    let binary = starting(&run, "ok    binary: the hooks run this binary, ");
    let [line] = binary.as_slice() else {
        return Err(format!("{:?}", run.lines).into());
    };
    let (path, version) = line
        .trim_start_matches("ok    binary: the hooks run this binary, ")
        .rsplit_once(", Cortex ")
        .ok_or("a binary line")?;
    assert_eq!(
        std::fs::canonicalize(path)?,
        std::fs::canonicalize(program)?
    );
    assert_eq!(version, CORTEX_VERSION);
    let expected = format!(
        "ok    daemon: running (process {}, Cortex {CORTEX_VERSION})",
        daemon.child.id()
    );
    assert_eq!(starting(&run, "ok    daemon"), [expected]);
    Ok(())
}

/// With no hooks and no daemon, the hooks and the daemon checks fail, each with its fix, the
/// binary check has nothing to look at, the latency check measures nothing (a probe would find no
/// daemon and start none), and the command fails naming how many checks did.
#[test]
fn doctor_fails_without_hooks_or_daemon() -> TestResult {
    let (repo, home) = repository()?;
    let run = cortex("doctor", repo.path(), home.path())?;
    assert!(!run.is_success);
    let root = std::fs::canonicalize(repo.path())?;
    let root = cortex_core::canonical_root(&root)?;
    assert_eq!(
        starting(&run, "FAIL"),
        [
            "FAIL  hooks: Claude Code has no Cortex hook in this repository's settings or the user's",
            "FAIL  daemon: not running"
        ]
    );
    assert!(
        run.lines.iter().any(|line| line
            == &format!(
                "      fix: start it with `cortex daemon --root {}`, or open Claude Code there: \
                 its first hook starts one",
                root.display()
            )),
        "{:?}",
        run.lines
    );
    assert_eq!(
        starting(&run, "note  binary"),
        ["note  binary: no Cortex hook names a binary to check"]
    );
    assert_eq!(
        starting(&run, "note  latency"),
        ["note  latency: not measured: no daemon answers, and probes start none"]
    );
    assert!(
        run.stderr.contains("2 of the checks failed"),
        "{}",
        run.stderr
    );
    Ok(())
}

/// A Claude Code older than the oldest version Cortex's hooks work with fails the agent check,
/// naming where it is, with an update as the fix: a stand-in on the PATH answers `--version` as
/// 2.1.200 does.
#[test]
fn doctor_fails_on_an_older_claude_code() -> TestResult {
    let (repo, home) = repository()?;
    let claude = stand_in_claude(home.path(), "2.1.200")?;
    let run = cortex("doctor", repo.path(), home.path())?;
    assert!(!run.is_success);
    let agent: Vec<String> = run
        .lines
        .iter()
        .skip_while(|line| !line.starts_with("FAIL  agent"))
        .take(2)
        .cloned()
        .collect();
    assert_eq!(
        agent,
        [
            format!(
                "FAIL  agent: Claude Code 2.1.200 at {} is older than 2.1.295, the oldest \
                 Cortex's hooks work with",
                claude.display()
            ),
            "      fix: update Claude Code to 2.1.295 or later".to_owned()
        ],
        "{:?}",
        run.lines
    );
    Ok(())
}

/// `disableAllHooks` in the user's settings fails the silenced check, naming the file; hooks that
/// run a binary that is gone fail the hooks check (not what this build writes) and the binary
/// check (it does not exist).
#[test]
fn doctor_fails_on_silenced_and_stale_hooks() -> TestResult {
    let (repo, home) = repository()?;
    assert!(cortex("init", repo.path(), home.path())?.is_success);
    let user_settings = home.path().join(".claude").join("settings.json");
    std::fs::create_dir_all(home.path().join(".claude"))?;
    std::fs::write(&user_settings, r#"{"disableAllHooks": true}"#)?;
    let settings = repo.path().join(".claude").join("settings.local.json");
    let text = std::fs::read_to_string(&settings)?;
    let program = serde_json::to_string(env!("CARGO_BIN_EXE_cortex"))?;
    let gone = serde_json::to_string(&repo.path().join("gone").join("cortex"))?;
    std::fs::write(&settings, text.replace(&program, &gone))?;
    let run = cortex("doctor", repo.path(), home.path())?;
    assert!(!run.is_success);
    let failed: Vec<String> = starting(&run, "FAIL")
        .into_iter()
        .map(|line| {
            line.split_once(':')
                .map(|(head, _)| head.to_owned())
                .unwrap_or(line)
        })
        .collect();
    assert_eq!(
        failed,
        [
            "FAIL  hooks",
            "FAIL  silenced",
            "FAIL  binary",
            "FAIL  daemon"
        ]
    );
    let silenced = starting(&run, "FAIL  silenced");
    let expected = format!(
        "FAIL  silenced: disableAllHooks is true in {}, so Claude Code runs no hook",
        user_settings.display()
    );
    assert_eq!(silenced, [expected]);
    let binary = starting(&run, "FAIL  binary");
    assert!(
        binary
            .iter()
            .any(|line| line.ends_with("which does not exist")),
        "{binary:?}"
    );
    Ok(())
}
