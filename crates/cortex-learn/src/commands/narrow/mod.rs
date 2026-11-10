//! The command that reruns only some test files, for an agent that just ran a whole test suite.
//! [`whole_suite`] reads a command line and says whether it ran one test check of a known runner
//! with nothing that selects tests, keeping what is needed to write another command the way the
//! agent launched this one; [`narrow_command`] writes that command for given test files. Both are
//! pure: nothing here reads the disk, so what a runner would find there (a `package.json` script,
//! a `Cargo.toml`) is never assumed beyond what the command and the file paths say.
//!
//! What counts as selecting, per runner (checked against the documentation named; the flags a
//! page lists are the whole of what is trusted, and anything not listed makes the run uncertain,
//! which tells nothing):
//!
//! - pytest (9.0 usage and reference pages): a path, a node id (`::`), `-k`, `-m`, `--lf`,
//!   `--sw`, `--deselect`, `--ignore`, `--pyargs`, `--collect-only` and `--doctest-modules` select.
//!   Only the flags listed in `pytest` keep a run whole (`-q`, `-x`, `-v`, `--tb=short`,
//!   `--no-header`, `--color`, `-n N`, `-p plugin`, ...); any other flag or short letter makes it
//!   uncertain.
//! - unittest (Python 3 `unittest` page, from memory, not re-read): a module name, `-k` or
//!   discovery's `-p` select; `discover` and the verbosity flags do not. Discovery's `-s` and `-t`
//!   change the folder test modules are imported from, which a module name run from the line's
//!   folder would not repeat, so they leave the run uncertain.
//! - go (`go help test`, go 1.2x): a package other than `./...` and `-run`, `-skip`, `-bench`,
//!   `-list`, `-fuzz` and `-args` select. `go test` with no package runs the folder's package,
//!   which is taken as whole, as the folder's tests are what the agent asked for.
//! - cargo (The Cargo Book, `cargo test`): a test name filter, a target flag (`--lib`, `--bin`,
//!   `--test`, `--tests`, `--doc`, ...), `--no-run`, `--exclude`, `--manifest-path` and flags
//!   that shape the build (`--features`, `--profile`, `--target`) make the run uncertain.
//!   `--workspace` and `--all` keep it whole, and so does exactly one `-p crate`: that package is
//!   the whole of what was run, and the narrow command names it. `--release` is kept for the
//!   narrow command, so that it does not rebuild the tree.
//! - jest (30 CLI page) and vitest (CLI page): a file filter, `-t`, `--testNamePattern`,
//!   `--changed`, `--shard`, `--project` and the rest of the selecting table make the run
//!   uncertain. A package script (`npm test`) has an unknown runner, so it tells nothing.
//!
//! The launcher stays as the agent wrote it: `python -m pytest`, `uv run pytest`, `npx jest`,
//! `pnpm exec jest`, and so do the run's own options that select nothing (`-x`,
//! `--import-mode=importlib`, `-tags integration`, `-- --test-threads=1`): they shape how the tests
//! are built, imported or run, so the narrow command repeats them, and a narrow run is the whole
//! run with fewer files. Left out of it are the options that name the whole of what ran
//! (`./...`, `discover`, cargo's `--workspace`, `--all` and `--all-targets`) and those that report
//! on the run or measure its coverage (`--junitxml`, `--cov-fail-under`, `-coverprofile`, jest's
//! and vitest's `--coverage`): a narrow run's coverage falls short of a threshold the suite meets,
//! and its report would replace the suite's. Each word is written so that a POSIX shell and
//! PowerShell pass it alike; a word neither can (one holding a single quote) leaves no command.
//!
//! A line that sets a variable not in a short harmless list, wraps the runner
//! in `env` or `timeout`, names it by an absolute path, or holds a word the shell expands, tells
//! nothing, and neither does a line with a group, a pipe into the check, or several test checks.

mod cargo;
mod go;
mod launcher;
mod node;
mod place;
mod pytest;
mod quote;
#[cfg(test)]
mod tests;
mod unittest;

use std::path::Path;

use cortex_core::{CommandKind, RepoPath, ShellCommand};

use self::launcher::Launch;
use super::Commands;
use super::shell;
use super::target;

