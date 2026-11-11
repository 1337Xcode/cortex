//! `cortex import`: reads a team's shared facts, `.cortex/shared/facts.json`, strictly (see
//! [`crate::shared_facts`]; a file that is not right is refused whole and nothing is imported), and
//! notes each as an import's: a candidate, never given to an agent until a person confirms it with
//! `cortex facts confirm`. A fact already kept on the same files and symbols in like words is not
//! added again, and one retired here stays retired. A fact this clone cannot anchor (a file it does
//! not index, a symbol a file does not define) is skipped and said; the rest are imported. The
//! daemon writes them, and is started when none runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use cortex_core::ipc::{FactsAnswer, FactsAsk, Note, NoteOutcome, RequestBody, ResponseBody};
use cortex_core::{FactSource, FactStatus, UserFolder};

use crate::commands::facts::answer_of;
use crate::shared_facts::{
    FACTS_FILE, SHARED_FOLDER, SharedFact, read_facts_file, read_shared_facts,
};
use crate::{output, repository, upgrade};

/// How long the daemon has to answer each fact, a daemon started by this command included: it
/// takes a fact request within 10 s and writes it within 5 s more.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex import`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the one the current folder is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Imports the shared facts and says what became of each.
///
/// # Errors
///
/// A root that is not a folder, no shared facts file, one that is refused, a daemon that cannot be
/// started or does not answer, or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let (root, _) = repository::root_and_worktree(args.root.as_deref())?;
    let Some((path, text)) = read_shared_facts(&root)? else {
        bail!(
            "no shared facts at {}; a teammate's `cortex export` writes them",
            root.join(".cortex")
                .join(SHARED_FOLDER)
                .join(FACTS_FILE)
                .display()
        );
    };
    let facts = read_facts_file(&text).with_context(|| format!("refused {}", path.display()))?;
    let retired = retired_here(&root)?;
    let mut added = 0;
    for fact in &facts {
        match noted(&root, fact)? {
            Ok((id, NoteOutcome::Added)) => {
                added += 1;
                output::line(&format!("fact {id} added, a candidate: {}", fact.text))?;
            }
            Ok((id, NoteOutcome::Merged)) if retired.contains(&id) => {
                output::line(&format!(
                    "fact {id} was retired here, and stays so: {}",
                    fact.text
                ))?;
            }
            Ok((id, NoteOutcome::Merged | NoteOutcome::Activated)) => {
                output::line(&format!("fact {id} says this already: {}", fact.text))?;
            }
            Err(why) => output::line(&format!("skipped: {}: {why}", fact.text))?,
        }
    }
    let imported = match added {
        1 => "1 fact".to_owned(),
        count => format!("{count} facts"),
    };
    output::line(&format!(
        "{imported} imported as candidates: `cortex facts list` shows them, and each is given to \
         agents only once `cortex facts confirm <number>` says it holds here"
    ))
}

/// The numbers of the facts retired in the repository at `root`, as its daemon lists them.
///
/// # Errors
///
/// No home folder, a daemon that cannot be started or does not answer, or one that answers with
/// something other than the list.
fn retired_here(root: &Path) -> anyhow::Result<BTreeSet<i64>> {
    match answer_of(root, &RequestBody::Facts(FactsAsk::List { all: true }))? {
        ResponseBody::Facts(FactsAnswer::Listed { facts }) => Ok(facts
            .into_iter()
            .filter(|fact| fact.status == FactStatus::Retired)
            .map(|fact| fact.id)
            .collect()),
        other => bail!("the daemon answered the list with {other:?}"),
    }
}

/// What became of `fact`, noted as an import's with the daemon of the repository at `root`: its
/// number and whether it is new, or, inside, the daemon's words when it refused it.
///
/// # Errors
///
/// No home folder, a daemon that cannot be started or does not answer, or an answer that is
/// neither a note's nor a refusal.
fn noted(root: &Path, fact: &SharedFact) -> anyhow::Result<Result<(i64, NoteOutcome), String>> {
    let note = Note {
        text: fact.text.clone(),
        kind: fact.kind,
        source: FactSource::Import,
        at: fact.at.clone(),
    };
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let asked = upgrade::ask_once_started(
        root,
        &user,
        &RequestBody::Note(note),
        Instant::now() + ANSWER_WITHIN,
    );
    if let Some(old) = &asked.replaced {
        output::note(&format!("replaced a daemon of Cortex {old}"))?;
    }
    let answer = asked
        .answer
        .with_context(|| format!("asking the daemon of {}", root.display()))?;
    match answer.body {
        ResponseBody::Facts(FactsAnswer::Noted { id, outcome }) => Ok(Ok((id, outcome))),
        ResponseBody::Error { message } => Ok(Err(message)),
        other => bail!("the daemon answered the note with {other:?}"),
    }
}
