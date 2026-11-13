//! The keys of the settings [`Config`] holds in fields of its own rather than in a section's
//! struct: the scale mode, the index, the daemon, the store, privacy, the holdout, learning, git
//! history, test commands and strict mode.

use super::registry::{integer, strings};

/// The most commits `git.max_commits` may name: far past any repository's history a miner could
/// read in its minutes, and a number every reader of the map holds exactly.
const MAX_GIT_COMMITS: u64 = 10_000_000;

/// The most days `git.max_age_days` may name: a hundred years, older than any git history.
const MAX_GIT_AGE_DAYS: u64 = 36_500;

/// The most files `git.max_files_per_commit` may name: past any commit worth reading for co-change.
const MAX_GIT_FILES_PER_COMMIT: u64 = 100_000;

/// What `git.max_commits` accepts, in its refusal.
const GIT_COMMITS: &str = "a whole number from 1 to 10000000";

/// What `git.max_age_days` accepts, in its refusal.
const GIT_AGE_DAYS: &str = "a whole number from 1 to 36500";

/// What `git.max_files_per_commit` accepts, in its refusal.
const GIT_FILES_PER_COMMIT: &str = "a whole number from 1 to 100000";

/// The value as a whole number from 1 to `max`.
fn at_most(value: &toml::Value, max: u64) -> Option<u64> {
    positive_integer(value).filter(|number| *number <= max)
}
use super::{
    ABOVE_ZERO, ConfigKey, FACTOR, MAX_MMAP_BYTES, ModeSetting, SHARE, SWITCH, factor, is_regex,
    mode_setting, positive_integer, share, string_list, whole_number,
};

