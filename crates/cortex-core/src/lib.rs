//! Shared types for every Cortex crate.
//!
//! The types here carry the rules that hold everywhere: a [`RepoPath`] is always relative to the
//! repository root with `/` separators, hashes are BLAKE3 at the sizes storage uses, a
//! [`Language`] has one stable name, an [`Identifier`] is the only text besides paths that the
//! index stores, each record kind such as [`SymbolKind`] has a fixed stored name or number, and
//! [`Config`] holds every setting with its default, and [`identifier_tokens`] splits names and
//! paths into the words a search matches. Its only I/O is what every process needs alike:
//! canonicalizing a repository's folder, creating folders and files only their user can read,
//! reading a small text file whole, and finding and running another program, bounded in time and
//! output.

#![warn(clippy::pedantic)]

mod activity;
mod child;
mod config;
mod event;
mod file_content;
mod hash;
mod identifier;
pub mod ipc;
mod kinds;
mod language;
mod log_rotation;
mod named;
mod one_line;
mod path;
mod private;
mod program;
mod repo_id;
mod root;
mod scale_mode;
mod text_file;
mod tokens;
mod ulid;
mod unix_time;
mod user_folder;

pub use activity::{
    CommandKind, EventKind, EventMeta, EventSource, PROMPT_EXCERPT_CHARS, PromptExcerpt,
    SessionStatus, TaskOutcome,
};
pub use child::{Captured, capture, time_fed};
pub use config::{
    Config, ConfigError, ConfigLayer, DEFAULT_HOLDOUT_SHARE, DEFAULT_IDLE_SHUTDOWN_MINUTES,
    DEFAULT_MAX_FILE_BYTES, DEFAULT_MMAP_BYTES, DEFAULT_PARSE_TIMEOUT_MS, LoadedConfig,
    MAX_MMAP_BYTES, ModeSetting, keys_set as config_keys_set, load as load_config,
};
pub use event::{
    Agent, CompactTrigger, Event, HookKind, LineRange, LineRangeError, MAX_COMMAND_CHARS, MAX_HITS,
    MAX_HITS_BYTES, MAX_RANGES, ShellCommand, ShellCommandError, StartSource, ToolKind, ToolResult,
    ToolTarget,
};
pub use file_content::FileContent;
pub use hash::{ContentHash, ShortHash};
pub use identifier::{Identifier, IdentifierError, MAX_IDENTIFIER_BYTES};
pub use kinds::{EdgeType, FileFlags, NodeKind, RefKind, Resolution, SymbolKind};
pub use language::{Language, Tier};
pub use log_rotation::rotate_log;
pub use one_line::one_line;
pub use path::{PathError, RepoPath};
pub use private::{append_private_file, create_private_folder, open_private_file};
pub use program::{GIT, find_program};
pub use repo_id::RepoId;
pub use root::canonical_root;
pub use scale_mode::ScaleMode;
pub use text_file::{MAX_SETTINGS_BYTES, TextFileError, read_text_file};
pub use tokens::identifier_tokens;
pub use ulid::{Ulid, UlidError};
pub use unix_time::unix_millis;
pub use user_folder::{Endpoint, RunFiles, UserFolder};
