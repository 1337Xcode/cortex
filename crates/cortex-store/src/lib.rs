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

mod connection;
mod error;
mod folders;
mod ids;
mod layout;
mod links;
mod lock;
mod meta;
mod migrate;
mod private;
mod queries;
mod raw;
mod reader;
mod records;
mod replace;
mod rows;
mod schema;
mod values;
mod writer;

pub use error::StoreError;
pub use ids::{FileId, NodeId, RefId, SymbolId};
pub use meta::MetaKey;
pub use queries::Queries;
pub use reader::Reader;
pub use records::{
    ChangeKind, EdgeRecord, FileChange, FileIndex, FileRecord, FileRemoval, RefRecord, Resolved,
    SymbolChange, SymbolRecord, Target,
};
pub use rows::{EdgeRow, FileRow, IndexCounts, RefRow, SymbolRow};
pub use writer::{WriteTx, Writer, WriterOptions};
