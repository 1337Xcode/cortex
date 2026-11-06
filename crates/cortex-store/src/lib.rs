//! Storage for one repository: a SQLite database in the repository's `.cortex` folder.
//!
//! [`Writer::open`] takes a lock that makes it the database's only writer, opens the database
//! with every setting checked, and brings the schema up to date after backing up the old one.
//! Changes go through a [`WriteTx`], one transaction at a time. [`Reader::open`] opens the same
//! database read-only, beside the writer. Both read through [`Queries`].
//!
//! The index tables take no text but repository paths, names of one line and the fixed names of
//! kinds: the records a writer accepts have no field for code, comments, prompts or tool output.

#![warn(clippy::pedantic)]

mod activity;
mod archetype;
mod checkpoint;
mod cochange;
mod connection;
mod episode;
mod error;
mod folders;
mod graph;
mod ids;
mod injections;
mod interest;
mod layout;
mod links;
mod listing;
mod lock;
mod matching;
mod meta;
mod migrate;
mod nodes;
mod passes_on;
mod pause;
mod queries;
mod raw;
mod reader;
mod records;
mod replace;
mod rows;
mod schema;
mod search;
mod self_check;
mod usage;
mod values;
mod warnings;
mod writer;

pub use activity::{EventRecord, EventRow, SessionRecord, SessionRow, TaskRecord, TaskRow};
pub use archetype::ArchetypeRecord;
pub use checkpoint::CheckpointFailures;
pub use cochange::{CountedFile, FileCommitsRow, GitStateKey, PairRow};
pub use episode::{
    ChangedFile, ChangedSymbol, EpisodeRecord, MAX_EPISODE_BYTES, Pitfalls, RanCheck, ReadAgain,
    ReadFile,
};
pub use error::StoreError;
pub use graph::GraphNodeRow;
pub use ids::{FileId, InjectionId, NodeId, RefId, SymbolId, WarningId};
pub use injections::{InjectedFile, InjectionRecord, InjectionRow};
pub use interest::InterestRow;
pub use lock::is_writer_running;
pub use meta::MetaKey;
pub use pause::{PAUSE_EVERY, Pacer, Pause, in_turns, wait_for};
pub use queries::Queries;
pub use reader::Reader;
pub use records::{
    ChangeKind, EdgeRecord, FileChange, FileIndex, FileRecord, FileRemoval, PassedOn, RefRecord,
    Resolved, SymbolChange, SymbolRecord, Target,
};
pub use rows::{EdgeRow, FileRow, IndexCounts, RefName, RefRow, SymbolName, SymbolRow, SymbolSpan};
pub use self_check::SelfCheckRecord;
pub use usage::UsageRow;
pub use warnings::{WarningRecord, WarningRow};
pub use writer::{WriteTx, Writer, WriterOptions};
