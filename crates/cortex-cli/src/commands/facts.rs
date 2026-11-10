//! `cortex facts list|confirm|retire|edit`: the project rules the daemon keeps, listed, and the
//! user's word on one of them, refused from inside a coding agent's shell. The daemon, the store's
//! one writer, carries it out, and is started when none runs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cortex_core::ipc::{FactLine, FactsAnswer, FactsAsk, RequestBody, ResponseBody};
use cortex_core::{UserFolder, is_agent_shell};

use crate::{output, repository, upgrade};

/// How long the daemon has to answer, a daemon started by this command included: it takes a fact
/// request within 10 s and writes it within 5 s more.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex facts`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// What to do.
    #[command(subcommand)]
    action: Action,
    /// The repository's root folder; the one the current folder is in when left out.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

/// What `cortex facts` does.
#[derive(Debug, clap::Subcommand)]
enum Action {
    /// List the facts that are active, candidates or suspect.
    List {
        /// List retired facts too.
        #[arg(long)]
        all: bool,
    },
    /// Say a fact holds for the code as it is now: it becomes active.
    Confirm {
        /// The fact's number, as the list shows it.
        id: i64,
    },
    /// Say a fact no longer holds: it is never given to an agent again.
    Retire {
        /// The fact's number.
        id: i64,
    },
    /// Correct a fact: the new text replaces it, about the same files and symbols.
    Edit {
        /// The fact's number.
        id: i64,
        /// The corrected rule, at most 240 characters.
        text: String,
    },
}

/// Lists the facts, or carries the user's word on one, and prints the outcome.
///
/// # Errors
///
/// A root that is not a folder, no home folder, a daemon that cannot be started or does not answer,
/// a request it refused (no such fact, a fact corrected since, a file gone), or stdout that cannot
/// be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let is_word = !matches!(args.action, Action::List { .. });
    // INVARIANT: no rule an agent states is the user's (an agent's shell gives no person's word)
    if is_word && is_agent_shell(|name| std::env::var_os(name)) {
        bail!(
            "a person confirms, retires or corrects a fact, not a coding agent: run this in your \
             own terminal"
        );
    }
    let ask = match &args.action {
        Action::List { all } => FactsAsk::List { all: *all },
        Action::Confirm { id } => FactsAsk::Confirm { id: *id },
        Action::Retire { id } => FactsAsk::Retire { id: *id },
        Action::Edit { id, text } => FactsAsk::Edit {
            id: *id,
            text: text.clone(),
        },
    };
    let root = repository::root(args.root.as_deref())?;
    let answer = match answer_of(&root, &RequestBody::Facts(ask))? {
        ResponseBody::Facts(answer) => answer,
        other => bail!("the daemon answered with {other:?}"),
    };
    match answer {
        FactsAnswer::Listed { facts } if facts.is_empty() => output::line("no facts"),
        FactsAnswer::Listed { facts } => facts
            .iter()
            .try_for_each(|fact| output::line(&line_of(fact))),
        FactsAnswer::Confirmed { id } => output::line(&format!("fact {id} confirmed")),
        FactsAnswer::Retired { id } => output::line(&format!("fact {id} retired")),
        FactsAnswer::Edited { id, by } => {
            output::line(&format!("fact {id} corrected; the correction is fact {by}"))
        }
        FactsAnswer::Noted { .. } => bail!("the daemon answered with a note"),
    }
}

/// The daemon's answer to `body`, a request that may change what the daemon keeps (a note, the
/// user's word on a fact, a claim), for the repository at `root`: sent to one daemon only.
///
/// # Errors
///
/// No home folder, a daemon that cannot be started or does not answer, or one that refused the
/// request, in its words.
pub(crate) fn answer_of(root: &Path, body: &RequestBody) -> anyhow::Result<ResponseBody> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let asked = upgrade::ask_once_started(root, &user, body, Instant::now() + ANSWER_WITHIN);
    if let Some(old) = &asked.replaced {
        output::note(&format!("replaced a daemon of Cortex {old}"))?;
    }
    if let Some(error) = &asked.not_replaced {
        output::note(&format!(
            "an older daemon answered and was not replaced: {error}"
        ))?;
    }
    let answer = asked
        .answer
        .with_context(|| format!("asking the daemon of {}", root.display()))?;
    match answer.body {
        ResponseBody::Error { message } => bail!("{message}"),
        body => Ok(body),
    }
}

/// One fact as the list prints it: its number, status, kind and text, what it is about, and how
/// often it was given to an agent when it was.
fn line_of(fact: &FactLine) -> String {
    let used = match fact.uses {
        0 => String::new(),
        1 => ", given once".to_owned(),
        times => format!(", given {times} times"),
    };
    format!(
        "{:>4}  {:<9}  {:<10}  {}  (at {}{used})",
        fact.id,
        fact.status.name(),
        fact.kind.name(),
        fact.text,
        fact.anchors.join(", ")
    )
}
