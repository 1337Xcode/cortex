//! `cortex export`: writes the repository's active facts to `.cortex/shared/facts.json` (see
//! [`crate::shared_facts`]), for a team that shares them by committing `.cortex/shared/`. Only the
//! rules' words and what they are about: never code, prompts or events. A fact the file could not
//! hold, as `cortex import` reads it (one about code that is gone, with more anchors than a shared
//! fact names, a name too long), is left out and said, and so are the facts past the most a file
//! holds, the last in the file's order. The daemon lists the facts, and is started when none runs.
//! The file is written whole, through a new file renamed over the old one, and only as a file of
//! its own in folders of the repository's own, never through a link; written again with nothing
//! changed, it keeps its bytes.

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use cortex_core::ipc::{FactsAnswer, FactsAsk, RequestBody, ResponseBody};
use cortex_core::{FactStatus, read_text_file};

use crate::backup::replace_unchanged;
use crate::commands::facts::answer_of;
use crate::commands::init::own_folder;
use crate::shared_facts::{
    FACTS_FILE, MAX_FACTS_FILE_BYTES, MAX_SHARED_FACTS, SHARED_FOLDER, SharedFact, facts_file,
    is_own_file, shareable,
};
use crate::{output, repository};

/// The arguments of `cortex export`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the one the current folder is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Writes the shared facts and says how many, and where.
///
/// # Errors
///
/// A root that is not a folder, a daemon that cannot be started or does not answer, a `.cortex` or
/// shared folder that is a link, a file there that cannot be read or written, or stdout that
/// cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let listed = match answer_of(&root, &RequestBody::Facts(FactsAsk::List { all: false }))? {
        ResponseBody::Facts(FactsAnswer::Listed { facts }) => facts,
        other => bail!("the daemon answered the list with {other:?}"),
    };
    let mut shared = Vec::new();
    for fact in listed
        .into_iter()
        .filter(|fact| fact.status == FactStatus::Active)
    {
        let id = fact.id;
        let candidate = SharedFact {
            text: fact.text,
            kind: fact.kind,
            at: fact.anchors,
        };
        match shareable(&candidate) {
            Ok(()) => shared.push(candidate),
            Err(why) => output::note(&format!("fact {id} is not shared: {why:#}"))?,
        }
    }
    if shared.len() > MAX_SHARED_FACTS {
        output::note(&format!(
            "{} active facts; the file holds the first {MAX_SHARED_FACTS} in its order",
            shared.len()
        ))?;
    }
    let folder = own_folder(&own_folder(&root, ".cortex")?, SHARED_FOLDER)?;
    let path = folder.join(FACTS_FILE);
    let before = if is_own_file(&path)? {
        read_text_file(&path, MAX_FACTS_FILE_BYTES)
            .with_context(|| format!("reading {}", path.display()))?
    } else {
        None
    };
    let text = facts_file(&shared, MAX_SHARED_FACTS)?;
    let count = match shared.len().min(MAX_SHARED_FACTS) {
        1 => "1 active fact".to_owned(),
        count => format!("{count} active facts"),
    };
    if before.as_deref() == Some(text.as_str()) {
        return output::line(&format!("{count} in {}, as before", path.display()));
    }
    replace_unchanged(&path, before.as_deref().map(str::as_bytes), text.as_bytes())?;
    output::line(&format!(
        "{count} written to {}; commit it for your team to `cortex import`",
        path.display()
    ))
}
