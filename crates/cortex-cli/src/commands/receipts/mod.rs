//! `cortex receipts`: what each task of the repository's recent sessions took, in turns, tool
//! calls, tokens and estimated cost, and how far the tasks held out are from being enough to
//! compare with the rest. The store is only read. The user's price table is written from the one
//! this build ships the first time it is needed, and read from then on, so that edits to it win.

mod text;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, bail};
use cortex_core::{Identifier, Ulid, UserFolder, create_private_folder, unix_millis};
use cortex_meter::{
    Activity, Arms, MIN_TASKS_PER_ARM, PriceTable, SessionReceipt, load_or_write, receipt,
};
use cortex_store::{Queries, Reader, SessionRow, StoreError};
use serde::Serialize;

use crate::{output, repository};

/// Milliseconds in a day.
const DAY_MS: i64 = 86_400_000;

/// The arguments of `cortex receipts`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Only this session, whenever it started.
    #[arg(long)]
    session: Option<String>,
    /// The sessions that started in this many days before now.
    #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u32).range(1..=36_500))]
    days: u32,
    /// Print the receipts as JSON.
    #[arg(long)]
    json: bool,
}

/// The receipts and what they were made with, as `--json` prints them.
#[derive(Debug, Serialize)]
struct Report {
    /// The price table the costs were estimated from, or why there is none.
    prices: Prices,
    /// What the repository's store holds, or `None` when it has no store yet.
    recorded: Option<Recorded>,
}

/// The price table costs were estimated from.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Prices {
    /// The user's price table was read.
    Read {
        /// Its file.
        file: String,
        /// The day its prices were read, as `YYYY-MM-DD`.
        updated: String,
        /// Where they were read from, when it says.
        source: Option<String>,
    },
    /// There is no price table, so no cost is estimated.
    Missing {
        /// Why.
        why: String,
    },
}

/// What the store holds: the arms of the held-out tasks and the receipts of the sessions asked
/// for.
#[derive(Debug, Serialize)]
struct Recorded {
    /// How many of the repository's tasks were held out and how many were not.
    holdout: Holdout,
    /// Each session's receipt, by start.
    sessions: Vec<SessionReceipt>,
}

/// The tasks held out from Cortex and the rest, which receipts compare once both are enough.
#[derive(Debug, Serialize)]
struct Holdout {
    /// How many of the repository's tasks each arm holds, whenever they ran.
    #[serde(flatten)]
    arms: Arms,
    /// How many each arm needs before they are compared.
    needed_per_arm: u64,
}

/// Prints the receipts of the sessions `args` selects, as text or as JSON.
///
/// # Errors
///
/// A root that is not a folder; a session id that is no name, is not in the store, names a
/// subagent, or is asked of a repository with no store; a store that cannot be read; or stdout
/// that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let (prices, table) = match price_table() {
        Ok((path, table)) => (
            Prices::Read {
                file: path.display().to_string(),
                updated: table.updated().to_owned(),
                source: table.source().map(str::to_owned),
            },
            Some(table),
        ),
        Err(why) => (Prices::Missing { why }, None),
    };
    let report = Report {
        prices,
        recorded: recorded(&root, args, table.as_ref())?,
    };
    if args.json {
        let json = serde_json::to_string_pretty(&report).context("writing the receipts as JSON")?;
        // JSON escapes every control character inside its strings, so its own line breaks are
        // the only ones, and each line is printed as it is.
        for line in json.lines() {
            output::line(line)?;
        }
        return Ok(());
    }
    let days = args.session.is_none().then_some(args.days);
    for line in text::lines(&report, &root, days) {
        output::line(&line)?;
    }
    Ok(())
}

/// The user's price table and its file, written from the shipped one when the user has none, or
/// why there is none.
fn price_table() -> Result<(PathBuf, PriceTable), String> {
    let user = UserFolder::of_current_user()
        .ok_or_else(|| "there is no home folder to keep a price table in".to_owned())?;
    create_private_folder(user.path())
        .map_err(|error| format!("creating {}: {error}", user.path().display()))?;
    let path = user.pricing_path();
    let table = load_or_write(&path).map_err(|error| error.to_string())?;
    Ok((path, table))
}

