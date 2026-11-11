//! What init writes for one agent, planned before anything is asked: its hooks merged into a
//! settings file (see [`hooks`]), the files of Cortex's own in its folder (see [`own`]), Cortex's
//! MCP server merged into its JSON settings (see [`mcp`]), and what the user is to do themselves,
//! said once everything is written: the steps Cortex never takes, and the lines to add to a file
//! Cortex does not write.

use std::path::PathBuf;

use anyhow::Context as _;

use super::{Change, Owner, Scope, hooks, mcp, own};
use crate::agents::Setup;
use crate::backup::Kind;

/// What a file init writes for an agent holds of Cortex's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum What {
    /// Cortex's hooks, merged into the agent's settings.
    Hooks,
    /// A file of Cortex's own, as the agent calls it (`plugin`, `hook`).
    Own(&'static str),
    /// Cortex's MCP server, merged into the agent's settings.
    Mcp,
}

impl What {
    /// What it is called, and the verb that goes with it.
    fn words(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Hooks => ("hooks", "are", "serve"),
            Self::Own(what) => (what, "is", "serves"),
            Self::Mcp => ("MCP server", "is", "serves"),
        }
    }
}

/// A file init writes for an agent, what it holds, and what it holds once written.
#[derive(Debug)]
pub(super) struct AgentFile {
    /// The agent.
    pub(super) setup: &'static Setup,
    /// The file, links followed.
    pub(super) file: PathBuf,
    /// What it holds, or `None` when there is none.
    pub(super) before: Option<String>,
    /// What it holds once written.
    pub(super) after: String,
    /// Whose it is.
    pub(super) owner: Owner,
    /// What the file is, for the manifest.
    pub(super) kind: Kind,
    /// What it holds of Cortex's.
    pub(super) what: What,
    /// The line that has git leave the file out, for a file of the repository's.
    pub(super) ignore_line: Option<&'static str>,
    /// Whether the agent runs the file as a program.
    pub(super) is_executable: bool,
}

impl AgentFile {
    /// Whether the file holds what init would write already.
    pub(super) fn is_current(&self) -> bool {
        self.before.as_deref() == Some(self.after.as_str())
    }

    /// The repository's file init makes, which git should then leave out; none when the file is
    /// there already or is the user's.
    pub(super) fn made_in_repository(&self) -> Option<&'static str> {
        let is_made = self.before.is_none() && matches!(self.owner, Owner::Repository { .. });
        self.ignore_line.filter(|_| is_made)
    }

    /// What init says when the file holds what it would write already.
    pub(super) fn current(&self) -> String {
        let (what, is, _) = self.what.words();
        format!(
            "{}: the {what} in {} {is} current",
            self.setup.agent.title(),
            self.file.display()
        )
    }

    /// The change that writes the file.
    pub(super) fn into_change(self) -> Change {
        let (what, _, serve) = self.what.words();
        // A user's file serves every repository, and only `--scope user` takes Cortex's entries
        // out of it again.
        let reach = match self.owner {
            Owner::User => format!(
                ", which {serve} every repository; `cortex uninstall --scope user` takes {} out",
                if self.what == What::Hooks {
                    "them"
                } else {
                    "it"
                }
            ),
            Owner::Repository { .. } => String::new(),
        };
        Change {
            said: format!(
                "{}: {what} written to {}{reach}",
                self.setup.agent.title(),
                self.file.display()
            ),
            file: self.file,
            kind: self.kind,
            before: self.before,
            after: self.after,
            owner: self.owner,
            is_executable: self.is_executable,
        }
    }
}

/// What init writes for one agent, and what it says the user is to do.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// The files it writes.
    pub(super) files: Vec<AgentFile>,
    /// What the user is to do, each said as a line once everything is written.
    pub(super) told: Vec<String>,
}

/// What init writes for `setup`'s agent at `scope`, for the working tree whose canonical top is
/// `root` (none for the user's own files alone, as `--global` writes them). Nothing is written.
///
/// # Errors
///
/// As [`hooks::planned`], [`own::planned`] and [`mcp::planned`]; or this program's path cannot be
/// found, or written for the lines the user is told to add.
pub(super) fn planned(
    scope: Scope,
    root: Option<&std::path::Path>,
    setup: &'static Setup,
) -> anyhow::Result<Plan> {
    let mut plan = Plan::default();
    plan.files.extend(hooks::planned(scope, root, setup)?);
    if setup.own.is_none() && setup.mcp.is_none() && setup.told_lines.is_none() {
        return Ok(plan);
    }
    let program = crate::program::this_program()?;
    plan.files
        .extend(own::planned(scope, root, setup, &program)?);
    match mcp::planned(setup, &program)? {
        Some(mcp::Planned::File(file)) => plan.files.push(*file),
        Some(mcp::Planned::Told(told)) => plan.told.push(told),
        None => {}
    }
    if let Some(told) = &setup.told_lines {
        let home = std::env::home_dir().context("finding the home folder")?;
        let lines = (told.lines)(&program)?;
        plan.told.push(format!(
            "Cortex writes nothing for it, since {}; add these lines to {} yourself, beside any \
             keys of the same name there:\n{}",
            told.why,
            super::place::joined(&home, told.file).display(),
            lines.trim_end()
        ));
    }
    Ok(plan)
}
