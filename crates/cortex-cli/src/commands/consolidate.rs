//! `cortex consolidate`: asks the repository's daemon to run upkeep as soon as its writer is free,
//! instead of waiting for a quiet spell. The daemon answers at once; upkeep runs in the background,
//! and `cortex status` says what its last run removed. The daemon is started when none runs.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::UserFolder;
use cortex_core::ipc::{RequestBody, ResponseBody};

use crate::{output, repository, upgrade};

/// How long the daemon has to answer, a daemon started by this command included: it answers as
/// soon as it listens.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex consolidate`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Asks for upkeep and says it was asked for.
///
/// # Errors
///
/// A root that is not a folder, no home folder, a daemon that cannot be started or does not
/// answer, or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let asked = upgrade::ask_started(
        &root,
        &user,
        &RequestBody::Consolidate,
        Instant::now() + ANSWER_WITHIN,
    );
    if let Some(old) = &asked.replaced {
        output::note(&format!("replaced a daemon of Cortex {old}"))?;
    }
    let answer = asked
        .answer
        .with_context(|| format!("asking the daemon of {}", root.display()))?;
    match answer.body {
        ResponseBody::Ack => output::line(
            "upkeep asked for: it runs as soon as the daemon is free; `cortex status` says what it \
             removed",
        ),
        ResponseBody::Error { message } => bail!("{message}"),
        other => bail!("the daemon answered with {other:?}"),
    }
}
