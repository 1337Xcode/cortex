//! The coordination settings: how long each kind of position a session takes holds, how recent an
//! event keeps a session live, how strongly two files must be linked for an edit of one to concern
//! a session holding the other, how much coordination text one answer gives, and whether a
//! notice of a changed signature says what it changed from and to and where the held file calls
//! it.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use super::{ConfigKey, SHARE, count, share};

/// The default for `coordination.read_minutes`: how long a read holds.
pub const DEFAULT_READ_MINUTES: u32 = 10;

/// The default for `coordination.edit_minutes`: how long an edit holds after the session's last
/// edit of the file.
pub const DEFAULT_EDIT_MINUTES: u32 = 30;

/// The default for `coordination.edit_after_task_minutes`: how long an edit holds after the task
/// that made it ended.
pub const DEFAULT_EDIT_AFTER_TASK_MINUTES: u32 = 5;

/// The default for `coordination.claim_minutes`: how long a claim holds.
pub const DEFAULT_CLAIM_MINUTES: u32 = 120;

/// The default for `coordination.live_minutes`: how recent a session's last event must be for it
/// to be live.
pub const DEFAULT_LIVE_MINUTES: u32 = 15;

/// The default for `coordination.handoff_idle_minutes`: how long a session must have been quiet
/// for its handoff to be given though it never ended, as when its terminal was closed.
pub const DEFAULT_HANDOFF_IDLE_MINUTES: u32 = 30;

/// The default for `coordination.same_file_minutes`: how close two sessions' edits of one file must
/// be for both to be told first.
pub const DEFAULT_SAME_FILE_MINUTES: u32 = 10;

/// The default for `coordination.structural_min_weight`: the least confidence of a code link
/// between two files for an edit of one to concern a session holding the other.
pub const DEFAULT_STRUCTURAL_MIN_WEIGHT: f64 = 0.8;

/// The default for `coordination.cochange_min_confidence`: the least share of the edited file's
/// commits that also changed the held file.
pub const DEFAULT_COCHANGE_MIN_CONFIDENCE: f64 = 0.5;

/// The default for `coordination.max_notices`: the most coordination lines one answer gives,
/// those that act first leading; the rest wait for the session's next hook.
pub const DEFAULT_MAX_NOTICES: u32 = 3;

/// The most `coordination.max_notices` may be set to: more lines than this would crowd out what
/// the answer is for.
const MAX_NOTICES_LIMIT: u32 = 10;

/// The default for `coordination.max_tokens`: the most tokens of coordination text one answer
/// gives; three notices naming two long paths each fit.
pub const DEFAULT_MAX_TOKENS: u32 = 200;

/// The most `coordination.max_tokens` may be set to: a third of the work order's hard cap, so
/// notices and a work order together stay under it.
const MAX_TOKENS_LIMIT: u32 = 400;

/// The longest any position may be set to hold, in minutes: a day.
const MAX_MINUTES: u32 = 24 * 60;

/// The settings coordination reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinationConfig {
    /// `coordination.read_minutes`. Default [`DEFAULT_READ_MINUTES`].
    pub read_minutes: u32,
    /// `coordination.edit_minutes`. Default [`DEFAULT_EDIT_MINUTES`].
    pub edit_minutes: u32,
    /// `coordination.edit_after_task_minutes`. Default [`DEFAULT_EDIT_AFTER_TASK_MINUTES`].
    pub edit_after_task_minutes: u32,
    /// `coordination.claim_minutes`. Default [`DEFAULT_CLAIM_MINUTES`].
    pub claim_minutes: u32,
    /// `coordination.live_minutes`. Default [`DEFAULT_LIVE_MINUTES`].
    pub live_minutes: u32,
    /// `coordination.same_file_minutes`. Default [`DEFAULT_SAME_FILE_MINUTES`].
    pub same_file_minutes: u32,
    /// `coordination.handoff_idle_minutes`. Default [`DEFAULT_HANDOFF_IDLE_MINUTES`].
    pub handoff_idle_minutes: u32,
    /// `coordination.structural_min_weight`, from 0 to 1. Default
    /// [`DEFAULT_STRUCTURAL_MIN_WEIGHT`].
    pub structural_min_weight: f64,
    /// `coordination.cochange_min_confidence`, from 0 to 1. Default
    /// [`DEFAULT_COCHANGE_MIN_CONFIDENCE`].
    pub cochange_min_confidence: f64,
    /// `coordination.max_notices`, from 0 to [`MAX_NOTICES_LIMIT`]; 0 gives none. Default
    /// [`DEFAULT_MAX_NOTICES`].
    pub max_notices: u32,
    /// `coordination.max_tokens`, from 1 to [`MAX_TOKENS_LIMIT`]. Default [`DEFAULT_MAX_TOKENS`].
    pub max_tokens: u32,
    /// `coordination.contracts`: a notice that another session changed the signature of a
    /// definition the held file calls says the signature before and after and the lines that
    /// call it, instead of the definition's name alone. Default `false`, until an evaluation
    /// with two agents shows its effect.
    pub is_contracts: bool,
}