/// The keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[
    ConfigKey::new(
        "mode",
        "The scale mode: auto to choose it from the repository's size, or tiny, small, medium or \
         huge to pin one.",
        |config, value| {
            config.mode = value
                .as_str()
                .and_then(mode_setting)
                .ok_or("one of auto, tiny, small, medium, huge")?;
            Ok(())
        },
        |config| {
            let word = match config.mode {
                ModeSetting::Auto => "auto",
                ModeSetting::Pinned(mode) => mode.name(),
            };
            toml::Value::String(word.to_owned())
        },
    ),
    ConfigKey::new(
        "index.max_file_bytes",
        "Files larger than this many bytes are recorded but never read.",
        |config, value| {
            config.max_file_bytes = positive_integer(value).ok_or(ABOVE_ZERO)?;
            Ok(())
        },
        |config| integer(config.max_file_bytes),
    ),
    ConfigKey::new(
        "index.extra_ignores",
        "Patterns in .gitignore syntax that the index also leaves out.",
        |config, value| {
            config.extra_ignores = string_list(value).ok_or("a list of strings")?;
            Ok(())
        },
        |config| strings(&config.extra_ignores),
    ),
    ConfigKey::new(
        "index.parse_timeout_ms",
        "How long parsing one file may take before it is indexed at file level only.",
        |config, value| {
            config.parse_timeout_ms = positive_integer(value).ok_or(ABOVE_ZERO)?;
            Ok(())
        },
        |config| integer(config.parse_timeout_ms),
    ),
    ConfigKey::new(
        "index.crawl_files_per_second",
        "How many files the background crawler parses a second in huge mode.",
        |config, value| {
            config.crawl_files_per_second = positive_integer(value).ok_or(ABOVE_ZERO)?;
            Ok(())
        },
        |config| integer(config.crawl_files_per_second),
    ),
    ConfigKey::new(
        "daemon.idle_shutdown_minutes",
        "The daemon stops after this many minutes without work.",
        |config, value| {
            config.idle_shutdown_minutes = positive_integer(value).ok_or(ABOVE_ZERO)?;
            Ok(())
        },
        |config| integer(config.idle_shutdown_minutes),
    ),
    ConfigKey::new(
        "store.mmap_bytes",
        "How much of the database SQLite maps into memory; 0 turns mapping off.",
        |config, value| {
            config.mmap_bytes = whole_number(value)
                .filter(|bytes| *bytes <= MAX_MMAP_BYTES)
                .ok_or("a whole number from 0 to 2147418112")?;
            Ok(())
        },
        |config| integer(config.mmap_bytes),
    ),
    ConfigKey::user_only(
        "privacy.store_prompt_excerpt",
        "Keep the first 200 characters of each prompt with its task, not only its hash. Read only \
         from the user's own file.",
        |config, value| {
            config.store_prompt_excerpt = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.store_prompt_excerpt),
    ),
    ConfigKey::new(
        "holdout.share",
        "The share of tasks given no injections, to measure Cortex's effect against; 0 turns the \
         holdout off.",
        |config, value| {
            config.holdout_share = share(value).ok_or(SHARE)?;
            Ok(())
        },
        |config| toml::Value::Float(config.holdout_share),
    ),
    ConfigKey::new(
        "learning.eta",
        "How much one successful task strengthens the link between two files it edited.",
        |config, value| {
            config.learning_eta = factor(value).ok_or(FACTOR)?;
            Ok(())
        },
        |config| toml::Value::Float(config.learning_eta),
    ),
    ConfigKey::new(
        "learning.link_decay",
        "What a link keeps of its weight for each task since it was last strengthened.",
        |config, value| {
            config.link_decay = factor(value).ok_or(FACTOR)?;
            Ok(())
        },
        |config| toml::Value::Float(config.link_decay),
    ),
    ConfigKey::new(
        "learning.interest_decay",
        "What a file or symbol keeps of its interest for each task since it was last touched.",
        |config, value| {
            config.interest_decay = factor(value).ok_or(FACTOR)?;
            Ok(())
        },
        |config| toml::Value::Float(config.interest_decay),
    ),
    ConfigKey::new(
        "git.max_commits",
        "The most commits of history the co-change miner reads.",
        |config, value| {
            config.git_max_commits = at_most(value, MAX_GIT_COMMITS).ok_or(GIT_COMMITS)?;
            Ok(())
        },
        |config| integer(config.git_max_commits),
    ),
    ConfigKey::new(
        "git.max_age_days",
        "The oldest history the co-change miner reads, in days.",
        |config, value| {
            config.git_max_age_days = at_most(value, MAX_GIT_AGE_DAYS).ok_or(GIT_AGE_DAYS)?;
            Ok(())
        },
        |config| integer(config.git_max_age_days),
    ),
    ConfigKey::new(
        "git.max_files_per_commit",
        "Commits touching more files than this are skipped by the co-change miner.",
        |config, value| {
            config.git_max_files_per_commit =
                at_most(value, MAX_GIT_FILES_PER_COMMIT).ok_or(GIT_FILES_PER_COMMIT)?;
            Ok(())
        },
        |config| integer(config.git_max_files_per_commit),
    ),
    ConfigKey::new(
        "git.half_life_commits",
        "A commit this many commits older than the newest counts half toward co-change.",
        |config, value| {
            config.git_half_life_commits = positive_integer(value).ok_or(ABOVE_ZERO)?;
            Ok(())
        },
        |config| integer(config.git_half_life_commits),
    ),
    ConfigKey::new(
        "tests.extra_patterns",
        "Regular expressions naming further test or build commands.",
        |config, value| {
            config.test_extra_patterns = string_list(value)
                .filter(|patterns| patterns.iter().all(|pattern| is_regex(pattern)))
                .ok_or("a list of regular expressions")?;
            Ok(())
        },
        |config| strings(&config.test_extra_patterns),
    ),
    ConfigKey::new(
        "strict_mode",
        "When a task's agent stops with call sites of a signature it changed still unchanged, ask \
         it once to go on.",
        |config, value| {
            config.is_strict_mode = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.is_strict_mode),
    ),
];
