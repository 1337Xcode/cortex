//! Telling what a shell command line ran: the checks of the runners Cortex knows (tests, builds,
//! type checks and linters), the commands the user's `tests.extra_patterns` name, and git restores
//! with the paths they name. Only the commands the line's exit status vouches for are told: when
//! the line exits 0, each of them ran and exited 0. That holds for a command that ends its
//! pipeline (`cargo test | tail` exits as `tail` does), in the line's last and-or list, run
//! whenever the commands before it in that list succeeded and followed by `&&` alone, neither
//! inverted by `!` nor sent to the background. When the line exits otherwise, any of them may have
//! failed or not run. A line the reader does not follow tells nothing.
//!
//! Nothing here keeps the command's text: the caller keeps what was told, never the line.

mod argv;
mod folder;
mod restore;
mod runners;
mod shell;

use std::path::Path;

use cortex_core::{CommandKind, RepoPath, ShellCommand};
use regex::Regex;
use thiserror::Error;

use self::folder::Folder;
use self::shell::{AndOr, Command, Join, List, Pipeline, Word};

/// The runner's name for a command one of the user's `tests.extra_patterns` names.
pub const CONFIGURED: &str = "configured";

/// A check a command ran: the runner, by its name, and what it checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    /// The runner, such as `cargo` or `pytest`, or [`CONFIGURED`].
    pub runner: &'static str,
    /// What it checks.
    pub kind: CommandKind,
}

/// What a git restore put back as git holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restore {
    /// Every file.
    Everything,
    /// These files and folders, in path order; a folder stands for everything in it.
    Paths(Vec<RepoPath>),
}

/// A command a line ran that Cortex tells apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// A check.
    Check(Check),
    /// A git restore.
    Restore(Restore),
}

/// Why one of the user's patterns cannot be used.
#[derive(Debug, Error)]
#[error("tests.extra_patterns: {pattern:?} is not a regular expression: {reason}")]
pub struct PatternError {
    /// The pattern.
    pattern: String,
    /// Why it does not compile.
    reason: String,
}

/// The commands Cortex tells apart: the runners it knows and the user's patterns, compiled once.
/// The default knows the runners alone.
#[derive(Debug, Default)]
pub struct Commands {
    /// The user's patterns.
    patterns: Vec<Regex>,
}

impl Commands {
    /// The runners Cortex knows and the commands `patterns` (`tests.extra_patterns`) name, as
    /// regular expressions matched against each command's words after the variable assignments
    /// before it, joined by single spaces. A pattern names tests.
    ///
    /// # Errors
    ///
    /// A pattern that is not a regular expression; the configuration refuses such a pattern
    /// before it gets here.
    pub fn new(patterns: &[String]) -> Result<Self, PatternError> {
        let patterns = patterns
            .iter()
            .map(|pattern| {
                Regex::new(pattern).map_err(|error| PatternError {
                    pattern: pattern.clone(),
                    reason: error.to_string(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { patterns })
    }

    /// What `command`, run in the folder `cwd` of the repository whose top is `root`, ran that
    /// its exit status vouches for, in the order of the line.
    #[must_use]
    pub fn recognise(&self, command: &ShellCommand, root: &Path, cwd: &Path) -> Vec<Ran> {
        let Ok(list) = shell::read(command.as_str()) else {
            return Vec::new();
        };
        let mut walk = Walk {
            commands: self,
            root,
            ran: Vec::new(),
        };
        walk.list(&list, &mut Folder::new(cwd), true);
        walk.ran
    }

    /// What the simple command `words`, run from `folder` in the repository whose top is `root`,
    /// is, if Cortex tells it apart: a git restore, a runner's check, or a command the user's
    /// patterns name.
    fn ran(&self, words: &[Word], folder: &Folder, root: &Path) -> Option<Ran> {
        if let Some(invocation) = argv::invocation(words) {
            if invocation.program == "git" {
                return restore::restore(invocation.args, folder, root).map(Ran::Restore);
            }
            if let Some(check) = runners::check(&invocation) {
                return Some(Ran::Check(check));
            }
        }
        let text = argv::command_text(words);
        self.patterns
            .iter()
            .any(|pattern| pattern.is_match(&text))
            .then_some(Ran::Check(Check {
                runner: CONFIGURED,
                kind: CommandKind::Test,
            }))
    }
}

/// A walk through a line's commands, in order, keeping what it tells apart.
struct Walk<'a> {
    /// What tells the commands apart.
    commands: &'a Commands,
    /// The repository's top folder.
    root: &'a Path,
    /// What was told so far.
    ran: Vec<Ran>,
}

impl Walk<'_> {
    /// Walks `list` from `folder`. When `is_vouched`, the line's exit status is the list's, which
    /// is its last and-or list's unless that one runs in the background.
    fn list(&mut self, list: &List, folder: &mut Folder, is_vouched: bool) {
        let final_at = list.items.len().checked_sub(1);
        for (at, item) in list.items.iter().enumerate() {
            let is_final = Some(at) == final_at;
            self.and_or(item, folder, is_vouched && is_final && !item.is_background);
        }
    }

    /// Walks `and_or` from `folder`. When `is_vouched`, a pipeline in it is vouched for when it
    /// runs whenever the ones before it succeeded (it is first, or follows `&&`) and only `&&`
    /// follows it: the list then exits 0 only when it ran and exited 0.
    fn and_or(&mut self, and_or: &AndOr, folder: &mut Folder, is_vouched: bool) {
        for (at, pipeline) in and_or.pipelines.iter().enumerate() {
            let runs_after_success = at == 0 || and_or.joins.get(at - 1) == Some(&Join::And);
            let decides = tail(&and_or.joins, at)
                .iter()
                .all(|join| *join == Join::And);
            self.pipeline(
                pipeline,
                folder,
                is_vouched && runs_after_success && decides,
            );
        }
    }

    /// Walks `pipeline` from `folder`. When `is_vouched` and nothing inverts it, its last command
    /// is vouched for. A pipeline of several commands runs each in a shell of its own, so a `cd`
    /// in one moves nothing after it.
    fn pipeline(&mut self, pipeline: &Pipeline, folder: &mut Folder, is_vouched: bool) {
        let last = pipeline.commands.len().saturating_sub(1);
        let is_alone = pipeline.commands.len() == 1;
        for (at, command) in pipeline.commands.iter().enumerate() {
            let vouched = is_vouched && !pipeline.is_negated && at == last;
            if is_alone {
                self.command(command, folder, vouched);
            } else {
                self.command(command, &mut folder.clone(), vouched);
            }
        }
    }

    /// Walks `command` from `folder`, which a `cd` moves for what comes after it.
    fn command(&mut self, command: &Command, folder: &mut Folder, is_vouched: bool) {
        match command {
            Command::Simple(words) => {
                if let Some(moved) = folder.after(words) {
                    *folder = moved;
                } else if is_vouched {
                    let ran = self.commands.ran(words, folder, self.root);
                    self.ran.extend(ran);
                }
            }
            Command::Group { list, is_subshell } => {
                if *is_subshell {
                    self.list(list, &mut folder.clone(), is_vouched);
                } else {
                    self.list(list, folder, is_vouched);
                }
            }
        }
    }
}

/// The items of `items` from `at` on: none when `at` is past the end.
fn tail<T>(items: &[T], at: usize) -> &[T] {
    items.get(at..).unwrap_or_default()
}
