//! `cortex doctor` on temporary repositories with an empty home folder: green once `cortex init`
//! wrote the hooks and a daemon runs; each check failing once, with its fix, and the command
//! failing then.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, TestResult, isolate};
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

/// Runs `cortex <command> --root <repo>` with `home` as the home folder.
fn cortex(command: &str, repo: &Path, home: &Path) -> TestResult<Run> {
    let mut process = Command::new(env!("CARGO_BIN_EXE_cortex"));
    process.args([command, "--root"]).arg(repo);
    isolate(&mut process, home);
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
/// repository's local settings, nothing turns hooks off, they run this binary, and the daemon of
/// this build answers. What doctor cannot see is said last.
#[test]
fn doctor_is_green_after_init_with_a_daemon() -> TestResult {
    let (repo, home) = repository()?;
    assert!(cortex("init", repo.path(), home.path())?.is_success);
    let daemon = Daemon::start(repo.path(), home.path())?;
    let ready = daemon.first_line()?;
    assert!(ready.starts_with("cortex: daemon ready"), "{ready}");
    let run = cortex("doctor", repo.path(), home.path())?;
    assert!(run.is_success, "{:?}\n{}", run.lines, run.stderr);
    let program = env!("CARGO_BIN_EXE_cortex");
    let names: Vec<&str> = run
        .lines
        .iter()
        .filter_map(|line| line.split_once(':').map(|(head, _)| head))
        .collect();
    assert_eq!(
        names,
        [
            "ok    hooks",
            "ok    silenced",
            "ok    binary",
            "ok    daemon"
        ]
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
/// binary check has nothing to look at, and the command fails naming how many checks did.
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
    assert!(
        run.stderr.contains("2 of the checks failed"),
        "{}",
        run.stderr
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
