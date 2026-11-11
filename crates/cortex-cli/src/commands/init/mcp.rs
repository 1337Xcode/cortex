//! Cortex's MCP server in an agent's JSON settings (see [`crate::agents::mcp`]), always the user's
//! file, whatever the scope: an agent's settings of a repository's are meant to be committed, or
//! run a server only once the user approves it, and the entry names this machine's program. A file
//! Cortex cannot rewrite without losing something of the user's (comments, or a server of the
//! same name) is left as it is, and init says what to add instead, under another name when the
//! name is taken: the agent is set up all the same. So it does for an agent whose file of the
//! user's Cortex cannot find (Roo Code's), saying where the agent lets the user add it.

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, read_text_file};

use super::agent_file::{AgentFile, What};
use super::{Owner, place};
use crate::agents::Setup;
use crate::agents::mcp::{
    NAME, OTHER_NAME, by_hand_step, entry, is_name_taken, merged, servers_path,
};
use crate::backup::Kind;

/// What init does with an agent's MCP settings.
#[derive(Debug)]
pub(super) enum Planned {
    /// Writes them, with Cortex's server.
    File(Box<AgentFile>),
    /// Leaves them as they are, and says what the user is to add.
    Told(String),
}

/// Cortex's MCP server, run by `program`, in `setup`'s agent's settings, or what to tell the user
/// when that file cannot take it; `None` for an agent Cortex writes no MCP server for. Nothing is
/// written.
///
/// # Errors
///
/// No home folder, or a file that is a link to nothing, is read-only, or cannot be read as small
/// text.
pub(super) fn planned(setup: &'static Setup, program: &str) -> anyhow::Result<Option<Planned>> {
    let Some(mcp) = &setup.mcp else {
        return Ok(None);
    };
    if let Some(step) = by_hand_step(mcp, program) {
        return Ok(Some(Planned::Told(step)));
    }
    let mut paths = Vec::with_capacity(mcp.files().len());
    for user in mcp.files() {
        paths.push(user.resolved()?);
    }
    let path = paths
        .iter()
        .find(|path| path.exists())
        .or_else(|| paths.first())
        .context("the agent names no settings file")?;
    let (folder, name) = path
        .parent()
        .zip(path.file_name().and_then(|name| name.to_str()))
        .with_context(|| format!("{} names no file", path.display()))?;
    let file = place::file_beside(folder, name)?;
    place::writable(&file)?;
    let before = read_text_file(&file, MAX_SETTINGS_BYTES)?;
    match merged(before.as_deref(), program, mcp) {
        Ok(after) => Ok(Some(Planned::File(Box::new(AgentFile {
            setup,
            file,
            before,
            after,
            owner: Owner::User,
            kind: Kind::AgentMcp {
                agent: setup.agent.name().to_owned(),
            },
            what: What::Mcp,
            left_out: None,
            is_executable: false,
        })))),
        Err(error) => {
            let is_taken = before
                .as_deref()
                .is_some_and(|before| is_name_taken(before, program, mcp));
            let (name, why) = if is_taken {
                (OTHER_NAME, String::new())
            } else {
                (
                    NAME,
                    ", and it never rewrites a file with comments".to_owned(),
                )
            };
            Ok(Some(Planned::Told(format!(
                "add Cortex's MCP server to {} yourself, under `{}`: \"{name}\": {}; Cortex \
                 leaves that file as it is, since {error:#}{why}",
                file.display(),
                servers_path(mcp),
                entry(program, mcp.shape)
            ))))
        }
    }
}
