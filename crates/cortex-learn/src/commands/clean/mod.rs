//! Cleaning the command a handoff keeps of its last check, so the next session can see what was
//! run without the line's secrets. A command line can carry what must never be stored: values set
//! in the environment, tokens, passwords, URLs with credentials in them. So a command is kept only
//! when its check is one of the runners Cortex knows, and then only the runner's own words: its
//! program as the shell passed it and the arguments after it. The variable assignments before it
//! and the programs that only run another (`env A=1`, `env -i`, `timeout 60`, `uv run`,
//! `python -m`) are left out, and so is every other command of the line, a `$env:NAME='...'`
//! prefix of Windows' shell among them. Each argument a credential detector matches becomes
//! [`REDACTED`], in part or whole; the detectors are a table in `detectors`.
//!
//! When anything about the command is uncertain, only the runner's name and the test file it
//! named are kept, or the name alone: the check runs inside a group (`(cd x && pytest)`) or reads
//! from a pipe (`cat ids | pytest`); an argument is one the shell expands (`$VAR`, a command
//! substitution, a glob); a word holds a control, bidirectional or invisible character; a
//! detector finds a credential whose value it cannot bound; or the command does not fit in
//! `render::MAX_KEPT_CHARS` even cut after its program. A line the shell reader
//! refuses tells no check at all, so nothing is kept of it, and neither is anything of a line
//! whose check is not a runner's. Redirections are dropped by the reader with their targets, so
//! they are never kept.
//!
//! What is kept is a command, a path or a name. Nothing here keeps a check's output.

mod bearer;
mod detectors;
mod entropy;
mod flags;
mod jwt;
mod keys;
mod names;
mod prefixes;
mod redact;
mod render;
mod test_path;
#[cfg(test)]
mod tests;
mod urls;

use std::fmt;
use std::path::Path;

use cortex_core::ShellCommand;

use super::shell::{self, Word};
use super::target::{self, Located};
use super::{CONFIGURED, Check, Commands, argv};

/// What stands in a kept command for each secret a detector found.
pub const REDACTED: &str = "<redacted>";

/// What a handoff keeps of its last check's command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleaned {
    /// The runner's program and its arguments, joined by single spaces, each as the shell passed
    /// it, quoted when it holds anything but letters, digits and `_-./:=@%+,`, every secret in it
    /// [`REDACTED`], cut at an argument to at most 200 characters, ending ` ...`, when it is
    /// longer.
    Command(String),
    /// The runner's name and the test file the command named, a test id after `::` taken off.
    TestPath {
        /// The runner, such as `pytest`.
        runner: &'static str,
        /// The test file, relative, as the command wrote it.
        path: String,
    },
    /// The runner's name alone.
    Runner(&'static str),
}

impl fmt::Display for Cleaned {
    /// Writes the stored form: the command, the runner and the test file with a space between,
    /// or the runner.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(text) => formatter.write_str(text),
            Self::TestPath { runner, path } => write!(formatter, "{runner} {path}"),
            Self::Runner(runner) => formatter.write_str(runner),
        }
    }
}

/// Uncertainty about a command, which keeps only its runner's name and test file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unsure;

/// What a handoff keeps of `command`, run in the folder `cwd` of the repository whose top is
/// `root`: the cleaned command of the line's failing check, or of its only check when the line
/// does not exit as one check failing. `None` when the line ran no such check of a runner Cortex
/// knows or of the user's patterns; the caller then keeps only the exit code.
///
/// A check of the user's patterns is never kept as a command: the pattern tells the line apart
/// but not which of its words are the check's arguments, so only its test file is kept.
#[must_use]
pub fn clean_command(
    commands: &Commands,
    command: &ShellCommand,
    root: &Path,
    cwd: &Path,
) -> Option<Cleaned> {
    let told = commands.tell(command, root, cwd);
    let check = told.failure.or(match told.checks.as_slice() {
        [only] => Some(*only),
        _ => None,
    })?;
    let Ok(list) = shell::read(command.as_str()) else {
        return Some(Cleaned::Runner(check.runner));
    };
    Some(match target::locate(&list, check, told.failure.is_some()) {
        Some(located) => clean_located(check, &located),
        None => Cleaned::Runner(check.runner),
    })
}

/// What is kept of the simple command `located`, which ran `check`.
fn clean_located(check: Check, located: &Located<'_>) -> Cleaned {
    let is_sure = check.runner != CONFIGURED && !located.is_grouped && !located.is_piped_into;
    let kept = if is_sure {
        runner_words(located.words)
            .and_then(|words| redact::redact(&words))
            .and_then(|words| render::command(&words))
    } else {
        Err(Unsure)
    };
    kept.map_or_else(
        |Unsure| {
            let words = argv::after_assignments(located.words);
            fallback(check.runner, words.get(1..).unwrap_or_default())
        },
        Cleaned::Command,
    )
}

/// The runner's own words in the simple command `words`: its program and the arguments after it,
/// past the assignments and the programs that only run another. The program is kept by its name
/// alone (`/home/dev/.venv/bin/pytest` is `pytest`), so that no folder, and no user name in one,
/// is kept; a module Python runs keeps `python -m` before it, so that the command still runs.
fn runner_words(words: &[Word]) -> Result<Vec<Word>, Unsure> {
    let invocation = argv::invocation(words).ok_or(Unsure)?;
    // The arguments are always the words' tail, so the program is the word just before them.
    let program_at = words
        .len()
        .checked_sub(invocation.args.len() + 1)
        .ok_or(Unsure)?;
    let program = words.get(program_at).ok_or(Unsure)?;
    let named = |text: String| Word {
        text,
        is_literal: true,
    };
    let mut kept = Vec::with_capacity(invocation.args.len() + 3);
    let is_module = program_at >= 2
        && words.get(program_at - 1).and_then(Word::literal) == Some("-m")
        && words
            .get(..program_at - 1)
            .unwrap_or_default()
            .iter()
            .any(|word| {
                word.literal()
                    .is_some_and(|text| argv::is_python(&argv::program_name(text)))
            });
    if is_module {
        kept.push(named("python".to_owned()));
        kept.push(named("-m".to_owned()));
    }
    kept.push(named(argv::program_name(program.literal().ok_or(Unsure)?)));
    kept.extend(
        words
            .get(program_at + 1..)
            .unwrap_or_default()
            .iter()
            .cloned(),
    );
    Ok(kept)
}

/// What is kept when the command cannot be: `runner` and the first of `args` that is a test file,
/// or `runner` alone.
fn fallback(runner: &'static str, args: &[Word]) -> Cleaned {
    match args.iter().find_map(test_path::test_path) {
        Some(path) => Cleaned::TestPath { runner, path },
        None => Cleaned::Runner(runner),
    }
}
