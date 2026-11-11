//! Configuration: the settings a user may change, their defaults, and how files override them.
//!
//! Cortex reads two files, `~/.cortex/config.toml` for the user and `.cortex/config.toml` for the
//! repository; the repository's wins over the user's, which wins over the defaults. The caller
//! reads the files and passes their text here, so this module does no I/O. A key Cortex does not
//! know produces a warning and is otherwise ignored. A known key with a value of the wrong type or
//! out of range is an error that names the file and the key: Cortex never guesses what a broken
//! setting meant.
//!
//! Every key is one entry of the registry ([`config_keys`]), which reads it and shows it; loading
//! a file looks each key up there. The `profile` key alone stands outside it. A registry entry
//! reads a value into a field of [`Config`] and shows that field; the profile is no setting
//! `Config` holds but the choice of the defaults every other key starts from, read before them
//! and over every layer. A field for it would hold a value the keys after it may contradict, and
//! entries over a wider type would make every other entry reach through it. So four places name it
//! instead: [`load`] reads it first, [`LoadedConfig::settings`] lists it, and [`with_setting`]
//! checks it and offers it as the nearest key.

mod adopt;
mod compaction;
mod coordination;
mod edit;
mod effective;
mod hints;
mod main_keys;
mod profile;
mod registry;
mod retrieval;
mod set;
mod switches;
mod warnings;

use std::collections::BTreeMap;

use thiserror::Error;

pub use self::adopt::AdoptConfig;
pub use self::compaction::CompactionConfig;
pub use self::coordination::CoordinationConfig;
pub use self::effective::{Setting, SettingSource};
pub use self::hints::HintsConfig;
pub use self::profile::{PROFILE_KEY, Profile};
pub use self::registry::{ConfigKey, config_key, config_keys};
pub use self::retrieval::{MAX_WORK_ORDER_TOKENS, RetrievalConfig, TypeWeights};
pub use self::set::{SetError, setting_value, with_setting};
pub use self::switches::{SWITCH_KEYS, SwitchKey, switch_key};
pub use self::warnings::WarningsConfig;
use crate::ScaleMode;

/// The default for `index.max_file_bytes`: one mebibyte. Larger files are almost always data or
/// generated code, and parsing them would cost more than they add.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1_048_576;

/// The default for `index.parse_timeout_ms`: one second. Parsing a 1 MiB Python file and reading
/// its definitions took about 0.2 s in a release build when this was set, so only a parse that
/// has gone wrong runs out of time.
pub const DEFAULT_PARSE_TIMEOUT_MS: u64 = 1000;

/// The default for `daemon.idle_shutdown_minutes`: half an hour without any work.
pub const DEFAULT_IDLE_SHUTDOWN_MINUTES: u64 = 30;

/// The default for `store.mmap_bytes`: 256 mebibytes. SQLite reads the mapped part of the
/// database straight from the operating system's file cache instead of copying it into its own.
pub const DEFAULT_MMAP_BYTES: u64 = 268_435_456;

/// The largest `store.mmap_bytes`: the most the bundled SQLite maps (its `SQLITE_MAX_MMAP_SIZE`,
/// 0x7fff0000 bytes). Asked for more, SQLite maps that much, and the store, which reads each setting
/// back, would refuse to open.
pub const MAX_MMAP_BYTES: u64 = 2_147_418_112;

/// The default for `index.crawl_files_per_second`: how many files the background crawler parses
/// a second in huge mode, on one thread. A file parses in a few milliseconds, so this keeps the
/// crawler to a fraction of one core; 100,000 files take under 20 minutes.
pub const DEFAULT_CRAWL_FILES_PER_SECOND: u64 = 100;

/// The default for `holdout.share`: one task in ten goes without injections, so that Cortex's
/// effect can be compared with tasks it did not help.
pub const DEFAULT_HOLDOUT_SHARE: f64 = 0.10;

/// The default for `learning.eta`: how much one successful task strengthens the link between two
/// files it edited, at full role weight.
pub const DEFAULT_LEARNING_ETA: f64 = 0.1;

/// The default for `learning.link_decay`: what a link keeps of its weight for each task in the
/// repository since it was last strengthened.
pub const DEFAULT_LINK_DECAY: f64 = 0.995;

/// The default for `learning.interest_decay`: what a node keeps of its interest for each task in
/// the repository since it was last touched (a half-life of about 34 tasks).
pub const DEFAULT_INTEREST_DECAY: f64 = 0.98;

/// The default for `git.max_commits`: the most commits of history the co-change miner reads.
pub const DEFAULT_GIT_MAX_COMMITS: u64 = 20_000;

/// The default for `git.max_age_days`: the oldest history the co-change miner reads, two years.
pub const DEFAULT_GIT_MAX_AGE_DAYS: u64 = 730;

/// The default for `git.max_files_per_commit`: commits touching more files than this (mass
/// refactors, formatting) say nothing about which files belong together and are skipped.
pub const DEFAULT_GIT_MAX_FILES_PER_COMMIT: u64 = 30;

/// The default for `git.half_life_commits`: a commit this many commits older than the newest
/// counts half as much toward co-change.
pub const DEFAULT_GIT_HALF_LIFE_COMMITS: u64 = 500;

/// What a key taking a whole number above 0 accepts, in its refusal.
const ABOVE_ZERO: &str = "a whole number above 0";

