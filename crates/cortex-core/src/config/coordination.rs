//! The coordination settings: how long each kind of position a session takes holds, how recent an
//! event keeps a session live, how strongly two files must be linked for an edit of one to concern
//! a session holding the other, and how much coordination text one answer gives.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

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
        }
    }
}

impl CoordinationConfig {
    /// Sets the coordination key `key` (written with dots, `coordination.` included) to `value`:
    /// `Some(Ok(()))` when set, `Some(Err(expected))` with what the key accepts when the value is
    /// unusable, `None` when the key is not a coordination key Cortex reads.
    pub(super) fn apply(
        &mut self,
        key: &str,
        value: &toml::Value,
    ) -> Option<Result<(), &'static str>> {
        let count = |low: u32, high: u32, expected: &'static str| {
            super::whole_number(value)
                .and_then(|number| u32::try_from(number).ok())
                .filter(|number| (low..=high).contains(number))
                .ok_or(expected)
        };
        let minutes = || count(1, MAX_MINUTES, "a whole number of minutes from 1 to 1440");
        let share = || super::share(value).ok_or("a number from 0 to 1");
        let set = match key {
            "coordination.read_minutes" => minutes().map(|held| self.read_minutes = held),
            "coordination.edit_minutes" => minutes().map(|held| self.edit_minutes = held),
            "coordination.edit_after_task_minutes" => {
                minutes().map(|held| self.edit_after_task_minutes = held)
            }
            "coordination.claim_minutes" => minutes().map(|held| self.claim_minutes = held),
            "coordination.live_minutes" => minutes().map(|live| self.live_minutes = live),
            "coordination.same_file_minutes" => {
                minutes().map(|close| self.same_file_minutes = close)
            }
            "coordination.handoff_idle_minutes" => {
                minutes().map(|quiet| self.handoff_idle_minutes = quiet)
            }
            "coordination.structural_min_weight" => {
                share().map(|least| self.structural_min_weight = least)
            }
            "coordination.cochange_min_confidence" => {
                share().map(|least| self.cochange_min_confidence = least)
            }
            "coordination.max_notices" => {
                count(0, MAX_NOTICES_LIMIT, "a whole number from 0 to 10")
                    .map(|most| self.max_notices = most)
            }
            "coordination.max_tokens" => count(1, MAX_TOKENS_LIMIT, "a whole number from 1 to 400")
                .map(|most| self.max_tokens = most),
            _ => return None,
        };
        Some(set)
    }
}