impl Default for CoordinationConfig {
    /// Every coordination setting at its documented default.
    fn default() -> Self {
        Self {
            read_minutes: DEFAULT_READ_MINUTES,
            edit_minutes: DEFAULT_EDIT_MINUTES,
            edit_after_task_minutes: DEFAULT_EDIT_AFTER_TASK_MINUTES,
            claim_minutes: DEFAULT_CLAIM_MINUTES,
            live_minutes: DEFAULT_LIVE_MINUTES,
            same_file_minutes: DEFAULT_SAME_FILE_MINUTES,
            handoff_idle_minutes: DEFAULT_HANDOFF_IDLE_MINUTES,
            structural_min_weight: DEFAULT_STRUCTURAL_MIN_WEIGHT,
            cochange_min_confidence: DEFAULT_COCHANGE_MIN_CONFIDENCE,
            max_notices: DEFAULT_MAX_NOTICES,
            max_tokens: DEFAULT_MAX_TOKENS,
            is_contracts: false,
        }
    }
}

/// The coordination keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[
    ConfigKey::new(
        "coordination.read_minutes",
        "How long a session's read of a file holds, in minutes.",
        |config, value| {
            config.coordination.read_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.read_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.edit_minutes",
        "How long a session's edit of a file holds after its last edit, in minutes.",
        |config, value| {
            config.coordination.edit_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.edit_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.edit_after_task_minutes",
        "How long an edit holds after the task that made it ended, in minutes.",
        |config, value| {
            config.coordination.edit_after_task_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.edit_after_task_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.claim_minutes",
        "How long a claim on files holds, in minutes.",
        |config, value| {
            config.coordination.claim_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.claim_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.live_minutes",
        "How recent a session's last event must be for it to be live, in minutes.",
        |config, value| {
            config.coordination.live_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.live_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.same_file_minutes",
        "How close two sessions' edits of one file must be for both to be told, in \
         minutes.",
        |config, value| {
            config.coordination.same_file_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.same_file_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.handoff_idle_minutes",
        "How long a session that never ended must be quiet before its handoff is \
         given, in minutes.",
        |config, value| {
            config.coordination.handoff_idle_minutes = minutes(value)?;
            Ok(())
        },
        |config| config.coordination.handoff_idle_minutes.into(),
    ),
    ConfigKey::new(
        "coordination.structural_min_weight",
        "The least confidence of a code link between two files for an edit of one to concern a \
         session holding the other.",
        |config, value| {
            config.coordination.structural_min_weight = share(value).ok_or(SHARE)?;
            Ok(())
        },
        |config| toml::Value::Float(config.coordination.structural_min_weight),
    ),
    ConfigKey::new(
        "coordination.cochange_min_confidence",
        "The least share of an edited file's commits that also changed a file another session \
         holds.",
        |config, value| {
            config.coordination.cochange_min_confidence = share(value).ok_or(SHARE)?;
            Ok(())
        },
        |config| toml::Value::Float(config.coordination.cochange_min_confidence),
    ),
    ConfigKey::new(
        "coordination.max_notices",
        "The most coordination lines one answer gives; 0 gives none.",
        |config, value| {
            config.coordination.max_notices =
                count(value, 0, MAX_NOTICES_LIMIT).ok_or("a whole number from 0 to 10")?;
            Ok(())
        },
        |config| config.coordination.max_notices.into(),
    ),
    ConfigKey::new(
        "coordination.max_tokens",
        "The most tokens of coordination text one answer gives.",
        |config, value| {
            config.coordination.max_tokens =
                count(value, 1, MAX_TOKENS_LIMIT).ok_or("a whole number from 1 to 400")?;
            Ok(())
        },
        |config| config.coordination.max_tokens.into(),
    ),
    ConfigKey::new(
        "coordination.contracts",
        "Tell a session holding a file that calls a definition another session changed the \
         signature before and after and the calling lines.",
        |config, value| {
            config.coordination.is_contracts = value.as_bool().ok_or("true or false")?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.coordination.is_contracts),
    ),
];

/// The value as a hold in minutes, from 1 to [`MAX_MINUTES`].
fn minutes(value: &toml::Value) -> Result<u32, &'static str> {
    count(value, 1, MAX_MINUTES).ok_or("a whole number of minutes from 1 to 1440")
}
