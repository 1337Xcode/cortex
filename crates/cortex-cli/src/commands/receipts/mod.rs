//! `cortex receipts`: what each task of the repository's recent sessions took, in turns, tool
//! calls, tokens and estimated cost, and the tokens Cortex added to it; how precise the work orders
//! Cortex gave were; and how far the tasks held out are from being enough to compare with the
//! rest. The store is only read. The user's price table is written from the one
//! this build ships the first time it is needed, and read from then on, so that edits to it win.

mod cache;
mod text;

pub(super) use self::text::{grouped, quantity};

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, bail};
use cortex_core::{Identifier, UserFolder, create_private_folder, unix_millis};
use cortex_meter::{PriceTable, Recorded, Selection, load_or_write};
use cortex_store::{Reader, StoreError};
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
/// A session id that is no name, a session that cannot be shown, or a store that cannot be read.
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
    let selection = match &args.session {
        Some(id) => Selection::Session(
            Identifier::new(id).with_context(|| format!("reading the session id {id}"))?,
        ),
        None => {
            Selection::StartedSince(unix_millis(SystemTime::now()) - i64::from(args.days) * DAY_MS)
        }
    };
    Ok(Some(cortex_meter::recorded(
        reader.queries(),
        &selection,
        prices,
    )?))
}