/// The most characters a narrow command has: it is a hint, so a long one is cut by dropping files.
pub const MAX_NARROW_COMMAND_CHARS: usize = 200;

/// What a whole-suite run needs kept to write a narrower command for the same runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteRun {
    /// The runner's name as the checks of a line name it: `pytest`, `unittest`, `go`, `cargo`,
    /// `jest` or `vitest`.
    pub runner: &'static str,
    /// The harmless variable assignments the line set before the runner, as written.
    pub env: Vec<String>,
    /// The words that start the runner as the agent launched it, subcommand included
    /// (`python -m pytest`, `cargo +nightly test`, `npx vitest run`); every word is plain.
    pub launcher: Vec<String>,
    /// The folder the check ran in, relative to the repository.
    pub dir: RepoPath,
    /// The folder to enter from where the line started, relative, when the line moved to `dir`
    /// with `cd`.
    pub enter: Option<String>,
    /// The one package a cargo run named with `-p`; the whole of what it ran.
    pub package: Option<String>,
    /// Whether a cargo run was built with `--release`.
    pub is_release: bool,
    /// The run's own arguments that select nothing, as written, repeated before the files.
    pub options: Vec<String>,
    /// The arguments the run passed on to the test binary (cargo's, after `--`), repeated after
    /// the files.
    pub test_args: Vec<String>,
}

/// A command that runs only some test files, and which of the files asked for it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowCommand {
    /// The command line, at most [`MAX_NARROW_COMMAND_CHARS`] characters.
    pub line: String,
    /// The files it runs, in the order they were asked for: a file the runner cannot be told by,
    /// outside the folder the suite ran in, or dropped to fit is not among them.
    pub files: Vec<RepoPath>,
}

/// What a runner's arguments say of a run that selects nothing.
#[derive(Debug, Default)]
pub(super) struct Whole {
    /// The words after the runner's own that the narrow command repeats (`test`, `run`).
    pub(super) tail: Vec<String>,
    /// The arguments that select nothing, repeated before the files.
    pub(super) options: Vec<String>,
    /// The arguments for the test binary, repeated after the files.
    pub(super) test_args: Vec<String>,
    /// The one package a cargo run named.
    pub(super) package: Option<String>,
    /// Whether the run was a release build.
    pub(super) is_release: bool,
}

/// One runner: the programs that name it, how its arguments show a whole run, and how a narrow
/// command is written for it.
struct Rule {
    /// The runner's name, as the checks of a line name it.
    runner: &'static str,
    /// The programs that name the runner, by file name in lower case without an extension.
    programs: &'static [&'static str],
    /// Whether the arguments after the runner's word select nothing.
    whole: fn(&[&str]) -> Option<Whole>,
    /// The arguments, after the launcher, that run only some of the files (relative to the run's
    /// folder), and which.
    narrow: fn(&SuiteRun, &[&str]) -> Option<Narrowed>,
}

/// The arguments that run some of the test files a runner was given, and which of them.
#[derive(Debug, Default)]
pub(super) struct Narrowed {
    /// The arguments, after the launcher and the run's options.
    pub(super) args: Vec<String>,
    /// The places, among the files given, of those the arguments run.
    pub(super) runs: Vec<usize>,
}

impl Narrowed {
    /// Adds `arg`, which runs the file at `place`, unless the arguments hold it already (two files
    /// of one Go package).
    pub(super) fn add(&mut self, place: usize, arg: String) {
        if !self.args.contains(&arg) {
            self.args.push(arg);
        }
        self.runs.push(place);
    }

    /// These arguments, or `None` when they run no file.
    pub(super) fn found(self) -> Option<Self> {
        (!self.runs.is_empty()).then_some(self)
    }
}

