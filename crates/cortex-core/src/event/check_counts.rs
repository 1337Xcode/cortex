//! The counts a failed check's output printed: how many tests failed, passed, ended in an error and
//! were skipped. Numbers only, read from the output in the daemon's memory; the output itself is
//! never kept.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The counts a runner printed.
///
/// `failed`, `errors` and `skipped` are 0 when the runner printed none of them. `passed` is `None`
/// when the runner printed no passing count and none can be worked out from what it printed:
/// `go test` without `-v` prints only its failures, and a compiler runs no tests. It is `Some(0)`
/// when the runner prints every count it has (pytest, cargo, jest, `JUnit`) and none passed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    /// Tests that failed.
    pub failed: u32,
    /// Tests that passed, when the output says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<u32>,
    /// Tests or builds that ended in an error rather than a failed assertion.
    pub errors: u32,
    /// Tests skipped or ignored.
    pub skipped: u32,
}

impl fmt::Display for CheckCounts {
    /// The counts in words, those of 0 left out but the failures: `1 failed, 7 passed`,
    /// `0 failed, 2 errors`, `3 failed, 1 skipped`.
    ///
    /// ```
    /// use cortex_core::CheckCounts;
    ///
    /// let counts = CheckCounts { failed: 1, passed: Some(7), errors: 0, skipped: 0 };
    /// assert_eq!(counts.to_string(), "1 failed, 7 passed");
    /// let counts = CheckCounts { failed: 0, passed: None, errors: 2, skipped: 1 };
    /// assert_eq!(counts.to_string(), "0 failed, 2 errors, 1 skipped");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed", self.failed)?;
        if let Some(passed) = self.passed.filter(|passed| *passed > 0) {
            write!(formatter, ", {passed} passed")?;
        }
        match self.errors {
            0 => {}
            1 => write!(formatter, ", 1 error")?,
            errors => write!(formatter, ", {errors} errors")?,
        }
        if self.skipped > 0 {
            write!(formatter, ", {} skipped", self.skipped)?;
        }
        Ok(())
    }
}
