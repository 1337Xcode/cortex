//! `cortex why TARGET`: why code is where it is, as the daemon tells it: the files its file is
//! linked to and how strongly, the project facts about it that are true now, the sessions working
//! on its file now, and the warnings about it whose tasks have not ended. The target is a file
//! from the current folder, that file and `:line` or `:symbol`, or a symbol's name. The daemon is
//! started when none runs.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::UserFolder;
use cortex_core::ipc::{RequestBody, ResponseBody};

use crate::commands::note::file_target;
use crate::{output, repository, upgrade};

/// How long the daemon has to answer, a daemon started by this command included: it answers once
/// it listens, and an answer reads a few rows and the target's file.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex why`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// A file, `file:line`, `file:symbol`, or a symbol's name.
    target: String,
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Prints why the target is where it is.
///
/// # Errors
///
/// A root that is not a folder, a file outside the repository, no home folder to find the daemon
/// from, a daemon that cannot be started or does not answer, a target it refused (naming nothing,
/// or several things), or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let target = match file_target(&root, &args.target) {
        Some(target) => target?,
        None => args.target.clone(),
    };
    let body = RequestBody::Why { target };
    let asked = upgrade::ask_started(&root, &user, &body, Instant::now() + ANSWER_WITHIN);
    if let Some(old) = &asked.replaced {
        output::note(&format!("replaced a daemon of Cortex {old}"))?;
    }
    let answer = asked
        .answer
        .with_context(|| format!("asking the daemon of {}", root.display()))?;
    match answer.body {
        ResponseBody::Reply {
            text: Some(text), ..
        } => output::lines(&text),
        ResponseBody::Error { message } => bail!("{message}"),
        other => bail!("the daemon answered with {other:?}"),
    }
}