/// What a key taking true or false accepts, in its refusal.
const SWITCH: &str = "true or false";

/// What a key taking a share accepts, in its refusal.
const SHARE: &str = "a number from 0 to 1";

/// What a key taking a factor accepts, in its refusal.
const FACTOR: &str = "a number above 0 and at most 1";

/// The scale mode setting: chosen from the repository's size, or pinned by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSetting {
    /// Choose from the repository's size (`auto`, the default).
    Auto,
    /// Always use this mode.
    Pinned(ScaleMode),
}

/// The settings in effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `mode`: `auto`, or `tiny`, `small`, `medium` or `huge` to pin one. Default `auto`.
    pub mode: ModeSetting,
    /// `index.max_file_bytes`: files larger than this are recorded but never read. Default
    /// [`DEFAULT_MAX_FILE_BYTES`].
    pub max_file_bytes: u64,
    /// `index.extra_ignores`: patterns in `.gitignore` syntax that the index also leaves out.
    /// Default none.
    pub extra_ignores: Vec<String>,
    /// `index.parse_timeout_ms`: how long parsing one file and reading what it defines may take;
    /// a file that takes longer is indexed at file level only. Default
    /// [`DEFAULT_PARSE_TIMEOUT_MS`].
    pub parse_timeout_ms: u64,
    /// `daemon.idle_shutdown_minutes`: the daemon stops after this many minutes without work.
    /// Default [`DEFAULT_IDLE_SHUTDOWN_MINUTES`].
    pub idle_shutdown_minutes: u64,
    /// `store.mmap_bytes`: how much of the database SQLite maps into memory; 0 turns mapping off.
    /// Default [`DEFAULT_MMAP_BYTES`].
    pub mmap_bytes: u64,
    /// `index.crawl_files_per_second`: how many files the background crawler parses a second in
    /// huge mode. Default [`DEFAULT_CRAWL_FILES_PER_SECOND`].
    pub crawl_files_per_second: u64,
    /// `privacy.store_prompt_excerpt`: keep the first 200 characters of each prompt with its task.
    /// Default `false`: a prompt is kept only as its hash.
    pub store_prompt_excerpt: bool,
    /// `holdout.share`: the share of tasks, from 0 to 1, that get no injections so that Cortex's
    /// effect can be measured against them; 0 turns the holdout off. Default
    /// [`DEFAULT_HOLDOUT_SHARE`].
    pub holdout_share: f64,
    /// `learning.eta`: the link learning rate, above 0 and at most 1. Default
    /// [`DEFAULT_LEARNING_ETA`].
    pub learning_eta: f64,
    /// `learning.link_decay`: what a link keeps of its weight per task, above 0 and at most 1.
    /// Default [`DEFAULT_LINK_DECAY`].
    pub link_decay: f64,
    /// `learning.interest_decay`: what a node keeps of its interest per task, above 0 and at most 1.
    /// Default [`DEFAULT_INTEREST_DECAY`].
    pub interest_decay: f64,
    /// `git.max_commits`: the most commits of history mined. Default [`DEFAULT_GIT_MAX_COMMITS`].
    pub git_max_commits: u64,
    /// `git.max_age_days`: the oldest history mined, in days. Default [`DEFAULT_GIT_MAX_AGE_DAYS`].
    pub git_max_age_days: u64,
    /// `git.max_files_per_commit`: commits touching more files are skipped. Default
    /// [`DEFAULT_GIT_MAX_FILES_PER_COMMIT`].
    pub git_max_files_per_commit: u64,
    /// `git.half_life_commits`: the age, in commits, at which a commit counts half. Default
    /// [`DEFAULT_GIT_HALF_LIFE_COMMITS`].
    pub git_half_life_commits: u64,
    /// `tests.extra_patterns`: regular expressions naming further test or build commands, matched
    /// against a shell command's text. Default none.
    pub test_extra_patterns: Vec<String>,
    /// The `retrieval.*` keys: how much a work order holds and how activation spreads.
    pub retrieval: RetrievalConfig,
    /// The `warnings.*` keys: how many warnings an edit brings and when a co-change partner counts.
    pub warnings: WarningsConfig,
    /// The `coordination.*` keys: how long positions hold and when an edit concerns another
    /// session.
    pub coordination: CoordinationConfig,
    /// `strict_mode`: when a task's agent stops while call sites of a signature it changed are still
    /// unchanged, ask it once to go on. Default `false`: Cortex never holds a stop.
    pub is_strict_mode: bool,
    /// The `hints.*` keys: which texts about a check's run are on.
    pub hints: HintsConfig,
    /// The `compaction.*` keys: what a compaction is given.
    pub compaction: CompactionConfig,
    /// The `adopt.*` keys: which folders Cortex sets up on its own.
    pub adopt: AdoptConfig,
}