/// What the store of the repository at `root` holds for `args`: the arms of its held-out tasks,
/// and the receipt of each session it selects, priced from `prices`. `None` when the repository
/// has no store yet and no session was named.
///
/// # Errors
///
/// A session that cannot be shown, or a store that cannot be read.
fn recorded(
    root: &Path,
    args: &Args,
    prices: Option<&PriceTable>,
) -> anyhow::Result<Option<Recorded>> {
    let folder = root.join(".cortex");
    let reader = match Reader::open(&folder, 0) {
        Ok(reader) => reader,
        Err(StoreError::Missing { .. }) => {
            if let Some(id) = &args.session {
                bail!("no session {id}: {} has no store yet", root.display());
            }
            return Ok(None);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading the store in {}", folder.display()));
        }
    };
    let queries = reader.queries();
    let (held_out, treated) = queries
        .holdout_arms()
        .context("counting the held-out tasks")?;
    let sessions = sessions(queries, args)?
        .into_iter()
        .map(|session| Ok(receipt(&activity(queries, session)?, prices)))
        .collect::<anyhow::Result<_>>()?;
    Ok(Some(Recorded {
        holdout: Holdout {
            arms: Arms { held_out, treated },
            needed_per_arm: MIN_TASKS_PER_ARM,
        },
        sessions,
    }))
}

/// The sessions `args` selects: the one it names, or each that started in the days it gives. A
/// subagent's work is its parent session's, so a subagent is never one of them.
///
/// # Errors
///
/// A session id that is no name, is not in the store or names a subagent, or a store that cannot
/// be read.
fn sessions(queries: Queries<'_>, args: &Args) -> anyhow::Result<Vec<SessionRow>> {
    let Some(id) = &args.session else {
        let since_ms = unix_millis(SystemTime::now()) - i64::from(args.days) * DAY_MS;
        let started = queries
            .sessions_started_since(since_ms)
            .context("reading the sessions")?;
        return Ok(started
            .into_iter()
            .filter(|session| session.record.parent_session_id.is_none())
            .collect());
    };
    let id = Identifier::new(id).with_context(|| format!("reading the session id {id}"))?;
    let session = queries.session(&id).context("reading the session")?;
    let session = session.with_context(|| format!("no session {} in the store", id.as_str()))?;
    if let Some(parent) = &session.record.parent_session_id {
        bail!(
            "session {} is a subagent of session {}, whose receipts count its work",
            id.as_str(),
            parent.as_str()
        );
    }
    Ok(vec![session])
}

/// What `session` did: its tasks, its responses and its subagents', each task's tool calls, and
/// the calls it and its subagents made while it had no task.
///
/// # Errors
///
/// The store cannot be read.
fn activity(queries: Queries<'_>, session: SessionRow) -> anyhow::Result<Activity> {
    let id = &session.record.session_id;
    let reading = || format!("reading the activity of session {}", id.as_str());
    let tasks = queries.tasks_of_session(id).with_context(reading)?;
    let subagents = queries.sessions_started_by(id).with_context(reading)?;
    let mut usage = queries.usage_of_session(id).with_context(reading)?;
    let mut outside_tool_calls = queries.tool_calls_outside_tasks(id).with_context(reading)?;
    for subagent in &subagents {
        let subagent = &subagent.record.session_id;
        usage.extend(queries.usage_of_session(subagent).with_context(reading)?);
        let calls = queries.tool_calls_outside_tasks(subagent);
        outside_tool_calls = outside_tool_calls.saturating_add(calls.with_context(reading)?);
    }
    let task_ids: Vec<Ulid> = tasks.iter().map(|task| task.record.task_id).collect();
    let tool_calls = queries
        .tool_calls_of_tasks(&task_ids)
        .with_context(reading)?;
    Ok(Activity {
        subagents: u64::try_from(subagents.len()).context("counting subagents")?,
        session,
        tasks,
        usage,
        tool_calls,
        outside_tool_calls,
    })
}
