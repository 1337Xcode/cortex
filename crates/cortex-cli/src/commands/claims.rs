//! `cortex claims list|add|release`: advisory claims on files. A claim tells the other sessions
//! working in the repository that a session works on a file, so that an edit of it by another is
//! noticed to both; it locks nothing. A claim is held by the agent session at work when it is
//! made (the one whose latest tool call came last within the five minutes before), so `add` is run
//! from the agent's own shell. Anyone may release any claim. The daemon, started when none runs,
//! keeps them.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::bail;
use cortex_core::ipc::{ClaimLine, ClaimsAnswer, ClaimsAsk, RequestBody, ResponseBody};
use cortex_core::unix_millis;

use crate::commands::facts::answer_of;
use crate::commands::note::file_target;
use crate::commands::status::counted;
use crate::{output, repository};

/// The arguments of `cortex claims`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// What to do.
    #[command(subcommand)]
    action: Action,
    /// The repository's root folder; the one the current folder is in when left out.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
}

/// What `cortex claims` does.
#[derive(Debug, clap::Subcommand)]
enum Action {
    /// List the claims holding now.
    List,
    /// Claim files for the agent session at work, for two hours unless the settings say otherwise.
    Add {
        /// The files, from the current folder.
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Release the claims on files, whoever holds them; every claim when no file is named.
    Release {
        /// The files, from the current folder.
        files: Vec<String>,
    },
}

/// Lists, makes or releases claims and prints the outcome.
///
/// # Errors
///
/// A root that is not a folder, a name that is not a file inside the repository, no home folder,
/// a daemon that cannot be started or does not answer, a request it refused (no session at work, a
/// file it does not index), or stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let (root, worktree) = repository::root_and_worktree(args.root.as_deref())?;
    let files_of = |given: &[String]| -> anyhow::Result<Vec<String>> {
        given
            .iter()
            .map(|file| match file_target(&worktree, file) {
                // A claim is on a whole file, never on a line or a symbol of one.
                Some(found) if Path::new(file).is_file() => found,
                _ => bail!("{file}: a claim is on a file, and this is none"),
            })
            .collect()
    };
    let ask = match &args.action {
        Action::List => ClaimsAsk::List,
        Action::Add { files } => ClaimsAsk::Add {
            at: files_of(files)?,
        },
        Action::Release { files } => ClaimsAsk::Release {
            at: files_of(files)?,
        },
    };
    let answer = match answer_of(&root, &RequestBody::Claims(ask))? {
        ResponseBody::Claims(answer) => answer,
        other => bail!("the daemon answered with {other:?}"),
    };
    match answer {
        ClaimsAnswer::Listed { claims } if claims.is_empty() => output::line("no claims"),
        ClaimsAnswer::Listed { claims } => {
            let now_ms = unix_millis(SystemTime::now());
            claims
                .iter()
                .try_for_each(|claim| output::line(&line_of(claim, now_ms)))
        }
        ClaimsAnswer::Added {
            agent,
            session,
            files,
        } => output::line(&format!(
            "claimed {} for the {agent} session {}",
            counted(u64::from(files), ("file", "files")),
            cortex_core::shown_session(&session)
        )),
        ClaimsAnswer::Released { count: 0 } => output::line("no claims to release"),
        ClaimsAnswer::Released { count } => {
            output::line(&format!("released {}", counted(count, ("claim", "claims"))))
        }
    }
}

/// One claim as the list prints it, `now_ms` saying how long ago it was made.
fn line_of(claim: &ClaimLine, now_ms: i64) -> String {
    format!(
        "{}  {} session {}, claimed {}",
        claim.path,
        claim.agent,
        cortex_core::shown_session(&claim.session),
        cortex_coord::ago(now_ms - claim.since_ms)
    )
}