impl Default for Config {
    /// Every setting at its documented default.
    fn default() -> Self {
        Self {
            mode: ModeSetting::Auto,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            extra_ignores: Vec::new(),
            parse_timeout_ms: DEFAULT_PARSE_TIMEOUT_MS,
            idle_shutdown_minutes: DEFAULT_IDLE_SHUTDOWN_MINUTES,
            mmap_bytes: DEFAULT_MMAP_BYTES,
            crawl_files_per_second: DEFAULT_CRAWL_FILES_PER_SECOND,
            store_prompt_excerpt: false,
            holdout_share: DEFAULT_HOLDOUT_SHARE,
            learning_eta: DEFAULT_LEARNING_ETA,
            link_decay: DEFAULT_LINK_DECAY,
            interest_decay: DEFAULT_INTEREST_DECAY,
            git_max_commits: DEFAULT_GIT_MAX_COMMITS,
            git_max_age_days: DEFAULT_GIT_MAX_AGE_DAYS,
            git_max_files_per_commit: DEFAULT_GIT_MAX_FILES_PER_COMMIT,
            git_half_life_commits: DEFAULT_GIT_HALF_LIFE_COMMITS,
            test_extra_patterns: Vec::new(),
            retrieval: RetrievalConfig::default(),
            warnings: WarningsConfig::default(),
            coordination: CoordinationConfig::default(),
            is_strict_mode: false,
            hints: HintsConfig::default(),
            compaction: CompactionConfig::default(),
            adopt: AdoptConfig::default(),
        }
    }
}

/// Whose file a configuration layer is: a repository can carry its own `.cortex/config.toml` in
/// its commits, so a key that must be the user's own decision is read from the user's file alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// The user's own file, `~/.cortex/config.toml`.
    User,
    /// A repository's file, `.cortex/config.toml`.
    Repository,
}

/// One configuration file's text and a name for it in messages, such as its path.
#[derive(Debug, Clone, Copy)]
pub struct ConfigLayer<'a> {
    /// Whose file it is.
    pub kind: LayerKind,
    /// Where the text came from, shown in warnings and errors.
    pub origin: &'a str,
    /// The file's TOML text.
    pub text: &'a str,
}

/// The settings that result from the layers, with a warning for each key that was ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    /// The settings in effect.
    pub config: Config,
    /// One line per key that was ignored, naming the file and the key: an unknown key, or a key
    /// only the user's file may set that a repository's file sets.
    pub warnings: Vec<String>,
    /// The profile whose defaults the keys started from.
    pub profile: Profile,
    /// For each key a layer sets, the profile's included, the position in the layers given to
    /// [`load`] of the last one that sets it.
    pub set_by: BTreeMap<&'static str, usize>,
}

/// Why a configuration file cannot be used.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// The file is not valid TOML.
    #[error("{origin}: not valid TOML: {message}")]
    Syntax {
        /// The file.
        origin: String,
        /// The parser's description of the problem.
        message: String,
    },
    /// A known key has a value of the wrong type or out of range.
    #[error("{origin}: `{key}` must be {expected}")]
    Value {
        /// The file.
        origin: String,
        /// The key, written with dots.
        key: String,
        /// What the key accepts.
        expected: &'static str,
    },
}

/// The settings that result from `layers`, applied in order so that each overrides the ones
/// before it: pass the user's file first and the repository's last. The last `profile` any layer
/// sets is applied first, so that every other key a layer sets overrides it. A key only the user's
/// file may set (see [`ConfigKey::is_user_only`]) is skipped, with a warning, in a repository's
/// layer.
///
/// # Errors
///
/// A file that is not valid TOML, or a known key with an unusable value.
pub fn load(layers: &[ConfigLayer<'_>]) -> Result<LoadedConfig, ConfigError> {
    let tables = layers.iter().map(parsed).collect::<Result<Vec<_>, _>>()?;
    let mut set_by = BTreeMap::new();
    let mut profile = Profile::default();
    for (position, (layer, table)) in layers.iter().zip(&tables).enumerate() {
        if let Some(value) = table.get(PROFILE_KEY) {
            profile = Profile::read(value)
                .map_err(|expected| refusal(layer.origin, PROFILE_KEY, expected))?;
            set_by.insert(PROFILE_KEY, position);
        }
    }
    let mut config = profile.defaults()?;
    let mut warnings = Vec::new();
    for (position, (layer, table)) in layers.iter().zip(&tables).enumerate() {
        let mut entries = Vec::new();
        flatten("", table, &mut entries);
        for (key, value) in entries {
            if key == PROFILE_KEY {
                continue;
            }
            let Some(entry) = config_key(&key) else {
                warnings.push(format!("{}: unknown key `{key}` is ignored", layer.origin));
                continue;
            };
            // INVARIANT: no-content-stored (a repository's file must not turn on keeping prompt
            // text, which is the user's own decision)
            if entry.is_user_only() && layer.kind == LayerKind::Repository {
                warnings.push(format!(
                    "{}: `{key}` is read only from the user's file; this value is ignored",
                    layer.origin
                ));
                continue;
            }
            entry
                .read(&mut config, value)
                .map_err(|expected| refusal(layer.origin, &key, expected))?;
            set_by.insert(entry.name, position);
        }
    }
    Ok(LoadedConfig {
        config,
        warnings,
        profile,
        set_by,
    })
}

/// The error for the value of `key` in the file `origin`, which must be `expected`.
fn refusal(origin: &str, key: &str, expected: &'static str) -> ConfigError {
    ConfigError::Value {
        origin: origin.to_owned(),
        key: key.to_owned(),
        expected,
    }
}

/// The keys the configuration file `layer` sets, known or not, written with dots, in the order of
/// their names.
///
/// # Errors
///
/// The file is not valid TOML.
pub fn keys_set(layer: &ConfigLayer<'_>) -> Result<Vec<String>, ConfigError> {
    let table = parsed(layer)?;
    let mut entries = Vec::new();
    flatten("", &table, &mut entries);
    Ok(entries.into_iter().map(|(key, _)| key).collect())
}

/// The TOML table the file `layer` holds.
///
/// # Errors
///
/// The file is not valid TOML.
fn parsed(layer: &ConfigLayer<'_>) -> Result<toml::Table, ConfigError> {
    parsed_text(layer.origin, layer.text)
}

/// The TOML table `text` holds, a file named `origin` in messages.
///
/// # Errors
///
/// The text is not valid TOML.
fn parsed_text(origin: &str, text: &str) -> Result<toml::Table, ConfigError> {
    text.parse()
        .map_err(|error: toml::de::Error| ConfigError::Syntax {
            origin: origin.to_owned(),
            message: error.message().to_owned(),
        })
}

/// Every value in `table` that is not itself a table, with its key written with dots.
fn flatten<'a>(prefix: &str, table: &'a toml::Table, out: &mut Vec<(String, &'a toml::Value)>) {
    for (name, value) in table {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            toml::Value::Table(inner) => flatten(&key, inner, out),
            other => out.push((key, other)),
        }
    }
}

