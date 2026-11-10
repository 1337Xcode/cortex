//! The hint settings: which of the texts Cortex gives about a check's run are on. Each is off by
//! default until its evaluation shows it helps.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use super::{ConfigKey, SWITCH};

/// The settings of the hints about a check's run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HintsConfig {
    /// `hints.runtime_grounding`: beside a failed check's output, name the repository lines its
    /// stack ran through when the output does not show them. Default `false`.
    pub is_runtime_grounding: bool,
    /// `hints.narrow_tests`: after a whole test-suite run, name the test files of what the task
    /// edited and the command that runs only them, once a task. Default `false`.
    pub is_narrow_tests: bool,
    /// `hints.redundant_commands`: before a check runs, say what the same command line in the same
    /// working folder gave when it ran earlier in the session with no source change since, at most
    /// twice a task. Default `false`.
    pub is_redundant_commands: bool,
}

/// The hint keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[
    ConfigKey::new(
        "hints.runtime_grounding",
        "Beside a failed check's output, name the repository lines its stack ran through.",
        |config, value| {
            config.hints.is_runtime_grounding = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.hints.is_runtime_grounding),
    ),
    ConfigKey::new(
        "hints.narrow_tests",
        "After a whole test-suite run, name the tests of what the task edited and the command \
         that runs only them.",
        |config, value| {
            config.hints.is_narrow_tests = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.hints.is_narrow_tests),
    ),
    ConfigKey::new(
        "hints.redundant_commands",
        "Before a check runs again with no source change since, say what its last run gave.",
        |config, value| {
            config.hints.is_redundant_commands = value.as_bool().ok_or(SWITCH)?;
            Ok(())
        },
        |config| toml::Value::Boolean(config.hints.is_redundant_commands),
    ),
];
