//! Running `cortex init`, and other commands, the way the CLI's tests do: an empty home folder of
//! the test's own, a PATH of its own holding git and a stand-in for Claude Code, git looking for no
//! repository above the system's temporary folder, and the output read through pipes.

use std::collections::BTreeMap;
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
    run_with(command, home, programs, &[])
}

/// Runs `command` as [`run`] does, with the variables `env` set after the home folder's own.
pub fn run_with(
    command: &mut Command,
    home: &Path,
    programs: &[&str],
    env: &[(&str, &Path)],
) -> TestResult<Printed> {
    isolate(command, home);
    for (name, value) in env {
        command.env(name, value);
    }
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

/// Runs `cortex <args> --root <repo>` with `home` as the home folder and git on the PATH.
pub fn cortex(args: &[&str], repo: &Path, home: &Path) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).arg("--root").arg(repo);
    run(&mut command, home, &["git"])
}

/// The lines doctor's agents check printed, its fixes with them.
pub fn agents_lines(doctor: &Printed) -> Vec<String> {
    let lines: Vec<&str> = doctor.stdout.lines().collect();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if line.contains("  agents: ") {
            found.push((*line).to_owned());
            if let Some(fix) = lines
                .get(at + 1)
                .filter(|next| next.starts_with("      fix: "))
            {
                found.push((*fix).to_owned());
            }
        }
    }
    found
}

/// Every file and folder under `folders` but git's and Cortex's own (`.git`, `.cortex`), each with
/// its bytes (none for a folder), by path.
pub fn snapshot(folders: &[&Path]) -> TestResult<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    let mut found = BTreeMap::new();
    let mut pending: Vec<PathBuf> = folders.iter().map(|folder| folder.to_path_buf()).collect();
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let path = entry.path();
            if matches!(entry.file_name().to_str(), Some(".git" | ".cortex")) {
                continue;
            }
            if entry.file_type()?.is_dir() {
                found.insert(path.clone(), None);
                pending.push(path);
            } else {
                found.insert(path.clone(), Some(std::fs::read(&path)?));
            }
        }
    }
    Ok(found)
}
