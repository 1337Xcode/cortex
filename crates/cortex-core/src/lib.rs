//! Shared types for every Cortex crate.
//!
//! The types here carry the rules that hold everywhere: a [`RepoPath`] is always relative to the
//! repository root with `/` separators, hashes are BLAKE3 at the sizes storage uses, a
//! [`Language`] has one stable name, an [`Identifier`] is the only text besides paths that the
//! index stores, each record kind such as [`SymbolKind`] has a fixed stored name or number, and
//! [`Config`] holds every setting with its default, each key read and shown through one registry
//! entry ([`config_keys`]), [`FEATURES`] lists what Cortex gives agents and the key that switches
//! each, [`identifier_tokens`] splits names and paths into the words a search matches, and
//! [`estimate_tokens`] sizes the text Cortex injects. Its only I/O is what every process needs
//! alike: canonicalizing a repository's folder, reading where git keeps a working tree's
//! repository, looking at which folders above a working folder hold a `.git` or a `.cortex/off`
//! ([`adoption_candidate`]), creating folders and files only their user can read, reading a small text file
//! whole, and finding and running another program, bounded in time and output.

#![warn(clippy::pedantic)]

mod activity;
mod adoption;
mod agent_shell;
mod child;
mod config;
mod coordination;
mod estimate;
mod event;
mod fact;
mod feature;
mod file_content;
mod git_layout;
mod hash;
mod identifier;
mod ignore_block;
mod injection;
pub mod ipc;
mod jaccard;
mod kinds;
mod language;
mod log_rotation;
mod named;
mod notice_subject;
mod object_id;
mod one_line;
mod path;
mod plain;
mod private;
mod program;
mod reminder_tag;
mod repo_id;
mod root;
mod scale_mode;
mod set_up;
mod task;
mod text_file;
mod tokens;
mod ulid;
mod unix_time;
mod user_folder;
mod warning;

pub use activity::{
    CommandKind, EventKind, EventMeta, EventSource, PROMPT_EXCERPT_CHARS, PromptExcerpt,
    SessionStatus, TaskOutcome,
};
pub use adoption::{
    ADOPTED_FILE, Candidate, OFF_FILE, Places, Refusal, adoption_candidate, is_turned_off,
    working_tree_top,
};
pub use agent_shell::{AGENT_SHELL_MARKERS, is_agent_shell};
pub use child::{Captured, capture, time_fed};
pub use config::{
    AdoptConfig, CompactionConfig, Config, ConfigError, ConfigKey, ConfigLayer, CoordinationConfig,
    DEFAULT_HOLDOUT_SHARE, DEFAULT_IDLE_SHUTDOWN_MINUTES, DEFAULT_MAX_FILE_BYTES,
    DEFAULT_MMAP_BYTES, DEFAULT_PARSE_TIMEOUT_MS, HintsConfig, HistoryConfig, LayerKind,
    LoadedConfig, MAX_MMAP_BYTES, MAX_WORK_ORDER_TOKENS, ModeSetting, PROFILE_KEY, Profile,
    RetrievalConfig, SWITCH_KEYS, SetError, Setting, SettingSource, SwitchKey, TypeWeights,
    UiConfig, WarningsConfig, config_key, config_keys, keys_set as config_keys_set,
    load as load_config, setting_value, switch_key, with_setting,
};
pub use coordination::{PositionMode, WorkingTree};
pub use estimate::estimate_tokens;
pub use event::{
    Agent, CheckCounts, CompactTrigger, Event, FailedOutput, FailedOutputError, HookKind,
    LineRange, LineRangeError, MAX_COMMAND_CHARS, MAX_EDITED_FILES, MAX_FAILED_OUTPUT_BYTES,
    MAX_HITS, MAX_HITS_BYTES, MAX_RANGES, MIN_WORK_PROMPT_CHARS, ShellCommand, ShellCommandError,
    StartSource, ToolKind, ToolResult, ToolTarget,
};
pub use fact::{FactKind, FactSource, FactStatus, MAX_FACT_ANCHORS, MAX_FACT_CHARS};
pub use feature::{FEATURES, Feature, FeatureState, feature};
pub use file_content::FileContent;
pub use git_layout::{GitLayout, MAX_GIT_FILE_BYTES, git_layout};
pub use hash::{ContentHash, ShortHash};
pub use identifier::{Identifier, IdentifierError, MAX_IDENTIFIER_BYTES};
pub use ignore_block::{
    IGNORE_ALL_BUT_SHARED, IGNORE_BLOCK_HEADER, IGNORE_CORTEX_FOLDER, IGNORE_LOCAL_FILES,
    IGNORE_LOCAL_SETTINGS, without_ignore_block,
};
pub use injection::{GateFeatures, ReasonKind};
pub use jaccard::jaccard;
pub use kinds::{EdgeType, FileFlags, InterestTier, NodeKind, RefKind, Resolution, SymbolKind};
pub use language::{Language, Tier};
pub use log_rotation::rotate_log;
pub use notice_subject::NoticeSubject;
pub use object_id::is_object_id;
pub use one_line::{is_hidden, one_line};
pub use path::{PathError, RepoPath};
pub use plain::{is_plain, is_taken_out};
pub use private::{
    append_private_file, create_new_private_file, create_private_folder, open_private_file,
};
pub use program::{GIT, find_program};
pub use reminder_tag::{REMINDER_TAGS, holds_reminder_tag};
pub use repo_id::RepoId;
pub use root::{canonical_root, is_never_a_root};
pub use scale_mode::ScaleMode;
pub use set_up::{forget_set_up, is_set_up_here, record_set_up};
pub use task::{is_held_out, task_id_of_prompt};
pub use text_file::{MAX_SETTINGS_BYTES, TextFileError, read_text_file};
pub use tokens::identifier_tokens;
pub use ulid::{Ulid, UlidError};
pub use unix_time::unix_millis;
pub use user_folder::{Endpoint, RunFiles, UserFolder};
pub use warning::WarningKind;
