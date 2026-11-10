//! The warning settings: how many warnings one edit may bring, and how strong a co-change partner
//! must be before an edit that leaves it untouched is worth a warning.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

/// The default for `warnings.max_per_edit`: the most warnings one edit brings, strongest first.
pub const DEFAULT_MAX_PER_EDIT: u32 = 3;

/// The most `warnings.max_per_edit` may be set to: every warning of an edit shares one 250-token
/// cap, which more than this many would not fit.
const MAX_PER_EDIT_LIMIT: u32 = 10;

/// The default for `warnings.cochange_min_confidence`: the share of the edited file's commits that
/// must also have changed the partner.
pub const DEFAULT_COCHANGE_MIN_CONFIDENCE: f64 = 0.5;

/// The default for `warnings.cochange_min_count`: how many commits must have changed both files.
pub const DEFAULT_COCHANGE_MIN_COUNT: u32 = 3;

/// The default for `warnings.cochange_min_link_weight`: the least weight, after fading, a link of
/// agents' past work needs before an edit that leaves its other file untouched is worth a warning:
/// about ten successful tasks that edited both files.
pub const DEFAULT_COCHANGE_MIN_LINK_WEIGHT: f64 = 1.0;

/// The default for `warnings.cochange_min_link_tasks`: the fewest tasks that must have drawn such a
/// link.
pub const DEFAULT_COCHANGE_MIN_LINK_TASKS: u32 = 3;

/// The settings the warning rules read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarningsConfig {
    /// `warnings.max_per_edit`: the most warnings one edit brings, from 0 to
    /// [`MAX_PER_EDIT_LIMIT`]; 0 gives none. Default [`DEFAULT_MAX_PER_EDIT`].
    pub max_per_edit: u32,
    /// `warnings.cochange_min_confidence`: the least co-change confidence of a partner worth a
    /// warning, from 0 to 1. Default [`DEFAULT_COCHANGE_MIN_CONFIDENCE`].
    pub cochange_min_confidence: f64,
    /// `warnings.cochange_min_count`: the fewest commits changing both files, above 0. Default
    /// [`DEFAULT_COCHANGE_MIN_COUNT`].
    pub cochange_min_count: u32,
    /// `warnings.cochange_min_link_weight`: the least faded weight of a link of agents' past work
    /// worth a warning, 0 or more. Default [`DEFAULT_COCHANGE_MIN_LINK_WEIGHT`].
    pub cochange_min_link_weight: f64,
    /// `warnings.cochange_min_link_tasks`: the fewest tasks that drew that link, above 0. Default
    /// [`DEFAULT_COCHANGE_MIN_LINK_TASKS`].
    pub cochange_min_link_tasks: u32,
}

impl Default for WarningsConfig {
    /// Every warning setting at its documented default.
    fn default() -> Self {
        Self {
            max_per_edit: DEFAULT_MAX_PER_EDIT,
            cochange_min_confidence: DEFAULT_COCHANGE_MIN_CONFIDENCE,
            cochange_min_count: DEFAULT_COCHANGE_MIN_COUNT,
            cochange_min_link_weight: DEFAULT_COCHANGE_MIN_LINK_WEIGHT,
            cochange_min_link_tasks: DEFAULT_COCHANGE_MIN_LINK_TASKS,
        }
    }
}

impl WarningsConfig {
    /// Sets the warning key `key` (written with dots, `warnings.` included) to `value`:
    /// `Some(Ok(()))` when set, `Some(Err(expected))` with what the key accepts when the value is
    /// unusable, `None` when the key is not a warning key Cortex reads.
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
        let set = match key {
            "warnings.max_per_edit" => count(0, MAX_PER_EDIT_LIMIT, "a whole number from 0 to 10")
                .map(|most| self.max_per_edit = most),
            "warnings.cochange_min_confidence" => super::share(value)
                .ok_or("a number from 0 to 1")
                .map(|confidence| self.cochange_min_confidence = confidence),
            "warnings.cochange_min_count" => count(1, u32::MAX, "a whole number above 0")
                .map(|fewest| self.cochange_min_count = fewest),
            "warnings.cochange_min_link_weight" => match value {
                toml::Value::Float(number) => Some(*number),
                toml::Value::Integer(number) => i32::try_from(*number).ok().map(f64::from),
                _ => None,
            }
            .filter(|weight| weight.is_finite() && *weight >= 0.0)
            .ok_or("a number of 0 or more")
            .map(|least| self.cochange_min_link_weight = least),
            "warnings.cochange_min_link_tasks" => count(1, u32::MAX, "a whole number above 0")
                .map(|fewest| self.cochange_min_link_tasks = fewest),
            _ => return None,
        };
        Some(set)
    }
}