/// The mode setting a configured word names.
fn mode_setting(word: &str) -> Option<ModeSetting> {
    if word == "auto" {
        Some(ModeSetting::Auto)
    } else {
        ScaleMode::from_name(word).map(ModeSetting::Pinned)
    }
}

/// The value as a whole number above zero.
fn positive_integer(value: &toml::Value) -> Option<u64> {
    whole_number(value).filter(|number| *number > 0)
}

/// The value as a whole number, zero included.
fn whole_number(value: &toml::Value) -> Option<u64> {
    value
        .as_integer()
        .and_then(|number| u64::try_from(number).ok())
}

/// The value as a whole number from `low` to `high`.
fn count(value: &toml::Value, low: u32, high: u32) -> Option<u32> {
    whole_number(value)
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| (low..=high).contains(number))
}

/// The value as a share from 0 to 1, written as a number with or without a fraction.
fn share(value: &toml::Value) -> Option<f64> {
    let number = match value {
        toml::Value::Float(number) => *number,
        toml::Value::Integer(number) => f64::from(i32::try_from(*number).ok()?),
        _ => return None,
    };
    (0.0..=1.0).contains(&number).then_some(number)
}

/// The value as a factor above 0 and at most 1, written as a number with or without a fraction.
fn factor(value: &toml::Value) -> Option<f64> {
    share(value).filter(|number| *number > 0.0)
}

/// Whether `pattern` is a regular expression the `regex` crate accepts.
fn is_regex(pattern: &str) -> bool {
    regex::Regex::new(pattern).is_ok()
}

