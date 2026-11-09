//! `cortex where "query"`: what the retrieval engine finds for a question, without the gate: the
//! files, the code it would hand over and why, and what the gate would have said, as the daemon
//! writes them. The daemon is started when none runs. When there is no answer, the command says
//! whether the daemon has not built its graph yet or found nothing for the question.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::UserFolder;
use cortex_core::ipc::{RequestBody, ResponseBody};
use cortex_hook::Answer;

use crate::{output, repository, upgrade};

/// How long the daemon has to answer, a daemon started by this command included: it answers once
/// it listens, and a question's spreading stops well within a second.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// How long the daemon has to give its status, asked when it had no answer.
const STATUS_WITHIN: Duration = Duration::from_secs(10);

/// The arguments of `cortex where`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The question, in the words a prompt would use.
    query: String,
    /// The most files to name, from 1 to 5.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=5))]
    limit: u8,
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Prints what the engine finds for the question, or why there is nothing to print.
///
/// # Errors
///
/// A root that is not a folder, no home folder to find the daemon from, a daemon that cannot be
/// started or does not answer, a question it could not answer, or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let body = RequestBody::Where {
        query: args.query.clone(),
        limit: args.limit,
    };
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
        ResponseBody::Reply { text: None, .. } => output::line(nothing_found(&root, &user)),
        ResponseBody::Error { message } => bail!("the daemon could not answer: {message}"),
        other => bail!("the daemon answered the question with {other:?}"),
    }
}

/// Why the daemon of the repository at `root`, whose user folder is `user`, gave no answer: it has
/// not built its graph yet, as before its first index ends, or the engine found nothing to show.
fn nothing_found(root: &Path, user: &UserFolder) -> &'static str {
    let asked = upgrade::ask_current(
        root,
        user,
        &RequestBody::Status,
        Instant::now() + STATUS_WITHIN,
    );
    let has_graph = matches!(
        &asked.answer,
        Ok(Answer { body: ResponseBody::Status(report), .. })
            if report.graph.as_ref().is_some_and(|graph| graph.builds > 0)
    );
    if has_graph {
        "nothing found: the question points at no file the engine can show"
    } else {
        "nothing yet: the daemon has not built its graph; `cortex status` says when it has"
    }
}
