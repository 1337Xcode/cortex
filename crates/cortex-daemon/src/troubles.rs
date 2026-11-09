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

    /// What the user is told at `now_ms`, one line a step that failed, with what was taken to say
    /// it; from then on nothing of those failures, unless they are put back. None when no step
    /// failed since.
    pub(crate) fn take_said(&mut self, now_ms: i64) -> Option<(String, Taken)> {
        let taken = std::mem::take(&mut self.by_step);
        let lines: Vec<String> = taken
            .iter()
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
        (!lines.is_empty()).then(|| (lines.join("\n"), Taken(taken)))
    }

    /// Puts back `taken`, whose telling never reached the user: each step's count adds to the
    /// failures since, its first time is the earlier one, and its last error stays the latest.
    pub(crate) fn put_back(&mut self, taken: Taken) {
        for (step, trouble) in taken.0 {
            self.by_step
                .entry(step)
                .and_modify(|since| {
                    since.count = since.count.saturating_add(trouble.count);
                    since.first_ms = since.first_ms.min(trouble.first_ms);
                })
                .or_insert(trouble);
        }
    }
}

/// The failures taken to tell the user, kept so that they can be put back if the telling never
/// reaches them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Taken(BTreeMap<Step, Trouble>);

/// How many times, in words.
fn times(count: u64) -> String {
    if count == 1 {
        "once".to_owned()
    } else {
        format!("{count} times")
    }
}

/// Over how long, `ago_ms` back from now, in whole minutes or hours rounded up: the span said
/// must hold the first failure, so 61 seconds is "the last 2 minutes" and 121 minutes "the last 3
/// hours".
fn since(ago_ms: i64) -> String {
    let minutes = u64::try_from(ago_ms.max(0))
        .unwrap_or(0)
        .div_ceil(MINUTE_MS.unsigned_abs());
    match minutes {
        0 | 1 => "in the last minute".to_owned(),
        2..=119 => format!("in the last {minutes} minutes"),
        _ => format!("in the last {} hours", minutes.div_ceil(60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Storing failed twice, first 12 minutes and 30 seconds before it is said (said as the last
    /// 13 minutes, rounded up to hold it), then mining once 51 seconds before: one line each,
    /// storing first, its last error shown; then nothing more until a step fails again.
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
            said(&mut troubles, 751_000).as_deref(),
            Some(
                "Cortex: storing what agents did failed 2 times in the last 13 minutes: recording \
                 an injection failed: database is locked. `cortex status` shows more.\n\
                 Cortex: reading the git history failed once in the last minute: reading the \
                 history: bad object. `cortex status` shows more."
            )
        );
        assert_eq!(said(&mut troubles, 752_000), None);
        troubles.note(Step::WorkTree, "git status: timed out", 800_000);
        assert_eq!(
            said(&mut troubles, 800_000 + 3 * 60 * MINUTE_MS).as_deref(),
            Some(
                "Cortex: reading the work tree's changes failed once in the last 3 hours: git \
                 status: timed out. `cortex status` shows more."
            )
        );
    }

    /// The text of what `troubles` says at `now_ms`, what it took dropped.
    fn said(troubles: &mut Troubles, now_ms: i64) -> Option<String> {
        troubles.take_said(now_ms).map(|(text, _)| text)
    }

    /// Failures taken whose telling never reached the user are put back: with a failure of the
    /// same step since, the counts add (1 + 1), the first time is the earlier (1,000), the last
    /// error the latest; said at 121,000, two minutes after the first.
    #[test]
    fn failures_put_back_are_said_again() {
        let mut troubles = Troubles::default();
        troubles.note(Step::Mining, "reading the history: bad object", 1_000);
        let (_, taken) = troubles.take_said(2_000).unwrap_or_default();
        troubles.note(Step::Mining, "reading the history: timed out", 60_000);
        troubles.put_back(taken);
        assert_eq!(
            said(&mut troubles, 121_000).as_deref(),
            Some(
                "Cortex: reading the git history failed 2 times in the last 2 minutes: reading \
                 the history: timed out. `cortex status` shows more."
            )
        );
    }

    /// The span said rounds up and holds the first failure: 0 and 60 seconds are the last minute,
    /// 61 seconds the last 2, 119 minutes the last 119, 120 minutes the last 2 hours and 121 the
    /// last 3.
    #[test]
    fn the_span_said_holds_the_first_failure() {
        let at = |seconds: i64| since(seconds * 1_000);
        assert_eq!(at(0), "in the last minute");
        assert_eq!(at(60), "in the last minute");
        assert_eq!(at(61), "in the last 2 minutes");
        assert_eq!(at(119 * 60), "in the last 119 minutes");
        assert_eq!(at(120 * 60), "in the last 2 hours");
        assert_eq!(at(121 * 60), "in the last 3 hours");
    }
}
