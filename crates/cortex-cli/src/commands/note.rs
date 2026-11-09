//! `cortex note "text" --at path[:symbol]...`: a project rule of the user's, active at once,
//! anchored to the files and symbols it is about. Each anchor is a file inside the repository,
//! given from the current folder, optionally followed by `:` and a symbol it defines; or the
//! repository's root folder itself, for a rule about the whole repository. The daemon writes it,
//! and is started when none runs.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use cortex_core::ipc::{FactsAnswer, Note, NoteOutcome, RequestBody, ResponseBody};
use cortex_core::{FactKind, FactSource, RepoPath};

use crate::commands::facts::answer_of;
use crate::{output, repository};

/// The arguments of `cortex note`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The rule, at most 240 characters.
    text: String,
    /// What the rule is about: a file, or a file, `:` and a symbol it defines, or the repository's
    /// root folder (`.` from it) for the whole repository. Repeat for each.
    #[arg(long = "at", required = true, value_name = "PATH[:SYMBOL]")]
    at: Vec<String>,
    /// What kind of rule it is: convention, decision, invariant, pitfall or ownership.
    #[arg(long, default_value = "convention", value_parser = kind)]
    kind: FactKind,
    /// The repository's root folder; the one the current folder is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Notes the fact and prints what became of it.
///
/// # Errors
///
/// A root that is not a folder, an anchor that is not a file inside the repository, a daemon that
/// cannot be started or does not answer, a note it refused (a file it does not index, a symbol the
/// file does not define, a text too long), or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let at = args
        .at
        .iter()
        .map(|anchor| anchor_of(&root, anchor))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let note = Note {
        text: args.text.clone(),
        kind: args.kind,
        source: FactSource::User,
        at,
    };
    let answer = match answer_of(&root, &RequestBody::Note(note))? {
        ResponseBody::Facts(answer) => answer,
        other => bail!("the daemon answered the note with {other:?}"),
    };
    match answer {
        FactsAnswer::Noted { id, outcome } => output::line(&match outcome {
            NoteOutcome::Added => format!("fact {id} added"),
            NoteOutcome::Merged => format!("fact {id} says this already"),
            NoteOutcome::Activated => format!("fact {id} says this already, and is now confirmed"),
        }),
        other => bail!("the daemon answered the note with {other:?}"),
    }
}

/// The anchor `given` names, as the daemon reads it: `.` for the repository's root folder `root`,
/// or the repository path of a file inside it, and the symbol after it. `given` is a path from the
/// current folder, whole when it is a file or a folder, else up to its last `:`, which is followed
/// by the symbol.
///
/// # Errors
///
/// A folder other than the root, a path that is not a file, or a file outside the repository.
pub(crate) fn anchor_of(root: &Path, given: &str) -> anyhow::Result<String> {
    if Path::new(given).is_dir() {
        let full = std::fs::canonicalize(given).with_context(|| format!("finding {given}"))?;
        let path = RepoPath::from_native(root, &full)
            .with_context(|| format!("{given} is not inside the repository"))?;
        if !path.is_root() {
            bail!(
                "{given}: a fact is about a file, a symbol in one, or the whole repository (its \
                 root folder), and this is another folder"
            );
        }
        return Ok(".".to_owned());
    }
    match file_target(root, given) {
        Some(anchor) => anchor,
        None => bail!("{given}: a fact is about a file, or a symbol in one, and this is neither"),
    }
}

/// `given` as the daemon reads it when it names a file inside the repository at `root`: the
/// file's repository path, and what follows the file's name after a `:` (a symbol or a line). The
/// file is a path from the current folder, `given` whole when it is a file, else up to its last
/// `:`. None when `given` names no file that way.
///
/// # Errors
///
/// Inside the option: a file outside the repository, or one that cannot be found again.
pub(crate) fn file_target(root: &Path, given: &str) -> Option<anyhow::Result<String>> {
    let (file, rest) = match given.rsplit_once(':') {
        _ if Path::new(given).is_file() => (given, None),
        Some((file, rest)) if !rest.is_empty() && Path::new(file).is_file() => (file, Some(rest)),
        _ => return None,
    };
    let in_repository = || -> anyhow::Result<String> {
        let full = std::fs::canonicalize(file).with_context(|| format!("finding {file}"))?;
        let path = RepoPath::from_native(root, &full)
            .with_context(|| format!("{file} is not inside the repository"))?;
        Ok(match rest {
            Some(rest) => format!("{}:{rest}", path.as_str()),
            None => path.as_str().to_owned(),
        })
    };
    Some(in_repository())
}

/// A fact kind from its name.
fn kind(name: &str) -> Result<FactKind, String> {
    FactKind::from_name(name).ok_or_else(|| {
        format!(
            "{name} is not a kind of fact: convention, decision, invariant, pitfall or ownership"
        )
    })
}