/// The value as a list of strings.
fn string_list(value: &toml::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A user's layer named `origin` holding `text`.
    fn layer<'a>(origin: &'a str, text: &'a str) -> ConfigLayer<'a> {
        ConfigLayer {
            kind: LayerKind::User,
            origin,
            text,
        }
    }

    /// A repository's layer named `origin` holding `text`.
    fn repository_layer<'a>(origin: &'a str, text: &'a str) -> ConfigLayer<'a> {
        ConfigLayer {
            kind: LayerKind::Repository,
            origin,
            text,
        }
    }

    /// With no files, every setting has its documented default.
    #[test]
    fn no_files_give_the_defaults() -> Result<(), ConfigError> {
        let loaded = load(&[])?;
        assert_eq!(
            loaded.config,
            Config {
                mode: ModeSetting::Auto,
                max_file_bytes: 1_048_576,
                extra_ignores: Vec::new(),
                parse_timeout_ms: 1000,
                idle_shutdown_minutes: 30,
                mmap_bytes: 256 * 1024 * 1024,
                crawl_files_per_second: 100,
                store_prompt_excerpt: false,
                holdout_share: 0.1,
                learning_eta: 0.1,
                link_decay: 0.995,
                interest_decay: 0.98,
                git_max_commits: 20_000,
                git_max_age_days: 730,
                git_max_files_per_commit: 30,
                git_half_life_commits: 500,
                test_extra_patterns: Vec::new(),
                retrieval: RetrievalConfig {
                    max_tokens: 900,
                    max_files: 5,
                    max_code_spans: 2,
                    span_max_lines: 60,
                    teleport: 0.5,
                    type_weights: TypeWeights {
                        structural: 1.0,
                        cochange: 0.6,
                        coaccess: 0.8,
                        about: 1.0,
                        containment: 0.1,
                    },
                    min_confidence: 0.7,
                },
                warnings: WarningsConfig {
                    max_per_edit: 3,
                    cochange_min_confidence: 0.5,
                    cochange_min_count: 3,
                    cochange_min_link_weight: 1.0,
                    cochange_min_link_tasks: 3,
                },
                coordination: CoordinationConfig {
                    read_minutes: 10,
                    edit_minutes: 30,
                    edit_after_task_minutes: 5,
                    claim_minutes: 120,
                    live_minutes: 15,
                    same_file_minutes: 10,
                    handoff_idle_minutes: 30,
                    structural_min_weight: 0.8,
                    cochange_min_confidence: 0.5,
                    max_notices: 3,
                    max_tokens: 200,
                    is_contracts: false,
                },
                is_strict_mode: false,
                hints: HintsConfig::default(),
                compaction: CompactionConfig::default(),
                adopt: AdoptConfig::default(),
            }
        );
        assert_eq!(loaded.warnings, Vec::<String>::new());
        Ok(())
    }

    /// The repository's file overrides the user's, which overrides the defaults, key by key.
    #[test]
    fn later_layers_override_earlier_ones() -> Result<(), ConfigError> {
        let user = layer(
            "user",
            "mode = \"small\"\nstrict_mode = true\n[index]\nmax_file_bytes = 2000\nextra_ignores = [\"*.log\"]\n\
             [hints]\nruntime_grounding = true\nnarrow_tests = true\nredundant_commands = true\n\
             [compaction]\nkeep_list = true\n[privacy]\nstore_prompt_excerpt = true\n",
        );
        let repository = repository_layer(
            "repository",
            "[index]\nmax_file_bytes = 5000\nparse_timeout_ms = 250\ncrawl_files_per_second = 7\n\
             [daemon]\nidle_shutdown_minutes = 5\n[store]\nmmap_bytes = 0\n\
             [holdout]\nshare = 0.25\n\
             [learning]\neta = 0.2\nlink_decay = 1\ninterest_decay = 0.5\n\
             [git]\nmax_commits = 100\nmax_age_days = 7\nmax_files_per_commit = 5\n\
             half_life_commits = 50\n[tests]\nextra_patterns = [\"^just test\", \"tox( |$)\"]\n\
             [retrieval]\nmax_tokens = 1200\nmax_files = 20\nmax_code_spans = 0\n\
             span_max_lines = 1\nteleport = 1\nmin_confidence = 0.8\n\
             [retrieval.type_weights]\nstructural = 0\ncochange = 1\ncoaccess = 0.25\nabout = 0.5\n\
             containment = 0.05\n\
             [warnings]\nmax_per_edit = 0\ncochange_min_confidence = 1\n\
             cochange_min_count = 7\ncochange_min_link_weight = 2.5\ncochange_min_link_tasks = 6\n\
             [coordination]\nread_minutes = 1\nedit_minutes = 2\nedit_after_task_minutes = 3\n\
             claim_minutes = 1440\nlive_minutes = 4\nsame_file_minutes = 6\n\
             handoff_idle_minutes = 45\n\
             structural_min_weight = 0.9\ncochange_min_confidence = 0\nmax_notices = 0\n\
             max_tokens = 400\ncontracts = true\n",
        );
        let loaded = load(&[user, repository])?;
        assert_eq!(
            loaded.config,
            Config {
                mode: ModeSetting::Pinned(ScaleMode::Small),
                max_file_bytes: 5000,
                extra_ignores: vec!["*.log".to_owned()],
                parse_timeout_ms: 250,
                idle_shutdown_minutes: 5,
                mmap_bytes: 0,
                crawl_files_per_second: 7,
                store_prompt_excerpt: true,
                holdout_share: 0.25,
                learning_eta: 0.2,
                link_decay: 1.0,
                interest_decay: 0.5,
                git_max_commits: 100,
                git_max_age_days: 7,
                git_max_files_per_commit: 5,
                git_half_life_commits: 50,
                test_extra_patterns: vec!["^just test".to_owned(), "tox( |$)".to_owned()],
                retrieval: RetrievalConfig {
                    max_tokens: 1200,
                    max_files: 20,
                    max_code_spans: 0,
                    span_max_lines: 1,
                    teleport: 1.0,
                    type_weights: TypeWeights {
                        structural: 0.0,
                        cochange: 1.0,
                        coaccess: 0.25,
                        about: 0.5,
                        containment: 0.05,
                    },
                    min_confidence: 0.8,
                },
                warnings: WarningsConfig {
                    max_per_edit: 0,
                    cochange_min_confidence: 1.0,
                    cochange_min_count: 7,
                    cochange_min_link_weight: 2.5,
                    cochange_min_link_tasks: 6,
                },
                coordination: CoordinationConfig {
                    read_minutes: 1,
                    edit_minutes: 2,
                    edit_after_task_minutes: 3,
                    claim_minutes: 1440,
                    live_minutes: 4,
                    same_file_minutes: 6,
                    handoff_idle_minutes: 45,
                    structural_min_weight: 0.9,
                    cochange_min_confidence: 0.0,
                    max_notices: 0,
                    max_tokens: 400,
                    is_contracts: true,
                },
                is_strict_mode: true,
                hints: HintsConfig {
                    is_runtime_grounding: true,
                    is_narrow_tests: true,
                    is_redundant_commands: true,
                },
                compaction: CompactionConfig { is_keep_list: true },
                adopt: AdoptConfig::default(),
            }
        );
        Ok(())
    }

    /// A key only the user's file may set is applied from the user's file. A repository's file
    /// setting it is skipped, whatever it holds: the settings stay as they were, the key's source is
    /// still the user's file or none, and one line names the file and the key.
    #[test]
    fn a_repository_cannot_set_a_user_only_key() -> Result<(), ConfigError> {
        let user_text =
            "privacy.store_prompt_excerpt = true\nadopt.exclude = [\"~/Downloads/**\"]\n";
        let from_user = load(&[layer("user", user_text)])?;
        assert!(from_user.config.store_prompt_excerpt);
        assert_eq!(from_user.config.adopt.exclude, ["~/Downloads/**"]);
        assert_eq!(
            from_user.set_by.get("privacy.store_prompt_excerpt"),
            Some(&0)
        );
        assert_eq!(from_user.set_by.get("adopt.exclude"), Some(&0));
        assert_eq!(from_user.warnings, Vec::<String>::new());

        // The second value is no glob: a skipped value is not even checked.
        let repository_text =
            "[privacy]\nstore_prompt_excerpt = true\n[adopt]\nexclude = [\"a[\"]\n";
        let alone = load(&[repository_layer("repo", repository_text)])?;
        assert_eq!(alone.config, Config::default());
        assert!(alone.set_by.is_empty());
        assert_eq!(
            alone.warnings,
            [
                "repo: `adopt.exclude` is read only from the user's file; this value is ignored",
                "repo: `privacy.store_prompt_excerpt` is read only from the user's file; this \
                 value is ignored",
            ]
        );

        let both = load(&[
            layer("user", "privacy.store_prompt_excerpt = false\n"),
            repository_layer(
                "repo",
                "privacy.store_prompt_excerpt = true\nmode = \"small\"\n",
            ),
        ])?;
        assert!(!both.config.store_prompt_excerpt);
        assert_eq!(both.config.mode, ModeSetting::Pinned(ScaleMode::Small));
        assert_eq!(both.set_by.get("privacy.store_prompt_excerpt"), Some(&0));
        assert_eq!(both.set_by.get("mode"), Some(&1));
        assert_eq!(
            both.warnings,
            [
                "repo: `privacy.store_prompt_excerpt` is read only from the user's file; this value \
              is ignored"
            ]
        );
        Ok(())
    }

    /// Dotted keys and tables name the same key; unknown keys, at any depth, warn with their file
    /// and change nothing.
    #[test]
    fn unknown_keys_warn() -> Result<(), ConfigError> {
        let loaded = load(&[layer(
            "repo config",
            "index.max_file_bytes = 10\nretrieval.max_tokenz = 3\n[index.extra]\nflavour = \"x\"\n",
        )])?;
        assert_eq!(loaded.config.max_file_bytes, 10);
        assert_eq!(
            loaded.warnings,
            vec![
                "repo config: unknown key `index.extra.flavour` is ignored".to_owned(),
                "repo config: unknown key `retrieval.max_tokenz` is ignored".to_owned(),
            ]
        );
        Ok(())
    }

    /// The keys a file sets are listed with dots, tables and dotted keys alike, whether Cortex
    /// knows them or not; a file that is not TOML is refused.
    #[test]
    fn the_keys_a_file_sets_are_listed() -> Result<(), ConfigError> {
        let text = "mode = \"small\"\nretrieval.max_tokenz = 3\n[holdout]\nshare = 0\n\
                    [warnings]\nmax_per_edit = 2\n";
        assert_eq!(
            keys_set(&layer("f", text))?,
            [
                "holdout.share",
                "mode",
                "retrieval.max_tokenz",
                "warnings.max_per_edit"
            ]
        );
        assert!(matches!(
            keys_set(&layer("f", "[holdout")),
            Err(ConfigError::Syntax { .. })
        ));
        Ok(())
    }

    /// A share may be written as the whole numbers 0 and 1; 0 turns the holdout off.
    #[test]
    fn whole_shares_are_read() -> Result<(), ConfigError> {
        assert_eq!(
            load(&[layer("f", "holdout.share = 0")])?
                .config
                .holdout_share,
            0.0
        );
        assert_eq!(
            load(&[layer("f", "holdout.share = 1")])?
                .config
                .holdout_share,
            1.0
        );
        Ok(())
    }

    /// A known key with an unusable value is an error naming the file, the key and what it
    /// accepts.
    #[test]
    fn bad_values_are_refused() {
        let cases = [
            (
                "mode = \"gigantic\"",
                "mode",
                "one of auto, tiny, small, medium, huge",
            ),
            ("mode = 3", "mode", "one of auto, tiny, small, medium, huge"),
            (
                "index.max_file_bytes = 0",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.max_file_bytes = -5",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.max_file_bytes = \"1MB\"",
                "index.max_file_bytes",
                "a whole number above 0",
            ),
            (
                "index.extra_ignores = [\"a\", 2]",
                "index.extra_ignores",
                "a list of strings",
            ),
            (
                "privacy.store_prompt_excerpt = \"yes\"",
                "privacy.store_prompt_excerpt",
                "true or false",
            ),
            (
                "index.parse_timeout_ms = 0",
                "index.parse_timeout_ms",
                "a whole number above 0",
            ),
            (
                "index.parse_timeout_ms = 1.5",
                "index.parse_timeout_ms",
                "a whole number above 0",
            ),
            (
                "daemon.idle_shutdown_minutes = 0",
                "daemon.idle_shutdown_minutes",
                "a whole number above 0",
            ),
            (
                "store.mmap_bytes = -1",
                "store.mmap_bytes",
                "a whole number from 0 to 2147418112",
            ),
            (
                "store.mmap_bytes = \"256MB\"",
                "store.mmap_bytes",
                "a whole number from 0 to 2147418112",
            ),
            (
                "store.mmap_bytes = 2147418113",
                "store.mmap_bytes",
                "a whole number from 0 to 2147418112",
            ),
            (
                "holdout.share = 1.5",
                "holdout.share",
                "a number from 0 to 1",
            ),
            (
                "holdout.share = -0.1",
                "holdout.share",
                "a number from 0 to 1",
            ),
            ("holdout.share = 2", "holdout.share", "a number from 0 to 1"),
            (
                "holdout.share = \"10%\"",
                "holdout.share",
                "a number from 0 to 1",
            ),
            (
                "holdout.share = nan",
                "holdout.share",
                "a number from 0 to 1",
            ),
        ];
        assert_refused(&cases);
    }

    /// The learning, git and tests keys refuse what they cannot use, naming the file, the key and
    /// what it accepts: a factor out of its range, a count that is not a whole number above 0, a
    /// pattern the `regex` crate refuses.
    #[test]
    fn bad_learning_and_git_values_are_refused() {
        let cases = [
            (
                "learning.eta = 0",
                "learning.eta",
                "a number above 0 and at most 1",
            ),
            (
                "learning.eta = 1.5",
                "learning.eta",
                "a number above 0 and at most 1",
            ),
            (
                "learning.link_decay = -0.5",
                "learning.link_decay",
                "a number above 0 and at most 1",
            ),
            (
                "learning.interest_decay = \"0.98\"",
                "learning.interest_decay",
                "a number above 0 and at most 1",
            ),
            (
                "git.max_commits = 0",
                "git.max_commits",
                "a whole number above 0",
            ),
            (
                "git.max_age_days = 1.5",
                "git.max_age_days",
                "a whole number above 0",
            ),
            (
                "git.max_files_per_commit = -1",
                "git.max_files_per_commit",
                "a whole number above 0",
            ),
            (
                "git.half_life_commits = \"500\"",
                "git.half_life_commits",
                "a whole number above 0",
            ),
            (
                "tests.extra_patterns = [\"(unclosed\"]",
                "tests.extra_patterns",
                "a list of regular expressions",
            ),
            (
                "tests.extra_patterns = \"pytest\"",
                "tests.extra_patterns",
                "a list of regular expressions",
            ),
        ];
        assert_refused(&cases);
    }

    /// `retrieval.min_confidence` is a chance of at least one half: 1.5, -0.1, 0 and 0.49 are
    /// refused, naming the file, the key and what it accepts; 0.5 and the whole number 1 are read.
    #[test]
    fn bad_min_confidence_is_refused() -> Result<(), ConfigError> {
        let key = "retrieval.min_confidence";
        let expected = "a number from 0.5 to 1";
        assert_refused(&[
            ("retrieval.min_confidence = 1.5", key, expected),
            ("retrieval.min_confidence = -0.1", key, expected),
            ("retrieval.min_confidence = 0", key, expected),
            ("retrieval.min_confidence = 0.49", key, expected),
        ]);
        for (text, expected) in [
            ("retrieval.min_confidence = 0.5", 0.5),
            ("retrieval.min_confidence = 1", 1.0),
        ] {
            let read = load(&[layer("f", text)])?.config.retrieval.min_confidence;
            assert!((read - expected).abs() < f64::EPSILON, "{text}");
        }
        Ok(())
    }

    /// The retrieval keys refuse what retrieval cannot use, naming the file, the key and what it
    /// accepts: a work order past its hard cap of 1,200 tokens or of no tokens at all, more files,
    /// code spans or lines than a work order may hold, a restart probability of 0 or above 1, a
    /// link weight outside 0 to 1. A kind of link Cortex does not weigh is an unknown key.
    #[test]
    fn bad_retrieval_values_are_refused() -> Result<(), ConfigError> {
        let cases = [
            (
                "retrieval.max_tokens = 0",
                "retrieval.max_tokens",
                "a whole number from 1 to 1200",
            ),
            (
                "retrieval.max_tokens = 1201",
                "retrieval.max_tokens",
                "a whole number from 1 to 1200",
            ),
            (
                "retrieval.max_tokens = \"900\"",
                "retrieval.max_tokens",
                "a whole number from 1 to 1200",
            ),
            (
                "retrieval.max_files = 0",
                "retrieval.max_files",
                "a whole number from 1 to 20",
            ),
            (
                "retrieval.max_files = 21",
                "retrieval.max_files",
                "a whole number from 1 to 20",
            ),
            (
                "retrieval.max_code_spans = -1",
                "retrieval.max_code_spans",
                "a whole number from 0 to 5",
            ),
            (
                "retrieval.max_code_spans = 6",
                "retrieval.max_code_spans",
                "a whole number from 0 to 5",
            ),
            (
                "retrieval.span_max_lines = 0",
                "retrieval.span_max_lines",
                "a whole number from 1 to 200",
            ),
            (
                "retrieval.span_max_lines = 201",
                "retrieval.span_max_lines",
                "a whole number from 1 to 200",
            ),
            (
                "retrieval.span_max_lines = 4294967356",
                "retrieval.span_max_lines",
                "a whole number from 1 to 200",
            ),
            (
                "retrieval.teleport = 0",
                "retrieval.teleport",
                "a number above 0 and at most 1",
            ),
            (
                "retrieval.teleport = 1.5",
                "retrieval.teleport",
                "a number above 0 and at most 1",
            ),
            (
                "retrieval.type_weights.structural = -0.1",
                "retrieval.type_weights.structural",
                "a number from 0 to 1",
            ),
            (
                "retrieval.type_weights.cochange = 1.5",
                "retrieval.type_weights.cochange",
                "a number from 0 to 1",
            ),
            (
                "retrieval.type_weights.coaccess = \"high\"",
                "retrieval.type_weights.coaccess",
                "a number from 0 to 1",
            ),
            (
                "retrieval.type_weights.about = nan",
                "retrieval.type_weights.about",
                "a number from 0 to 1",
            ),
            (
                "retrieval.type_weights.containment = 2",
                "retrieval.type_weights.containment",
                "a number from 0 to 1",
            ),
        ];
        assert_refused(&cases);
        let loaded = load(&[layer(
            "f",
            "retrieval.type_weights.semantic = 0.5
",
        )])?;
        assert_eq!(loaded.config.retrieval, RetrievalConfig::default());
        assert_eq!(
            loaded.warnings,
            ["f: unknown key `retrieval.type_weights.semantic` is ignored"]
        );
        Ok(())
    }

    /// The warning keys and `strict_mode` refuse what the rules cannot use, naming the file, the key
    /// and what it accepts: more warnings an edit than fit their shared cap, a count below zero, a
    /// confidence outside 0 to 1, a co-change count of 0, a word where true or false belongs.
    #[test]
    fn bad_warning_values_are_refused() {
        assert_refused(&[
            (
                "warnings.max_per_edit = 11",
                "warnings.max_per_edit",
                "a whole number from 0 to 10",
            ),
            (
                "warnings.max_per_edit = -1",
                "warnings.max_per_edit",
                "a whole number from 0 to 10",
            ),
            (
                "warnings.cochange_min_confidence = 1.5",
                "warnings.cochange_min_confidence",
                "a number from 0 to 1",
            ),
            (
                "warnings.cochange_min_count = 0",
                "warnings.cochange_min_count",
                "a whole number above 0",
            ),
            (
                "warnings.cochange_min_count = 4294967296",
                "warnings.cochange_min_count",
                "a whole number above 0",
            ),
            (
                "warnings.cochange_min_link_weight = -1",
                "warnings.cochange_min_link_weight",
                "a number of 0 or more",
            ),
            (
                "warnings.cochange_min_link_tasks = 0",
                "warnings.cochange_min_link_tasks",
                "a whole number above 0",
            ),
            ("strict_mode = \"yes\"", "strict_mode", "true or false"),
            (
                "hints.runtime_grounding = 1",
                "hints.runtime_grounding",
                "true or false",
            ),
            (
                "hints.narrow_tests = \"yes\"",
                "hints.narrow_tests",
                "true or false",
            ),
            (
                "hints.redundant_commands = 0",
                "hints.redundant_commands",
                "true or false",
            ),
            (
                "compaction.keep_list = 1",
                "compaction.keep_list",
                "true or false",
            ),
        ]);
    }

    /// The coordination keys refuse what the rules cannot use: a hold of no minutes or of more
    /// than a day, a link weight outside 0 to 1, more notices than fit an answer, a token cap of 0
    /// or past a third of the work order's hard cap, a switch that is not true or false.
    #[test]
    fn bad_coordination_values_are_refused() {
        let minutes = "a whole number of minutes from 1 to 1440";
        assert_refused(&[
            (
                "coordination.read_minutes = 0",
                "coordination.read_minutes",
                minutes,
            ),
            (
                "coordination.claim_minutes = 1441",
                "coordination.claim_minutes",
                minutes,
            ),
            (
                "coordination.live_minutes = 2.5",
                "coordination.live_minutes",
                minutes,
            ),
            (
                "coordination.structural_min_weight = 1.2",
                "coordination.structural_min_weight",
                "a number from 0 to 1",
            ),
            (
                "coordination.cochange_min_confidence = -0.1",
                "coordination.cochange_min_confidence",
                "a number from 0 to 1",
            ),
            (
                "coordination.max_notices = 11",
                "coordination.max_notices",
                "a whole number from 0 to 10",
            ),
            (
                "coordination.max_tokens = 0",
                "coordination.max_tokens",
                "a whole number from 1 to 400",
            ),
            (
                "coordination.max_tokens = 401",
                "coordination.max_tokens",
                "a whole number from 1 to 400",
            ),
            (
                "coordination.contracts = 1",
                "coordination.contracts",
                "true or false",
            ),
        ]);
    }

    /// Checks that each file text in `cases` is refused for its key with what the key accepts.
    fn assert_refused(cases: &[(&str, &str, &'static str)]) {
        for (text, key, expected) in cases {
            assert_eq!(
                load(&[layer("f", text)]),
                Err(ConfigError::Value {
                    origin: "f".to_owned(),
                    key: (*key).to_owned(),
                    expected,
                }),
                "{text}"
            );
        }
    }

    /// A file that is not TOML is an error naming the file.
    #[test]
    fn broken_files_are_refused() {
        let error = load(&[layer("user config", "mode = \"auto")]).err();
        assert!(
            matches!(&error, Some(ConfigError::Syntax { origin, .. }) if origin == "user config"),
            "{error:?}"
        );
    }
}
