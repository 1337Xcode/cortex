//! `cortex mcp`: the MCP server an agent starts over stdio. Its tools are answered by the daemon of
//! the repository the agent works in, found from the current folder at each call and started
//! when none runs, as `cortex where` does. Standard output carries the protocol alone.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use cortex_core::UserFolder;
use cortex_core::ipc::{RequestBody, ResponseBody};
use cortex_mcp::Daemon;

use crate::commands::note::{anchor_of, file_target};
use crate::{repository, upgrade};

/// How long the daemon has to answer one call, a daemon started by it included, as for
/// `cortex where`.
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The arguments of `cortex mcp`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder's repository when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// The daemon of the repository at `root`, or of the current folder's.
struct RepositoryDaemon {
    /// The repository's root folder as given, if it was.
    root: Option<PathBuf>,
}

impl Daemon for RepositoryDaemon {
    /// A note's anchors are read from the repository's root as `cortex note` reads them from its
    /// folder, and the note goes to one daemon only, since it changes what the daemon keeps.
    fn ask(&self, body: RequestBody) -> Result<ResponseBody, String> {
        let root = repository::root(self.root.as_deref()).map_err(|error| format!("{error:#}"))?;
        let user = UserFolder::of_current_user().ok_or("no home folder to find the daemon from")?;
        let deadline = Instant::now() + ANSWER_WITHIN;
        let asked = match body {
            RequestBody::Note(mut note) => {
                note.at = note
                    .at
                    .iter()
                    .map(|anchor| {
                        // An agent names files from the repository's root, whatever folder the
                        // server was started in; an absolute path stays as it is.
                        let joined = root.join(anchor);
                        let joined = joined.to_str().ok_or_else(|| {
                            anyhow::anyhow!("{anchor}: not a path Cortex can read")
                        })?;
                        anchor_of(&root, joined)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
                    .map_err(|error| format!("{error:#}"))?;
                upgrade::ask_once_started(&root, &user, &RequestBody::Note(note), deadline)
            }
            RequestBody::Why { target } => {
                // A file is named from the repository's root, as a note's anchor is; anything
                // else, a symbol's name, goes as it is.
                let joined = root.join(&target);
                let target = match joined
                    .to_str()
                    .and_then(|joined| file_target(&root, joined))
                {
                    Some(found) => found.map_err(|error| format!("{error:#}"))?,
                    None => target,
                };
                upgrade::ask_started(&root, &user, &RequestBody::Why { target }, deadline)
            }
            body => upgrade::ask_started(&root, &user, &body, deadline),
        };
        asked
            .answer
            .map(|answer| answer.body)
            .map_err(|error| format!("the daemon of {} did not answer: {error}", root.display()))
    }
}

/// Serves MCP on stdio until the agent ends the session.
///
/// # Errors
///
/// The server could not start, or the session ended with an error.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let daemon = RepositoryDaemon {
        root: args.root.clone(),
    };
    cortex_mcp::serve(Arc::new(daemon)).context("serving MCP on stdio")
}
