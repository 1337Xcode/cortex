//! Shared types for every Cortex crate.
//!
//! The types here carry the rules that hold everywhere: a [`RepoPath`] is always relative to the
//! repository root with `/` separators, hashes are BLAKE3 at the sizes storage uses, a
//! [`Language`] has one stable name, and an [`Identifier`] is the only text besides paths that the
//! index stores. This crate does no I/O and depends on nothing that does.

#![warn(clippy::pedantic)]

mod hash;
mod identifier;
mod language;
mod path;
mod repo_id;

pub use hash::{ContentHash, ShortHash};
pub use identifier::{Identifier, IdentifierError, MAX_IDENTIFIER_BYTES};
pub use language::{Language, Tier};
pub use path::{PathError, RepoPath};
pub use repo_id::RepoId;
