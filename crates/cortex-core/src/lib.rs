//! Shared types for every Cortex crate.
//!
//! The types here carry the rules that hold everywhere: a [`RepoPath`] is always relative to the
//! repository root with `/` separators, hashes are BLAKE3 at the sizes storage uses, a
//! [`Language`] has one stable name, an [`Identifier`] is the only text besides paths that the
//! index stores, each record kind such as [`SymbolKind`] has a fixed stored name or number, and
//! [`Config`] holds every setting with its default, and [`identifier_tokens`] splits names and
//! paths into the words a search matches. This crate does no I/O and depends on nothing that
//! does.

#![warn(clippy::pedantic)]

mod config;
mod hash;
mod identifier;
mod kinds;
mod language;
mod path;
mod repo_id;
mod root;
mod scale_mode;
mod tokens;

pub use config::{
    Config, ConfigError, ConfigLayer, DEFAULT_IDLE_SHUTDOWN_MINUTES, DEFAULT_MAX_FILE_BYTES,
    DEFAULT_MMAP_BYTES, DEFAULT_PARSE_TIMEOUT_MS, LoadedConfig, MAX_MMAP_BYTES, ModeSetting,
    load as load_config,
};
pub use hash::{ContentHash, ShortHash};
pub use identifier::{Identifier, IdentifierError, MAX_IDENTIFIER_BYTES};
pub use kinds::{EdgeType, FileFlags, NodeKind, RefKind, Resolution, SymbolKind};
pub use language::{Language, Tier};
pub use path::{PathError, RepoPath};
pub use repo_id::RepoId;
pub use root::canonical_root;
pub use scale_mode::ScaleMode;
pub use tokens::identifier_tokens;
