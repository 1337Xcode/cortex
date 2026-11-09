//! The steps the daemon runs on its own that failed since the user was last told: storing what
//! agents did, reading the git history, reading the work tree's changes. A failure there means
//! Cortex is quietly not doing its job, so the next session's start tells the user, once, and never
//! the model. A missed budget on an answer is momentary and is not one of them: it stays in
//! `cortex status`.

use std::collections::BTreeMap;

use cortex_core::one_line;

/// The longest last error shown, in characters: enough to name what failed and why.
const MAX_ERROR_CHARS: usize = 160;

/// A minute in milliseconds.
const MINUTE_MS: i64 = 60_000;

/// A step the daemon runs on its own, as the user is told of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    /// Writing what agents did and what was learned from it.
    Storing,
    /// Reading the git history for the files that change together.
    Mining,
    /// Reading the work tree's uncommitted changes.
    WorkTree,
}

impl Step {
    /// What the step does, as the user is told of it.
    fn said(self) -> &'static str {
        match self {
            Self::Storing => "storing what agents did",
            Self::Mining => "reading the git history",
            Self::WorkTree => "reading the work tree's changes",
        }
    }
}

/// How one step failed since the user was last told.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Trouble {
    /// How many times.
    count: u64,
    /// When first, in UTC milliseconds.
    first_ms: i64,
    /// The last failure's error, on one line.
    last: String,
}

/// The steps that failed since the user was last told, each with how.
#[derive(Debug, Default)]
pub(crate) struct Troubles {
    /// Each step's failures.
    by_step: BTreeMap<Step, Trouble>,
}

impl Troubles {
    /// Counts a failure of `step`, met at `at_ms` with `error`.
    pub(crate) fn note(&mut self, step: Step, error: &str, at_ms: i64) {
        let last = one_line(error, MAX_ERROR_CHARS);
        self.by_step
            .entry(step)
            .and_modify(|trouble| {
                trouble.count = trouble.count.saturating_add(1);
                trouble.last.clone_from(&last);
            })
            .or_insert(Trouble {
                count: 1,
                first_ms: at_ms,
                last: last.clone(),
            });
    }

    /// What the user is told at `now_ms`, one line a step that failed, and from then on nothing of
    /// those failures; none when no step failed since.
    pub(crate) fn take_said(&mut self, now_ms: i64) -> Option<String> {
        let taken = std::mem::take(&mut self.by_step);
        let lines: Vec<String> = taken
            .into_iter()
            .map(|(step, trouble)| {
                format!(
                    "Cortex: {} failed {} {}: {}. `cortex status` shows more.",
                    step.said(),
                    times(trouble.count),
                    since(now_ms.saturating_sub(trouble.first_ms)),
                    trouble.last
                )
            })
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

/// How many times, in words.
fn times(count: u64) -> String {
    if count == 1 {
        "once".to_owned()
    } else {
        format!("{count} times")
    }
}

/// Over how long, `ago_ms` back from now, in whole minutes or hours.
fn since(ago_ms: i64) -> String {
    match ago_ms.max(0) / MINUTE_MS {
        0 | 1 => "in the last minute".to_owned(),
        minutes @ 2..=119 => format!("in the last {minutes} minutes"),
        minutes => format!("in the last {} hours", minutes / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Storing failed twice, first 12 minutes and 30 seconds before it is said, then mining once
    /// just before: one line each, storing first, its last error shown; then nothing more until a
    /// step fails again.
    #[test]
    fn failures_are_said_once_a_step() {
        let mut troubles = Troubles::default();
        troubles.note(
            Step::Storing,
            "recording hook events failed: disk full",
            1_000,
        );
        troubles.note(Step::Mining, "reading the history: bad object", 700_000);
        troubles.note(
            Step::Storing,
            "recording an injection failed: database is locked",
            600_000,
        );
        assert_eq!(
            troubles.take_said(751_000).as_deref(),
            Some(
                "Cortex: storing what agents did failed 2 times in the last 12 minutes: recording \
                 an injection failed: database is locked. `cortex status` shows more.\n\
                 Cortex: reading the git history failed once in the last minute: reading the \
                 history: bad object. `cortex status` shows more."
            )
        );
        assert_eq!(troubles.take_said(752_000), None);
        troubles.note(Step::WorkTree, "git status: timed out", 800_000);
        assert_eq!(
            troubles.take_said(800_000 + 3 * 60 * MINUTE_MS).as_deref(),
            Some(
                "Cortex: reading the work tree's changes failed once in the last 3 hours: git \
                 status: timed out. `cortex status` shows more."
            )
        );
    }
}
