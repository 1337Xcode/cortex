//! Running `cortex init`, and other commands, the way the CLI's tests do: an empty home folder of
//! the test's own, a PATH of its own holding git and a stand-in for Claude Code, git looking for no
//! repository above the system's temporary folder, and the output read through pipes.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Printed, TestResult, isolate, printed, stand_in_claude, test_path};

/// The stand-in for Claude Code [`init`] puts in `home`'s `bin` folder when the test put none.
pub fn claude_in(home: &Path) -> PathBuf {
    home.join("bin").join(if cfg!(windows) {
        "claude.cmd"
    } else {
        "claude"
    })
}

/// Runs `cortex init --yes --root <repo>` with `extra` arguments, as [`run`] runs it with git on
/// the PATH, and with a stand-in for Claude Code 2.1.312, the newest version Cortex knows, ahead of
/// any on the machine unless the test put one in `home`'s `bin` folder.
pub fn init(repo: &Path, home: &Path, extra: &[&str]) -> TestResult<Printed> {
    let mut command = init_command(repo, home)?;
    command.arg("--yes").args(extra);
    run(&mut command, home, &["git"])
}

/// Runs `cortex init --root <repo>` as [`init`] does, but without `--yes`, its stdin a pipe that
/// is no terminal.
pub fn init_without_yes(repo: &Path, home: &Path) -> TestResult<Printed> {
    let mut command = init_command(repo, home)?;
    run(&mut command, home, &["git"])
}

/// `cortex init --root <repo>`, with the stand-in for Claude Code [`init`] describes.
fn init_command(repo: &Path, home: &Path) -> TestResult<Command> {
    if !claude_in(home).exists() {
        stand_in_claude(home, "2.1.312")?;
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["init", "--root"]).arg(repo);
    Ok(command)
}

/// Runs `command` with `home` as the home folder and a PATH holding `home`'s `bin` folder and the
/// folders of `programs`, git looking for no repository above the system's temporary folder,
/// where the tests' folders are, and reads what it printed. Its stdin is an empty pipe.
pub fn run(command: &mut Command, home: &Path, programs: &[&str]) -> TestResult<Printed> {
    isolate(command, home);
    command
        .env("PATH", test_path(home, programs)?)
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir());
    printed(command, b"")
}

/// Runs git with `args` in `folder`, with `home` as the home folder.
pub fn git(folder: &Path, args: &[&str], home: &Path) -> TestResult {
    let mut command = Command::new("git");
    command.args(args).current_dir(folder);
    isolate(&mut command, home);
    let output = command.output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(())
}