/// The runners a narrow command can be written for. A runner appears in one row only.
const RULES: &[Rule] = &[
    Rule {
        runner: "pytest",
        programs: &["pytest", "py.test"],
        whole: pytest::whole,
        narrow: pytest::narrow,
    },
    Rule {
        runner: "unittest",
        programs: &["unittest"],
        whole: unittest::whole,
        narrow: unittest::narrow,
    },
    Rule {
        runner: "go",
        programs: &["go"],
        whole: go::whole,
        narrow: go::narrow,
    },
    Rule {
        runner: "cargo",
        programs: &["cargo"],
        whole: cargo::whole,
        narrow: cargo::narrow,
    },
    Rule {
        runner: "jest",
        programs: &["jest"],
        whole: node::whole_jest,
        narrow: node::narrow_jest,
    },
    Rule {
        runner: "vitest",
        programs: &["vitest"],
        whole: node::whole_vitest,
        narrow: node::narrow_vitest,
    },
];

/// The rule of the runner `name`.
fn rule_of(name: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.runner == name)
}

/// The whole-suite run `command` was, when it ran one test check of a known runner and nothing
/// in it selects tests; run in the folder `cwd` of the repository whose top is `root`. `None`
/// when it ran no test check, one that selects, or anything uncertain.
#[must_use]
pub fn whole_suite(
    commands: &Commands,
    command: &ShellCommand,
    root: &Path,
    cwd: &Path,
) -> Option<SuiteRun> {
    let told = commands.tell(command, root, cwd);
    let mut tests = told
        .checks
        .iter()
        .filter(|check| check.kind == CommandKind::Test);
    let check = *tests.next()?;
    if tests.next().is_some() {
        return None;
    }
    let rule = rule_of(check.runner)?;
    let list = shell::read(command.as_str()).ok()?;
    let located = target::locate(&list, check, false)?;
    if located.is_grouped || located.is_piped_into {
        return None;
    }
    let launch = Launch::read(located.words, rule.programs)?;
    let args: Vec<&str> = launch.args.iter().map(String::as_str).collect();
    let whole = (rule.whole)(&args)?;
    let place = place::of(&list, located.words, root, cwd)?;
    let mut words = launch.words;
    words.extend(whole.tail);
    Some(SuiteRun {
        runner: rule.runner,
        env: launch.env,
        launcher: words,
        dir: place.dir,
        enter: place.enter,
        package: whole.package,
        is_release: whole.is_release,
        options: whole.options,
        test_args: whole.test_args,
    })
}

/// The command that runs only `test_files` (repository paths, a few of them) the way `run` ran the
/// whole suite: the launcher, the run's options, the files the runner can be told by, the folder
/// entered first when the line entered one; with the files it runs. Files that do not map to the
/// runner's way of naming tests, or lie outside the folder the suite ran in, are left out; files
/// are dropped from the end until the command is at most [`MAX_NARROW_COMMAND_CHARS`]
/// characters. `None` when no file maps or even one does not fit.
#[must_use]
pub fn narrow_command(run: &SuiteRun, test_files: &[RepoPath]) -> Option<NarrowCommand> {
    let rule = rule_of(run.runner)?;
    let mut files: Vec<(&RepoPath, String)> = Vec::new();
    for file in test_files {
        let Some(relative) = place::within(&run.dir, file).filter(|path| !path.starts_with('-'))
        else {
            continue;
        };
        if !files.iter().any(|(_, kept)| *kept == relative) {
            files.push((file, relative));
        }
    }
    (1..=files.len()).rev().find_map(|count| {
        let kept: Vec<&str> = files
            .iter()
            .take(count)
            .map(|(_, relative)| relative.as_str())
            .collect();
        let narrowed = (rule.narrow)(run, &kept)?;
        let line = render(run, &narrowed.args)?;
        let files = narrowed
            .runs
            .iter()
            .filter_map(|place| files.get(*place))
            .map(|(file, _)| (*file).clone())
            .collect();
        Some(NarrowCommand { line, files })
    })
}

/// The command line of `run` with the narrow arguments `args`, when it fits.
fn render(run: &SuiteRun, args: &[String]) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    if let Some(enter) = &run.enter {
        words.extend(["cd".to_owned(), quote::word(enter)?, "&&".to_owned()]);
    }
    words.extend(run.env.iter().cloned());
    words.extend(run.launcher.iter().cloned());
    for arg in run.options.iter().chain(args).chain(&run.test_args) {
        words.push(quote::word(arg)?);
    }
    let line = words.join(" ");
    (line.chars().count() <= MAX_NARROW_COMMAND_CHARS).then_some(line)
}
