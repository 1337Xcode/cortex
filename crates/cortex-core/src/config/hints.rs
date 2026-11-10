//! The hint settings: which of the texts Cortex gives about a check's run are on. Each is off by
//! default until its evaluation shows it helps.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

/// The settings of the hints about a check's run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HintsConfig {
    /// `hints.runtime_grounding`: beside a failed check's output, name the repository lines its
    /// stack ran through when the output does not show them. Default `false`.
    pub is_runtime_grounding: bool,
    /// `hints.narrow_tests`: after a whole test-suite run, name the test files of what the task
    /// edited and the command that runs only them, once a task. Default `false`.
    pub is_narrow_tests: bool,
}

impl HintsConfig {
    /// Sets the hint key `key` (written with dots, `hints.` included) to `value`: `Some(Ok(()))`
    /// when set, `Some(Err(expected))` with what the key accepts when the value is unusable,
    /// `None` when the key is not a hint key Cortex reads.
    pub(super) fn apply(
        &mut self,
        key: &str,
        value: &toml::Value,
    ) -> Option<Result<(), &'static str>> {
        let switch = || value.as_bool().ok_or("true or false");
        Some(match key {
            "hints.runtime_grounding" => switch().map(|is_on| self.is_runtime_grounding = is_on),
            "hints.narrow_tests" => switch().map(|is_on| self.is_narrow_tests = is_on),
            _ => return None,
        })
    }
}
